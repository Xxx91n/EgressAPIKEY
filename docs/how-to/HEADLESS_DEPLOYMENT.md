# Headless Deployment Guide

> Audience: operators deploying `@egressapikey/server` to a Linux VPS,
> Docker container, or a persistent macOS / Windows service.
> Template references: mihomo systemd unit, vaultwarden deployment docs,
> code-server service patterns.

## Security model

The headless server exposes the **full Resin admin control plane** on
`/api/v1/*` and `/metrics/*`. The BFF injects the Resin admin bearer
server-side, so *any client that can reach the port is an admin* - the port
itself is the trust boundary. Three controls protect it:

| Control | Flag | Behaviour |
| --- | --- | --- |
| Shared-secret token | `--auth-token` | Required on every control-plane request. |
| Host / Origin allowlist | `--allowed-host` | Rejects requests whose `Host` / `Origin` is not an accepted name. |
| Startup refusal | - | A non-loopback `--bind` without `--auth-token` refuses to start. |

### Token lifecycle

- **Loopback bind, no `--auth-token`** - a 256-bit token is generated from the
  OS CSPRNG at startup and printed once to stderr and the log. Open the printed
  URL (`http://127.0.0.1:14200/?auth_token=<token>`) once; the server plants an
  `HttpOnly; SameSite=Strict` cookie and the SPA authenticates with it from then
  on. The token never reaches JavaScript.
- **`--auth-token=<secret>`** - the operator-supplied secret is enforced on any
  bind address.
- **Non-loopback bind without a token** - startup is refused. This is
  deliberate: an unauthenticated admin plane must never be reachable off-host.

Time + PID entropy is explicitly **not** used - it is guessable from the process
start window. Comparison against the expected token is constant-time.

### Threat model

| # | Threat | Mitigation | Residual risk |
| --- | --- | --- | --- |
| T1 | **DNS rebinding.** A page on `evil.example` rebinds that name to `127.0.0.1`; the browser then treats calls as same-origin, so CORS no longer protects the admin API. | `Host` / `Origin` allowlist rejects the attacker's `Host` with `403`. Applied to every request, static assets included. | None known. Same mitigation as Caddy / Envoy / nginx. |
| T2 | **Unauthenticated exposure.** `--bind=0.0.0.0` on a LAN or VPS. | Startup refuses without `--auth-token`. | An operator who sets a weak `--auth-token` weakens this to T3. |
| T3 | **Token guessing.** | 256-bit CSPRNG token; constant-time comparison. | No rate limiting, but online guessing of 256 bits is infeasible. |
| T4 | **Cross-site request forgery** from a reachable origin. | `Origin` allowlist + `SameSite=Strict` cookie. | A browser that omits `Origin` on a simple GET still needs the token. |

### Not covered (deliberate)

- **TLS is not terminated by this binary.** Over plain HTTP the token crosses
  the wire in cleartext. Put TLS (Caddy / nginx) in front, or keep the bind on a
  trusted loopback / LAN segment.
- **`--auth-token` is visible in the process arguments** (`ps`, container
  inspect). Prefer the auto-generated token on a loopback bind; when a
  non-loopback bind is unavoidable, restrict who can read process state.
- **Reverse-proxy auth is a deployment layer, not a program boundary.** A
  basicauth / oauth2-proxy gate in front of the port is defence in depth; the
  in-process controls above are the boundary.
- **Resin entry ports (SOCKS5 / HTTP proxy) are a separate surface** with their
  own credential (`RESIN_PROXY_TOKEN`), documented below.

### Local reproduction

The rebinding primitive was reproduced locally before this hardening
(`.scratch/architecture-recovery/repro/dns-rebinding-repro.cjs`): the pre-fix
request shape served the admin plane to `Host: evil.example` with `HTTP 200`,
while the post-fix allowlist returns `403` and still accepts the loopback
`Host`. Evidence is recorded in
`.scratch/architecture-recovery/reports/03-headless-security-report.md`.

## Layout

A self-contained release artifact lives at `release/<os>-backend/`:

```text
release/linux-backend/
├── egressapikey-headless      # Rust binary (~4.4MB, portable)
├── dist/                       # Vite-built React SPA
│   ├── index.html
│   └── assets/
└── resin                       # Go sidecar binary (~38MB)
```

The binary expects `dist/` and `resin` siblings in the same directory
(or override via `--dist` / `--binary-dir`). This is the same layout
`build-all.sh` produces when invoked with no args.

## Linux systemd unit

The unit below binds loopback, so no `--auth-token` is needed: the binary
generates one at startup and prints it once to stderr, which systemd captures
in the journal. Read it with `journalctl -u egressapikey-headless | grep "auth token"`.
To pin your own secret instead, append `--auth-token=<secret>` to `ExecStart`.

Save as `/etc/systemd/system/egressapikey-headless.service`:

```ini
[Unit]
Description=EgressAPIKEY headless control surface
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/opt/egressapikey/egressapikey-headless \
  --bind=127.0.0.1 \
  --port=14200 \
  --no-browser \
  --dist=/opt/egressapikey/dist \
  --binary-dir=/opt/egressapikey
WorkingDirectory=/opt/egressapikey
StateDirectory=egressapikey
LogsDirectory=egressapikey
Restart=on-failure
RestartSec=5s
# Hardening
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
ReadWritePaths=/var/lib/egressapikey /var/log/egressapikey

[Install]
WantedBy=multi-user.target
```

Enable:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now egressapikey-headless
systemctl status egressapikey-headless
```

## Docker / container

A minimal Dockerfile (illustrative — adapt to your base image):

```dockerfile
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
COPY release/linux-backend/ /opt/egressapikey/
WORKDIR /opt/egressapikey
EXPOSE 14200
# Shell form so the token can come from the environment instead of the image.
# --auth-token is MANDATORY here: the bind is not loopback.
ENTRYPOINT ./egressapikey-headless --bind=0.0.0.0 --port=14200 --no-browser --auth-token="$AUTH_TOKEN"
```

Run it with the secret supplied at runtime:

```bash
docker run -e AUTH_TOKEN="$(openssl rand -hex 32)" -p 14200:14200 egressapikey-server
```

Note: bind to `0.0.0.0` inside the container so the port can be mapped out, and
pass `--auth-token` - the binary refuses to start on a non-loopback bind without
it (an empty `$AUTH_TOKEN` fails closed). On a bare-metal host, keep
`--bind=127.0.0.1` and put a TLS reverse proxy in front instead.

## Environment variables

| Variable | Default | Meaning |
| --- | --- | --- |
| `EGRESSAPIKEY_HEADLESS_BIN` | resolved | Override path to the Rust binary |
| `EGRESSAPIKEY_DIST` | sibling of exe | Override path to the built `dist/` dir |
| `RESIN_ADMIN_TOKEN` | generated at startup | Don't set manually; the binary generates one and uses it internally |
| `RESIN_PROXY_TOKEN` | empty (no-auth) | Set a non-empty value to require auth on SOCKS5/HTTP ports |
| `RESIN_LISTEN_ADDRESS` | `127.0.0.1` | Listen address for Resin proxy ports |
| `RESIN_STATE_DIR` | OS data dir | Override only if you need a custom SQLite location |
| `RESIN_LOG_DIR` | OS log dir | Override only if you need a custom log location |

The headless control-surface token is a CLI flag (`--auth-token`), not an
environment variable; the launcher forwards it verbatim. See Security model.

## TLS reverse proxy (recommended)

For remote access, put Caddy or nginx in front of the headless port
and add an auth layer (Cloudflare Access, oauth2-proxy, or a basic-auth
gate). Example Caddyfile:

```caddyfile
egress.example.com {
  reverse_proxy 127.0.0.1:14200
  basicauth / {
    admin <bcrypt-hash>
  }
}
```

**Declare the public hostname.** Caddy and nginx forward the original `Host`
header by default, so the Host allowlist would reject `egress.example.com`
with `403`. Pass it explicitly:

```bash
--bind=127.0.0.1 --auth-token=<secret> --allowed-host=egress.example.com
```

Alternatively, rewrite the header at the proxy so the upstream sees loopback:

```caddyfile
reverse_proxy 127.0.0.1:14200 {
  header_up Host 127.0.0.1:14200
}
```

The proxy's auth gate is defence in depth - the in-process token and Host
allowlist are the actual boundary (see Security model).

### Zero-trust overlay (recommended, not required)

For admin access that never touches the public Internet at all, serve the
control surface over a mesh VPN instead of exposing the port:

- **Tailscale** - run the binary on `--bind=127.0.0.1`, install tailscaled on
  the VPS, then reach `http://<vps-tailnet-ip>:14200` (with `--auth-token` and
  `--allowed-host=<vps-tailnet-name>`). MagicDNS gives you a stable name for
  the allowlist; Tailscale ACLs replace the proxy-auth layer entirely.
- **WireGuard** - same shape: bind loopback (or the WG interface address),
  allow only the tunnel subnet, pass `--allowed-host` for the WG-facing name.

These are *recommendations*, not dependencies - nothing in the binary knows
or cares which overlay is in front. The in-process token + Host allowlist
remain the security boundary either way; the overlay removes the public
attack surface (and the TLS-termination question) entirely.

## Resource budget & sizing

Minimum viable host: **1 vCPU / 1 GiB RAM** (1C1G). The headless binary is
~4.4 MB and the Resin sidecar ~38 MB; a healthy idle stack (shell + sidecar)
should stay well under ~80 MB RSS, and the current budget ceiling for the
full stack under load is **150 MB** (see `docs/how-to/PERF-BENCH.md` — the
representative-hardware column is the only source for tightening these).

For sustained high fan-in (hundreds of concurrent entry-port connections),
raise the descriptor limit — each client connection costs a few FDs
(listener + upstream + timers):

| Control | Target | How |
| --- | --- | --- |
| Open-file limit | **FD ≥ 8192** | systemd: `LimitNOFILE=8192` in the unit below; Docker: `--ulimit nofile=8192:8192` |
| Socket buffers | ~174 KB per connection is a reasonable WAN budget (≈ 100 Mbps × ~14 ms RTT BDP, doubled); Linux autotuning covers it when `net.ipv4.tcp_rmem`/`wmem` max ≥ 4 MiB | `net.ipv4.tcp_rmem="4096 131072 4194304"` (and same for `tcp_wmem`) via sysctl; defaults on bookworm are already sufficient |
| Ephemeral ports | defaults are fine | only raise `net.ipv4.ip_local_port_range` if you out-NAT the box |

On 1C1G the shared GitHub-hosted numbers in `PERF-BENCH.md` are the
informational reference; treat the representative-hardware column (your own
1C1G VPS measurement via `bench-selfhosted.yml`) as the absolute floor.

## Logs

- Logs land in `/var/log/egressapikey/` (systemd) or the OS log dir
  (`~/.local/state/egressapikey/logs/` on Linux without systemd).
- `tauri-plugin-tracing` rotates daily, 10 MB max per file, keeps 7
  files. The panic hook captures panics into the same pipeline.
- See `docs/how-to/HEADLESS_RUNBOOK.md` for log-grep recipes.
