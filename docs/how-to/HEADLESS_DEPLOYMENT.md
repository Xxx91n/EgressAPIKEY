# Headless Deployment Guide

> Audience: operators deploying `@egressapikey/server` to a Linux VPS,
> Docker container, or a persistent macOS / Windows service.
> Template references: mihomo systemd unit, vaultwarden deployment docs,
> code-server service patterns.

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
ENTRYPOINT ["./egressapikey-headless", "--bind=0.0.0.0", "--port=14200", "--no-browser"]
```

Note: bind to `0.0.0.0` inside the container so the port can be mapped
out. NEVER bind `0.0.0.0` on a bare-metal host without TLS + auth.

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

## Logs

- Logs land in `/var/log/egressapikey/` (systemd) or the OS log dir
  (`~/.local/state/egressapikey/logs/` on Linux without systemd).
- `tauri-plugin-tracing` rotates daily, 10 MB max per file, keeps 7
  files. The panic hook captures panics into the same pipeline.
- See `docs/how-to/HEADLESS_RUNBOOK.md` for log-grep recipes.
