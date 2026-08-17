# @egressapikey/server

Headless **EgressAPIKEY** control surface in a browser, without the Tauri
desktop shell. The launcher spawns the Rust `egressapikey-headless` binary,
which starts the Resin sidecar and serves the prebuilt React SPA at
`http://127.0.0.1:14200`. The same GUI controls (Platforms, Subscriptions,
Nodes, Topology) the desktop app exposes are reachable from any browser.

## Install & run (3-line closure)

```bash
npm install -g @egressapikey/server
egressapikey-server
# → open http://127.0.0.1:14200/ in any browser
```

No account, no API token — the headless binary injects the Resin admin
bearer server-side, so the browser request reaches the Resin admin surface
without ever seeing credentials. This mirrors the security model of
code-server / mihomo / vaultwarden local daemons.

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
| `--dry-run` | off | Resolve paths + print resolved config without spawning |

## Environment overrides

- `EGRESSAPIKEY_HEADLESS_BIN` — absolute path to the Rust binary (tests/CI).
- `EGRESSAPIKEY_DIST` — absolute path to the built `dist/` directory.

## Data placement

The headless binary uses the same OS-standard dirs the Tauri app uses:

- **Windows**: `%APPDATA%\\egressapikey\\` (state), `%LOCALAPPDATA%\\egressapikey\\logs\\`
- **macOS**: `~/Library/Application Support/egressapikey/`, `~/Library/Logs/egressapikey/`
- **Linux**: `~/.local/share/egressapikey/`, `~/.local/state/egressapikey/logs/`

## Two-phase shutdown

On `Ctrl+C` (SIGINT) or `SIGTERM`, the launcher kills the Resin child
process and exits cleanly. No orphan processes are left behind.

## Security model

- The admin bearer token is generated at startup and never printed to stdout
  or exposed to the browser. The headless axum reverse proxy strips inbound
  `Authorization` / `X-API-Key` headers and injects its own Bearer before
  forwarding `/api/v1/*` and `/metrics/*` to the local Resin sidecar
  (loopback only).
- The browser request never sees the Resin admin token — only the rendered
  UI and JSON responses.
- The bind address defaults to `127.0.0.1` (loopback). To expose the server
  to other machines on your LAN, pass `--bind=0.0.0.0` and put a TLS
  reverse proxy (caddy / nginx) in front. Never expose the port without
  TLS + an auth layer; the admin surface has no additional auth.

## License

GPL-3.0-or-later (same as the repository root).
