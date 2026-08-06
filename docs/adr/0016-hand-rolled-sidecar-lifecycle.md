# ADR-0016: Hand-rolled sidecar lifecycle (CoreManager pattern, not plugin)

Date: 2026-08-06
Status: ACCEPTED
Context: AGENTS sections 12-13 (G1/G3 sidecar + ghost safety net)

## Decision

Do NOT introduce tauri-sidecar-manager or any external sidecar lifecycle
plugin. Hand-roll the lifecycle layer in src-tauri/src/sidecar.rs following
the clash-verge-rev CoreManager pattern. Add:

- RunningMode enum (Sidecar / NotRunning) in a shared state module
- ArcSwap<State> for lock-free reads of running mode + child handle
- stdout/stderr CommandEvent -> bounded ring buffer (logs + health signal)
- Crash auto-restart policy (bounded retry with backoff)
- Port cleanup before spawn (detect stale process on the free port)
- Two-phase shutdown: SIGTERM -> wait 500ms -> try_wait -> SIGKILL -> wait
- try_wait death short-circuit in the readiness loop

## Rationale

Resin sidecar is not a generic child process. It requires:
1. API handshake (JSON stdout parse for port + tokens)
2. Admin token injection (never crosses into webview)
3. Ghost safety net (3-strike /healthz -> tray red + OS proxy clear)
4. Endpoint API forwarding (port_* IPC -> Resin REST)

clash-verge-rev (100k-node production-validated) also hand-rolls
CoreManager rather than using a plugin. The Resin-specific lifecycle
hooks would require as much adapter code as writing the logic directly.

## Alternatives rejected

- tauri-sidecar-manager plugin: builder config + I/O streaming +
  dual shutdown, but no health poll, no crash restart, no Resin-specific
  handshake. Adapter cost >= hand-roll cost.

## Tradeoffs

- More code to maintain (~200 lines new in sidecar.rs)
- Full control over Resin-specific lifecycle hooks
- No new external crate dependency (Ponytail)
