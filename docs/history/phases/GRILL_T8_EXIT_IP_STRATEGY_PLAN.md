# GRILL T8 EXIT_IP_STRATEGY PLAN

> Branch: codex/rust-port (parallel fix branch)
> Date: 2026-08-13
> Status: PLANNED (all Q1-Q5 decisions confirmed)
> Budget: 100,000,000 tokens
> Skills: ponytail:full, grill-with-docs, domain-modeling

## Decision Summary

| Q | Topic | Decision | Reference |
|---|-------|----------|-----------|
| Q1 | Auto-reconnect fallback | A — Expose Resin passive_circuit_breaker + max_consecutive_failures to GUI whitebox | Resin control_plane_platform.go:34, system config handler |
| Q2 | Strategy effect verification | A+C — strategy_verify IPC + DiagnosticsView panel | New IPC + extend DiagnosticsView |
| Q3 | Domain stickiness | A — Change sticky_ttl default from 168h to 0s + add GUI explanation | ~10 lines, strategy.ts + PlatformsView.tsx |
| Q4 | Subscription update interval | A — Add update_interval param to subscription_add, GUI input (default 30s, 1s-3600s) | commands/mod.rs:536 + SubscriptionsView.tsx |
| Q5 | Graceful strategy switch | C — Both "close all connections" and "reset kernel" via kill+restart sidecar | No Resin close-all API; shell-side boot_resin reuse |

## Architecture Facts (from Resin source code)

1. **passive_circuit_breaker_disabled** — platform-level boolean field
   - PATCH /api/v1/platforms/{id} with {"passive_circuit_breaker_disabled": false}
   - Default: false (circuit breaker ENABLED)
   - When enabled: nodes with consecutive failures get auto-isolated

2. **max_consecutive_failures** — system-level integer (default 7)
   - GET /api/v1/system/config → read current value
   - PATCH /api/v1/system/config with {"max_consecutive_failures": N}
   - Controls how many consecutive failures before a node is isolated

3. **sticky_ttl** — platform-level duration (stored as nanoseconds: sticky_ttl_ns)
   - Default in shell: 168h0m0s (7 days) ← NEEDS CHANGE to 0s
   - PATCH /api/v1/platforms/{id} with {"sticky_ttl": "0s"}
   - 0s = no stickiness (every request can use a different exit IP)
   - clash-verge-rev equivalent: session-sticky with configurable TTL

4. **update_interval** — subscription-level duration string
   - POST /api/v1/subscriptions with {"update_interval": "30s"}
   - Currently hardcoded to "30s" in commands/mod.rs:536
   - PATCH /api/v1/subscriptions does NOT exist (v1.2.0)
   - To change interval: delete + recreate (mesh with Q4-A)

5. **No close-all-connections API** — Resin v1.2.0 has no public endpoint
   - server.go:179 has graceful Shutdown() but no "close all" handler
   - No EvictAll/DeleteLease/PurgeLease in internal handlers
   - Shell-side: kill resin.exe + boot_resin() = equivalent to close-all + reset-kernel

## Execution Plan (ordered by dependency graph)

### Phase T8-1: Expose circuit breaker config (Q1-A)

**Files:**
- `crates/resin-core/src/resin_client.rs` — add system_config_get/system_config_patch methods (GET/PATCH /api/v1/system/config)
- `src-tauri/src/commands/mod.rs` — add system_config_get/system_config_patch IPC commands; extend platform_update to accept passive_circuit_breaker_disabled
- `src/lib/ipc.ts` — add ipcSystemConfigGet/ipcSystemConfigPatch wrappers; extend ipcPlatformUpdate with circuitBreakerDisabled param
- `src/views/SettingsView.tsx` — add "Circuit Breaker" section: max_consecutive_failures input + enable/disable toggle
- `src/views/PlatformsView.tsx` — per-platform circuit breaker toggle in platform card

**Tests:**
- cargo: mockito system_config_get round-trip, system_config_patch round-trip, platform_update with passive_circuit_breaker_disabled field
- vitest: ipcSystemConfigGet wrapper forward, ipcSystemConfigPatch wrapper validation, platform card displays circuit breaker state

**Acceptance:**
- GUI Settings shows "Circuit Breaker" section with max_consecutive_failures (default 7)
- Each platform card has a toggle for circuit breaker enable/disable
- PATCH /api/v1/platforms/{id} with passive_circuit_breaker_disabled works
- PATCH /api/v1/system/config with max_consecutive_failures works
- cargo test green, vitest green

### Phase T8-2: Strategy effect verification IPC (Q2-A)

**Files:**
- `src-tauri/src/commands/mod.rs` — add strategy_verify IPC: takes platform_name + sample_count (N), sends N probe requests through the port, collects exit IP + latency for each, returns distribution stats
- Reuses existing port_health_check TCP probe + Resin /metrics/realtime/leases for IP attribution
- `src/lib/ipc.ts` — add ipcStrategyVerify wrapper

**Tests:**
- cargo: strategy_verify with mock Resin (returns distribution stats), N=3 sample, verifies struct shape
- vitest: ipcStrategyVerify wrapper forward + validation (N range 3-50)

**Acceptance:**
- strategy_verify("Default", 10) returns { samples: [{ip, latency_ms}], distribution: {ip1: 4, ip2: 6}, avg_latency: 120, strategy: "random" }
- cargo test green, vitest green

### Phase T8-3: Strategy verification DiagnosticsView panel (Q2-C)

**Files:**
- `src/views/DiagnosticsView.tsx` — add "Strategy Verification" card: platform selector dropdown, N input (default 10), run button, result display (IP distribution pie/bar + latency histogram + strategy name)
- Uses chart.js or simple CSS bars (Ponytail: prefer SVG/CSS bars, no new dep)

**Tests:**
- vitest: DiagnosticsView renders strategy panel, run button calls ipcStrategyVerify, displays distribution

**Acceptance:**
- DiagnosticsView has "Strategy Verification" section
- User selects platform, clicks Run, sees IP distribution + avg latency
- vitest green

### Phase T8-4: Fix sticky_ttl default to 0s (Q3-A)

**Files:**
- `src/lib/strategy.ts` — no change needed (strategy mapping untouched)
- `src/views/PlatformsView.tsx` — change default stickyTtl from "168h0m0s" to "0s", add tooltip/label: "0 = no stickiness (every request may use different exit IP)"
- `src/lib/ipc.ts` — ipcPlatformCreateWithFields: default stickyTtl to "0s" if not specified
- `src-tauri/src/commands/mod.rs` — platform_create_with_fields: default sticky_ttl to "0s" in the JSON body sent to Resin
- i18n: add strategy.stickyTtlHint key to all 18 locales

**Tests:**
- vitest: PlatformsView shows sticky_ttl=0s by default, tooltip text present
- cargo: platform_create_with_fields sends sticky_ttl="0s" when not specified

**Acceptance:**
- New platforms have sticky_ttl = 0s by default
- GUI shows explanation text next to sticky_ttl input
- Existing platforms with 168h keep their value (only default changes)
- i18n:check green (287+1 keys × 18 locales)

### Phase T8-5: Subscription update_interval whitebox (Q4-A)

**Files:**
- `src-tauri/src/commands/mod.rs` — subscription_add: add update_interval: String param (default "30s"), validate format (Go duration string, 1s-3600s range), pass into POST body
- `src/views/SubscriptionsView.tsx` — add update_interval input field (default "30s", placeholder "30s") next to URL input
- `src/lib/ipc.ts` — extend ipcSubscriptionAdd with updateInterval param
- i18n: add subscription.updateInterval key to all 18 locales

**Tests:**
- cargo: subscription_add with custom update_interval "60s" posts correct body
- vitest: SubscriptionsView form has update_interval field, submits correct value

**Acceptance:**
- GUI subscription form has "Update Interval" input (default 30s)
- Creating a subscription with "60s" results in Resin scheduler fetching every 60s
- i18n:check green

### Phase T8-6: Close all connections + Reset kernel (Q5-C)

**Files:**
- `src-tauri/src/commands/mod.rs` — add two IPC commands:
  - close_all_connections: kill resin.exe child + boot_resin() (same process restart, ~2s downtime)
  - reset_kernel: same implementation (kill + boot_resin), but different semantic label + log message
  - Both reuse existing sidecar.rs boot_resin infrastructure
  - Add tracing::info!("user-initiated close-all-connections") / "user-initiated kernel-reset"
- `src/lib/ipc.ts` — add ipcCloseAllConnections/ipcResetKernel wrappers
- `src/views/DiagnosticsView.tsx` — add two buttons in a "Connection Control" card: "Close All Connections" (red) + "Reset Kernel" (orange), each with confirm dialog
- i18n: add diagnostics.closeAllConnections + diagnostics.resetKernel keys to all 18 locales

**Tests:**
- cargo: close_all_connections command validates (no crash on missing sidecar, returns IpcError)
- vitest: DiagnosticsView renders both buttons, click triggers correct IPC call

**Acceptance:**
- DiagnosticsView has "Connection Control" card with two buttons
- Clicking "Close All Connections" kills and restarts resin.exe
- Clicking "Reset Kernel" kills and restarts resin.exe (same impl, different label)
- Existing SSE connections are dropped (expected — this is the point)
- tray icon shows restart status
- i18n:check green

### Phase T8-7: Full gate + build + push

1. cargo test -p resin-core --lib (expect all green)
2. cargo build --release -p egressapikey-app --features custom-protocol
3. pnpm build (tsc + vite, new chunk hash)
4. grep new Vite chunk hash in exe bytes
5. pnpm test (vitest all green)
6. pnpm i18n:check
7. codegraph sync .
8. git add + commit + push

## Dependency Graph

```
T8-1 (circuit breaker) ──┐
                         ├──> T8-7 (gate + build + push)
T8-2 (strategy verify) ──┤
                         │
T8-3 (diag panel) ────────┤
                         │
T8-4 (sticky_ttl) ────────┤
                         │
T8-5 (sub interval) ─────┤
                         │
T8-6 (close-all/reset) ──┘
```

T8-1 through T8-6 are independent of each other (different files/features), but commits are sequential (Ponytail: small step commits). T8-7 depends on all.

## Resin API surface used (all verified from source)

| Endpoint | Method | Used in | Field |
|----------|--------|---------|-------|
| /api/v1/platforms/{id} | PATCH | T8-1, T8-4 | passive_circuit_breaker_disabled, sticky_ttl |
| /api/v1/system/config | GET | T8-1 | max_consecutive_failures |
| /api/v1/system/config | PATCH | T8-1 | max_consecutive_failures |
| /api/v1/subscriptions | POST | T8-5 | update_interval |
| (no close-all API) | — | T8-6 | shell-side kill+restart |
