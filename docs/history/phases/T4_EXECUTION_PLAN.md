# T4 Execution Plan — Domain-Modeling Audit Complete

> Generated: 2026-08-09
> Method: domain-modeling skill audit of ADRs 0021-0025 against actual codebase

## Audit Findings

### ADR-0021 (SOCKS5 Auth + Port Health)
- **Title fix needed**: file says "ADR-0015", should be "ADR-0021"
- **Code status**: `port_auth_info` IPC does NOT exist yet. `port_health_check` IPC does NOT exist yet.
- **Resin source confirmed**: `resin/internal/proxy/socks5.go` L261 — global token forces UserPass auth
- **SidecarHandle.proxy_token** exists in `sidecar.rs` but never exposed to IPC layer
- **Feasibility**: CONFIRMED — add 2 new IPC commands in commands/mod.rs, expose in ipc.ts, display in PlatformsView

### ADR-0022 (Strategy Engine)
- **Code status**: `strategy_engine.rs` does NOT exist yet. Current `strategy.rs` is only a static catalog (6 names mapping to 3 Resin policies).
- **Resin source confirmed**: `allocation_policy` has 3 values (BALANCED, PREFER_LOW_LATENCY, PREFER_IDLE_IP) in `internal/platform/policy.go`
- **Resin probing confirmed**: `internal/probe/manager.go` has ProbeManager with configurable intervals — liveness gate already works
- **Feasibility**: CONFIRMED — new module in resin-core, no Resin fork needed

### ADR-0023 (Node Pool Tree View)
- **Code status**: `NodesView.tsx` is a flat table. `NodeItem` interface lacks `reference_latency_ms` and `egress_ip` fields.
- **Resin source confirmed**: `NodeSummary` struct (control_plane_platform.go L591-608) has `reference_latency_ms`, `egress_ip`, `tags[].subscription_name` — all data is available from the API.
- **Feasibility**: CONFIRMED — pure frontend refactor, no backend changes

### ADR-0024 (Delete Legacy LaneCount)
- **Code status**: `laneCount` in appStore.ts (default 10, range 1-50). `setLaneCount` exists. SettingsView renders it. SubscriptionsView uses it for i18n display string ("已将 X 个节点导入 Y 条通道").
- **Rust side**: `build_shared_gateway(lanes)` in lib.rs creates `SharedGateway`. Used in main.rs `.manage(gateway)`. But `gateway_*` IPC commands do NOT use the SharedGateway state — `gateway_reserve`/`gateway_release`/`gateway_evict_lane`/`gateway_record_latency`/`gateway_select_account` are all stubs returning Ok(()). Only `gateway_snapshot` uses `SidecarHandle` (not SharedGateway).
- **SharedGateway callers**: main.rs `.manage(gateway)`, lib.rs tests, commands/mod.rs does NOT reference it in any command function signature.
- **Frontend callers of gateway_* IPC**: ipc.ts defines `ipcGatewaySelectAccount`, `ipcGatewaySnapshot`, `ipcReserve`, `ipcRecordLatency`, `ipcReleaseLease`, `ipcEvictLane`, `ipcAccountAdd`. But NO view (PlatformsView, SubscriptionsView, etc.) calls any of these except via appStore which doesn't call them either.
- **Feasibility**: CONFIRMED — but scope is larger than ADR states. Need to also clean up ipc.ts gateway functions, Rust gateway_* commands, main.rs registration, and i18n strings that reference "通道".

### ADR-0025 (Topology C-Column Dynamic Pool)
- **Code status**: TopologyView.tsx currently groups C-column by region (parseNodeGroups function). buildEdges() uses region_filters matching.
- **Dependency**: requires ADR-0022 (strategy_engine) for strategy data, and ADR-0023 (node pool tree) for subscription grouping pattern.
- **Feasibility**: CONFIRMED — but blocked by T4-4 completion

## Corrected Execution Plan

### Phase T4-1: Dead Code Cleanup (ADR-0024) — EXPANDED SCOPE
Files to modify:
1. `src/store/appStore.ts` — remove laneCount, setLaneCount, update addSubscription signature
2. `src/views/SettingsView.tsx` — remove lane count control + baseline tracking
3. `src/views/SubscriptionsView.tsx` — remove lanes variable, update localAdd call, update i18n display
4. `src/App.tsx` — remove setLaneCount call from settings load
5. `src/lib/settings.ts` — remove laneCount save/load
6. `src/lib/ipc.ts` — remove gateway_* functions, LaneSnapshot, SelectResult, ReserveResult types
7. `src/lib/ipc.test.ts` — remove gateway_* tests
8. `src/store/appStore.test.ts` — remove laneCount tests
9. `src/views/PlatformsView.test.tsx` — remove laneCount from mock state
10. `src/views/ProcessRouteView.test.tsx` — remove laneCount from mock state
11. `src/views/SubscriptionsView.test.tsx` — remove laneCount from mock state
12. `src-tauri/src/main.rs` — remove AI_API_ROUTE_LANES, build_shared_gateway, .manage(gateway), gateway_* command registrations
13. `src-tauri/src/lib.rs` — remove SharedGateway type alias, build_shared_gateway, new_gateway_state
14. `src-tauri/src/commands/mod.rs` — remove gateway_reserve, gateway_release, gateway_evict_lane, gateway_record_latency, gateway_select_account, gateway_snapshot, ReserveResult, SelectResult, LaneSnapshot, MAX_LANES
15. `crates/resin-core/src/gateway.rs` — remove or mark deprecated (integration tests reference it)
16. `crates/resin-core/src/lib.rs` — remove gateway module export, DEFAULT_LANES, MAX_LANES, MIN_LANES if unused elsewhere
17. i18n files — update "出口通道数" / "lanes" strings across all 18 locales
Test: cargo check -p egressapikey-app + cargo check -p resin-core + npx vitest run

### Phase T4-2: SOCKS5 Auth + Port Health (ADR-0021)
Files to create/modify:
1. `src-tauri/src/commands/mod.rs` — add `port_auth_info` and `port_health_check` IPC commands
2. `src-tauri/src/main.rs` — register new commands
3. `src/lib/ipc.ts` — add `ipcPortAuthInfo` and `ipcPortHealthCheck` functions + types
4. `src/views/PlatformsView.tsx` — show auth credentials + health indicator on port cards
5. `src/lib/ipc.test.ts` — tests for new IPC functions
6. `src/views/PlatformsView.test.tsx` — test auth display
Test: cargo test -p egressapikey-app + npx vitest run

### Phase T4-3: Node Pool Tree View (ADR-0023)
Files to modify:
1. `src/views/NodesView.tsx` — refactor flat table to collapsible tree
2. `src/views/NodesView.test.tsx` — update tests for tree structure
3. i18n — add new keys for tree labels if needed
Test: npx vitest run

### Phase T4-4: Strategy Engine (ADR-0022) — BLOCKS T4-5
Files to create:
1. `crates/resin-core/src/strategy_engine.rs` — new module
2. `crates/resin-core/src/lib.rs` — export strategy_engine
3. `crates/resin-core/src/strategy_config.rs` — whitebox config schema + hotswap-config integration
4. `src-tauri/src/commands/mod.rs` — strategy IPC commands (get/set/list)
5. `src-tauri/src/main.rs` — register strategy commands
6. `src/lib/ipc.ts` — strategy IPC wrappers
7. `src/views/PlatformsView.tsx` — strategy selection panel per platform
Files to modify:
8. `crates/resin-core/src/whitebox_config.rs` — add strategy config file path
Test: cargo test -p resin-core + cargo test -p egressapikey-app + npx vitest run

### Phase T4-5: Topology Canvas Refactor (ADR-0025) — DEPENDS ON T4-3 + T4-4
Files to modify:
1. `src/views/TopologyView.tsx` — rewrite parseNodeGroups to group by subscription, rewrite buildEdges for strategy-driven edges, add edge labels
2. `src/views/TopologyView.test.tsx` — update tests for new edge builder + subscription grouping
Test: npx vitest run

## Dependency Graph (corrected)

```
T4-1 (dead code)     ──── no deps ────┐
T4-2 (SOCKS5 auth)   ──── no deps ────┤
T4-3 (node tree)     ──── no deps ────┤
                                      ├──> T4-4 (strategy) ──> T4-5 (topology)
                                      │    depends on T4-2   depends on T4-3 + T4-4
```

T4-1, T4-2, T4-3 can execute in parallel.
T4-4 depends on T4-2 (port health for liveness display).
T4-5 depends on T4-3 (subscription grouping) + T4-4 (strategy data).