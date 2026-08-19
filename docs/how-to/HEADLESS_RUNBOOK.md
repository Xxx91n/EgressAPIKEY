# Headless Runbook

> Audience: operators running `@egressapikey/server` in production.
> Quick-reference for log location, health check, graceful shutdown,
> restart policy, and the most-common troubleshooting steps.
> Template references: code-server troubleshooting, mihomo operator guide.

## Log location

| OS | Path |
| --- | --- |
| Linux (systemd) | `/var/log/egressapikey/` or `journalctl -u egressapikey-headless -f` |
| Linux (no systemd) | `~/.local/state/egressapikey/logs/` |
| macOS | `~/Library/Logs/egressapikey/` |
| Windows | `%LOCALAPPDATA%\\egressapikey\\logs\\` |

Each log file is JSON-line structured tracing output. Daily rotation,
10 MB max per file, keeps 7 files. The panic hook also routes panics
into the same pipeline so a crash leaves a stack trace in the log.

## Health check

The headless binary does NOT expose `/healthz` on its own router; that
endpoint lives on the local Resin sidecar admin port (random loopback
port, generated at startup). For service health, use systemd's built-in
`Restart=on-failure` (or container health checks) — the binary failing
to start a sidecar exits non-zero.

From inside the host, you can probe the SPA root:

```bash
curl -sf http://127.0.0.1:14200/ -o /dev/null && echo SPA-OK || echo SPA-FAIL
```

And an admin endpoint:

```bash
curl -sf http://127.0.0.1:14200/api/v1/platforms | head -c 200
```

A 200 with JSON means both the binary and the Resin sidecar are alive.

## Graceful shutdown

Send `SIGTERM` (systemd stop, docker stop, or `Ctrl+C`). The binary
runs a two-phase shutdown:

1. Kill the Resin child process and wait for it to be reaped.
2. Abort the axum server task and exit 0.

Up to ~3 seconds for the sidecar to die. If you `SIGKILL` the binary,
the Resin child may be orphaned — systemd / cgroup v2 should kill it as
part of the unit's cgroup cleanup, but a bare metal SIGKILL can leak it.
See `docs/RESIN_UPSTREAM_MANIFEST.yaml` for the sidecar version.

## Restart policy

systemd unit (`Restart=on-failure`, `RestartSec=5s`) covers transient
crashes. For Docker, use `restart: unless-stopped` (compose) or
`--restart=unless-stopped` (bare docker). The binary itself is
stateless; all persistent state is in the Resin sidecar's SQLite state.db
under `StateDirectory`.

## Troubleshooting

### SPA loads but /api/v1/* returns 500 or 502

- Check the headless binary's stdout / log file for "control plane not
  reachable" — this means the Resin sidecar failed to boot within the
  15s deadline. Common causes: missing `resin` binary sibling, or the
  OS data dir cannot be created (permissions). Fix the path / dir + restart.
- If logs show "sidecar /healthz fail", the Resin sidecar is crashing.
  Run `./resin --help` directly to surface its startup error.

### Port 14200 already in use

Pass `--port=<other-port>` or stop the occupying process:

```bash
lsof -iTCP:14200 -sTCP:LISTEN
```

### Sidecar orphaned after `kill -9`

Run `pgrep -af resin` to find orphaned children. Kill them manually
or restart the systemd unit which the cgroup layer will reap.

### Logs not appearing

- Verify the log dir is writable: `ls -ld ~/.local/state/egressapikey/logs`
- systemd unit: check `LogsDirectory=egressapikey` is set and
  `/var/log/egressapikey` exists.

### Build issue: "headless binary not found"

The release artifact is built by `cargo build --release --bin egressapikey-headless --features headless`.
If your `build-all.sh` logs show "FATAL: headless binary not found",
the build step failed — re-run with full logs:

```bash
cargo build --release -p egressapikey-app --bin egressapikey-headless --features headless 2>&1 | tail -50
```

### Upgrading

1. Re-fetch the upstream Resin sidecar: `bash scripts/fetch_resin.sh`
2. Rebuild: `bash scripts/build-all.sh`
3. Copy the new `release/<os>-backend/` into your deploy dir.
4. `sudo systemctl restart egressapikey-headless`
