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
- The control plane requires a shared `--auth-token` on every `/api/v1/*` and
  `/metrics/*` request, and rejects any request whose `Host` / `Origin` is not an
  accepted name (DNS-rebinding mitigation). A non-loopback `--bind` without
  `--auth-token` refuses to start.
- The bind address defaults to `127.0.0.1` (loopback), where the token is
  generated with the OS CSPRNG and printed once. To expose the server to other
  machines, pass `--auth-token=<secret> --bind=0.0.0.0`, declare your public
  name with `--allowed-host`, and put a TLS reverse proxy (caddy / nginx) in
  front. The proxy's auth gate is defence in depth, not the boundary.

## License

GPL-3.0-or-later (same as the repository root).
