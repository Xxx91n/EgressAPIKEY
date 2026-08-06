# @egressapikey/server

Headless launcher for **EgressAPIKEY** — runs the Resin-backed control surface
in any browser **without** installing the Tauri desktop shell.

Use this when you want the same GUI controls (Platforms, Subscriptions, Nodes,
Topology) reachable from `http://localhost:14200` on a Linux server, Docker
container, or a developer machine without the desktop app.

## Install

```bash
npm install -g @egressapikey/server
egressapikey-server
```

A browser window opens on `http://127.0.0.1:14200/`.

## CLI flags

All flags forward directly to the Rust `egressapikey-headless` binary:

| Flag | Default | Description |
| --- | --- | --- |
| `--bind` | `127.0.0.1` | HTTP bind address |
| `--port` | `14200` | HTTP bind port |
| `--dist` | `dist/` (resolved) | Built Vite assets directory |
| `--state-root` | OS data dir | Resin sidecar `state.db` parent dir |
| `--log-root` | OS state/logs dir | Resin + headless log dir |
| `--binary-dir` | exe dir | Directory containing `resin-<triple>[.exe]` |
| `--no-browser` | off | Do not open a browser on boot |

## Environment overrides

- `EGRESSAPIKEY_HEADLESS_BIN` — absolute path to the Rust binary (tests/CI).
- `EGRESSAPIKEY_DIST` — absolute path to the built `dist/` directory.

## Data placement

The headless binary uses the same OS-standard dirs the Tauri app uses:

- **Windows**: `%APPDATA%\egressapikey\` (state), `%LOCALAPPDATA%\egressapikey\logs\`
- **macOS**: `~/Library/Application Support/egressapikey/`, `~/Library/Logs/egressapikey/`
- **Linux**: `~/.local/share/egressapikey/`, `~/.local/state/egressapikey/logs/`

## Two-phase shutdown

On `Ctrl+C` (SIGINT) or `SIGTERM`, the launcher kills the resin child and exits
cleanly. No orphan processes are left behind.

## License

GPL-3.0-or-later (same as the repository root).
