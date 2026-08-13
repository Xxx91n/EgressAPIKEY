# T8 Exit IP Strategy Fix Branch - Handoff

> Branch: `codex/rust-port` | HEAD: `5988c7d` (pushed) | Date: 2026-08-14

## Completed Phases

| Phase | Status | Summary |
|-------|--------|---------|
| T8-1 | Done | Circuit breaker: passive_circuit_breaker_disabled in platform PATCH + system_config_get/patch IPC + circuitBreakerThreshold system config |
| T8-2 | Done | Strategy verify: strategy_verify IPC probes N requests (3-50) via Resin proxy, collects exit IP + latency distribution, DiagnosticsView panel with platform dropdown, sample count, result visualization |
| T8-3 | Done | Connection control: close_all_connections + reset_kernel IPC (kill sidecar child via Mutex Option Child + sidecar_restart helper), DiagnosticsView panel with confirm dialogs |
| T8-4 | Done | Sticky TTL: default changed from 168h0m0s to 0s (disable domain stickiness so random/round-robin strategies work correctly) |
| T8-5 | Done | Subscription update_interval: subscription_add accepts update_interval Option String param (default 30s, validated against Resin) |
| T8-6 | Done | Close/Reset IPC: both commands kill sidecar, reset additionally restarts; SidecarHandle.child is Mutex Option Child for safe kill |
| T8-7 | Done | Final gate: tsc green, vitest 182 passed, cargo 118 passed, i18n 304 keys, diff --check clean |

## Key Decisions (Q1-Q5)

| Q | Decision | Rationale |
|---|----------|-----------|
| Q1 | A - Expose Resin passive_circuit_breaker_disabled + max_consecutive_failures | Shell exposes existing Resin mechanism, no fork needed |
| Q2 | A+C - strategy_verify IPC + DiagnosticsView panel | Verify strategies work + surface in diagnostics center |
| Q3 | A - sticky_ttl default to 0s | User wants no domain stickiness; sticky breaks random exit IP |
| Q4 | A - subscription_add accepts update_interval | Whitebox configurable sync interval |
| Q5 | C - Both close_all_connections + reset_kernel via kill+restart | Resin has no graceful connection close API; kill sidecar drops all TCP |

## Files Changed (22 files, +526 lines, -1 line)

### Rust backend
- crates/resin-core/src/resin_client.rs - system_config_get/patch methods added
- src-tauri/src/commands/mod.rs - system_config_get/patch, strategy_verify, close_all_connections, reset_kernel IPC + passive_circuit_breaker_disabled in platform_update + sticky_ttl default 0s + update_interval in subscription_add
- src-tauri/src/main.rs - invoke_handler registrations for all new commands

### TS frontend
- src/lib/ipc.ts - ipcSystemConfigGet/Patch, ipcStrategyVerify, ipcCloseAllConnections, ipcResetKernel wrappers + circuitBreakerDisabled param in ipcPlatformUpdate + updateInterval in ipcSubscriptionAdd
- src/views/DiagnosticsView.tsx - Strategy verify panel (platform dropdown, sample count, result bar chart) + Connection control panel (close all + reset kernel buttons with confirm)

### i18n
- All 18 locales src/locales/*/common.json - 10 new keys (304 total): strategy.circuitBreaker, strategy.circuitBreakerThreshold, strategy.stickyTtlHint, subscription.updateInterval, diagnostics.closeAllConnections, diagnostics.resetKernel, strategyVerify.*, connectionControl.*

## Test Results

- vitest: 182 passed / 14 files / 0 failures
- cargo: 118 passed / 0 failed / 0 ignored
- tsc: zero errors
- i18n: 304 keys x 18 locales, all match en canonical
- git diff --check: CLEAN (no CRLF)

## Build

- npx vite build green - chunks CzuPrGlZ (index) + CxDMLfHj (TopologyView)
- cargo build --release -p egressapikey-app --features custom-protocol - 12.94MB exe
- Chunk hash verified inside exe bytes (AGENTS section 5 hard close-loop)
- Staged at release/windows-gui/EgressAPIKEY.exe + resin.exe sidecar (36.17MB)

## Architecture Notes

- Circuit breaker: Resin v1.2.0 exposes passive_circuit_breaker_disabled (boolean, per-platform via PATCH) and max_consecutive_failures (integer, system-wide via GET/PATCH). Shell exposes both through IPC; no fork needed.
- Strategy verify: shell calls http://127.0.0.1:api_port/proxy_token/https/api.ipify.org with X-Resin-Account header, N times (3-50), collects unique exit IPs + latencies.
- Sticky TTL: Resin default 168h0m0s (7 days) causes domain stickiness. Changed to 0s at the IPC layer so Resin receives 0s by default; user can still override via platform_update if needed (whitebox).
- Close/reset: Resin has no graceful drop all connections API. close_all_connections kills the sidecar child process (drops all active TCP). reset_kernel kills + restarts via sidecar_restart().
- SidecarHandle.child changed to Mutex Option Child so the kill path can .take() the child once and call .kill() safely; a second kill finds None and no-ops.