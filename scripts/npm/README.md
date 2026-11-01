# @egressapikey/server

Headless **EgressAPIKEY** control surface in a browser, without the Tauri
desktop shell. The launcher (`launcher.cjs`) spawns the Rust
`egressapikey-headless` binary, which starts the Resin sidecar and serves the
prebuilt React SPA at `http://127.0.0.1:14200`. The same GUI controls
(Platforms, Subscriptions, Nodes, Topology) the desktop app exposes are
reachable from any browser.

**Not published to npm** — the registry install path is currently not
supported; run the launcher from a source checkout:

```bash
pnpm install && pnpm build
cargo build --release -p egressapikey-app --bin egressapikey-headless --features headless
bash scripts/fetch_resin.sh   # pins the sidecar into src-tauri/binaries/
node scripts/npm/launcher.cjs --dist dist --binary-dir src-tauri/binaries --no-browser
# → open http://127.0.0.1:14200/ in any browser
```

The launcher resolves the Rust binary in order: `EGRESSAPIKEY_HEADLESS_BIN`,
`binaries/egressapikey-headless<ext>` next to itself, then
`../../target/{release,debug}/egressapikey-headless<ext>` (including
`target/<host-triple>/release/`). Node.js 18+ required (`engines`).

The browser never holds the Resin admin bearer — the headless binary injects
it server-side. The control surface itself is gated by a shared `--auth-token`
plus a `Host` / `Origin` allowlist. On the default loopback bind the token is
generated at startup and printed once; the first request carries it via
`?auth_token=` and plants a session cookie. See Security model below.

## CLI flags

All flags forward directly to the Rust `egressapikey-headless` binary.

| Flag | Default | Description |
| --- | --- | --- |
| `--bind` | `127.0.0.1` | HTTP bind address |
| `--port` | `14200` | HTTP bind port |
| `--dist` | resolved | Built Vite assets dir (dist/) |
| `--state-root` | OS data dir | Resin sidecar state.db parent dir |
| `--log-root` | OS state/logs dir | Resin + headless log dir |
| `--binary-dir` | exe dir | Directory containing resin-<triple>[.exe] |
| `--no-browser` | off | Do not open a browser on boot |
| `--auth-token` | generated (loopback) | Shared secret required on `/api/v1/*` + `/metrics/*`; mandatory for a non-loopback `--bind` |
| `--allowed-host` | loopback names | Extra `Host` / `Origin` name to accept (repeatable); needed behind a reverse proxy |
| `--trusted-proxy` | none | Peer trusted to assert `X-Forwarded-Proto` (repeatable; exact IP or CIDR). Without it XFP is ignored entirely |
| `--trusted-proxy-unrestricted` | off | Acknowledges a wildcard `--trusted-proxy` CIDR (`0.0.0.0/0` / `::/0`); required or startup is refused |
| `--dry-run` | off | Resolve paths + print resolved config without spawning |

## Environment overrides

- `EGRESSAPIKEY_HEADLESS_BIN` — absolute path to the Rust binary (tests/CI).
- `EGRESSAPIKEY_DIST` — absolute path to the built `dist/` directory.

## Data placement

The headless binary uses the same OS-standard dirs the Tauri app uses:

- **Windows**: `%APPDATA%\\egressapikey\\` (state), `%LOCALAPPDATA%\\egressapikey\\logs\\`
- **macOS**: `~/Library/Application Support/egressapikey/`, `~/Library/Logs/egressapikey/`
- **Linux**: `~/.local/share/egressapikey/`, `~/.local/state/egressapikey/logs/`

## Shutdown

On `Ctrl+C` (SIGINT) or `SIGTERM` the child process exits and the launcher
propagates the signal, so no orphan processes are left behind.

## Security model

- The Resin admin bearer token is generated at startup and never printed to
  stdout or exposed to the browser. The headless axum reverse proxy strips
  inbound `Authorization` / `X-API-Key` headers and injects its own Bearer
  before forwarding `/api/v1/*` and `/metrics/*` to the local Resin sidecar
  (loopback only). The browser request never sees the Resin admin token —
  only the rendered UI and JSON responses.
- The control plane requires a shared `--auth-token` on every `/api/v1/*` and
  `/metrics/*` request, and rejects any request whose `Host` / `Origin` is not an
  accepted name (DNS-rebinding mitigation). A non-loopback `--bind` without
  `--auth-token` refuses to start.
- The bind address defaults to `127.0.0.1` (loopback), where the `--auth-token`
  is generated with the OS CSPRNG and printed once on stderr. To expose the
  server to other machines, pass `--auth-token=<secret> --bind=0.0.0.0`,
  declare your public name with `--allowed-host`, and put a TLS reverse proxy
  (caddy / nginx) in front. The proxy's auth gate is defence in depth, not the
  boundary.

## License

GPL-3.0-or-later (same as the repository root).
