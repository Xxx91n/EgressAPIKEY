# GRILL T18 — Canvas + Pipeline Sync Plan

> Canon: ADR-0042 (decisions) + CONTEXT.md deltas (terms) + this file (phases).
> Source grill rounds: R1 (Q1-Q7) + R2 (Q8-Q16) with atomcode research on Q8 / Q16.

## Decisions locked

| Q  | Topic | Answer |
|----|-------|--------|
| Q1 | Port state sync source | A: health chip on EntryPortNode, data from port_health_check + PortMapping.enabled/auth_required |
| Q2 | Port→Platform binding source | A: whitebox egressapikey-ports.json is truth; Resin rebuilt from whitebox on restart |
| Q3 | A-strategy selected-nodes sync | A-bis: C-column stays global pool; edges filter per-platform (not per-platform C column) |
| Q4 | B-strategy parameter display | B: B badge shows strategy params (round-robin N, latency threshold) via i18n templates |
| Q5 | region viewMode existence | A: keep dual viewMode; region still useful for "how many nodes per region" mental model |
| Q6 | Config-file entry placement | A: FileCog moved to right-top toolbar with dropdown (ports.json / strategy.json); left-bottom FileCog removed |
| Q7 | Home button (reset center) | A: already exists (L636-638 setViewport({x:0,y:0,zoom:1})) |
| Q8 | Health chip data source | atomcode: Rust watch_port_health Channel + single ticker + batch probe + concurrency cap 10 + atomic reentry guard + TTL skip + exponential backoff (5 fails → Dead) + idle pause when tab hidden. Front-end single subscription + Map update + React.memo |
| Q9 | auth_required display on EntryPortNode | A: lock icon 🔒/🔓 + i18n tooltip, no extra text line |
| Q10 | Port enable/disable switch | A: PortForwarder controls listen (enabled=false → not listening), whitebox enabled field is truth, Resin unaware |
| Q11 | B-strategy param i18n keys | A: per-strategy i18n template with {{param}} interpolation; strategyConfig gains b_class_params field |
| Q12 | strategyConfig vs Resin sync | A: strategyConfig is single source of truth (ADR-0036); sync merge overwrites Resin values; Resin direct edits overwritten by next strategy_apply |
| Q13 | port_health_check protocol | A: already implemented (L476-481), pass port.protocol to probe |
| Q14 | MiniMap i18n | A: already i18n (L1009 ariaLabel), no change |
| Q15 | Resin restart rebuild from whitebox | A: new restore_ports_from_whitebox logic after boot_resin success |
| Q16 | Manual mode canvas interaction | atomcode: canvas drag stays group-level only (no leaf-node handles); manual selection via expanded card checkbox → manual_nodes → strategy_engine → region_filters PATCH |

## Phase execution plan

### Phase 1 — Port health chip + batch probe foundation

**Files**: src-tauri/src/commands/mod.rs (new watch_port_health Channel command), src-tauri/src/forwarder.rs (batch probe logic), src/views/TopologyView.tsx (EntryPortNode chip + Channel subscription), src/lib/ipc.ts (TS wrapper for watch_port_health)

**Work**:
1. Rust: new `watch_port_health(on_event: tauri::ipc::Channel<PortHealthSnapshot>)` command. Single Tokio task: ticker (interval = max(5s, k·ln(1+N)) where N = port count), batch probe with concurrency cap 10, atomic reentry guard (AtomicBool), TTL skip (skip if last probe < interval), exponential backoff for dead ports (interval × 2^min(fails,5), cap 5m), pause when tab hidden (shell emits pause/resume events).
2. Rust: PortHealthSnapshot { revision: u64, ports: Vec<{port: u16, alive: bool, fails: u32, latency_ms: Option<u32>}> } serialized as one Channel message per tick.
3. TS: new `ipcWatchPortHealth(callback)` wrapper using `@tauri-apps/api/event` Channel. TopologyCanvas subscribes on mount, unsubscribes on unmount. health state stored in a Map<u16, HealthState> in zustand topology store. EntryPortNode reads health from store.
4. EntryPortNode: 4-state visual encoding (alive=green dot, degraded=amber, dead=red + card grayed, restarting=blue pulse). Lock icon for auth_required. i18n: topology.portAlive / portDead / portDegraded / portRestarting / portAuthRequired / portAuthNotRequired.
5. Remove per-port ipcPortHealthCheck from sync() poll loop (if any).

**Tests**: cargo test (batch probe logic + backoff math), vitest (health chip renders 4 states + lock icon).

**Est**: ~250 lines Rust + ~80 lines TS.

### Phase 2 — Port enable/disable switch

**Files**: src/views/PlatformsView.tsx (toggle button), src/lib/ipc.ts (ipcPortToggle wrapper), src-tauri/src/commands/mod.rs (port_toggle command), src-tauri/src/forwarder.rs (listen/unlisten), src/views/TopologyView.tsx (EntryPortNode grayed + lock when disabled)

**Work**:
1. Rust: new `port_toggle(port, enabled)` command. Updates whitebox egressapikey-ports.json enabled field (atomic write), calls PortForwarder to listen (enabled=true) or stop listening (enabled=false). Resin endpoint untouched.
2. TS: `ipcPortToggle(port, enabled)` wrapper.
3. PlatformsView: each port card top-right corner Toggle switch. Calls ipcPortToggle. Toast i18n platform.portEnabled / portDisabled.
4. TopologyView EntryPortNode: if PortMapping.enabled=false → card opacity-40 + lock icon + tooltip "已禁用" (i18n topology.portDisabled). Health probe skips disabled ports.

**Tests**: cargo test (forwarder listen/unlisten on toggle), vitest (toggle calls ipcPortToggle + toast).

**Est**: ~120 lines Rust + ~60 lines TS + i18n keys × 18.

### Phase 3 — B-strategy parameter display + i18n

**Files**: src/lib/strategy.ts (b_class_params type), crates/resin-core/src/strategy_engine.rs (PlatformStrategy gains b_class_params field), src/views/TopologyView.tsx (PlatformNode B badge with params), src/locales/*/topology.json (6 new i18n keys)

**Work**:
1. Rust: PlatformStrategy gains `b_class_params: BClassParams` with optional fields: round_robin_n: Option<u32>, latency_threshold_ms: Option<u32>, quality_score: Option<u32>. serde default.
2. TS: StrategyId → i18n key mapping gains param interpolation. New i18n keys:
   - topology.bClassRandom = "真随机"
   - topology.bClassRoundRobin = "轮询 N={{n}}"
   - topology.bClassLatency = "低延时 (<{{threshold}}ms)"
   - topology.bClassQuality = "质量分>={{score}}"
   - topology.bClassBandwidth = "带宽优先"
   - topology.bClassProtocolWeight = "协议权重"
3. PlatformNode B badge: `B: {t(key, params)}` where params come from strategyConfig.b_class_params.
4. PlatformsView strategy panel: per-strategy param input (round-robin N stepper, latency threshold slider, quality score input).

**Tests**: cargo test (b_class_params serde round-trip), vitest (B badge renders params with interpolation).

**Est**: ~100 lines Rust + ~60 lines TS + 6 i18n keys × 18 locales.

### Phase 4 — Manual mode card checkbox + canvas drag guard

**Files**: src/views/TopologyView.tsx (RegionGroupNode / SubscriptionGroupNode expanded node row checkbox, onConnect isValidConnection guard), src/lib/ipc.ts (ipcStrategyConfigPut with manual_nodes update), src-tauri/src/commands/mod.rs (strategy_config_put accepts manual_nodes)

**Work**:
1. RegionGroupNode / SubscriptionGroupNode: when expanded, each node row gains a checkbox. Checkbox state = whether strategyConfig.platforms[?].manual_nodes includes row.node_hash. Toggle → update strategyConfig → strategy_apply.
2. onConnect guard: if target starts with "leaf-" or is a non-group node → reject (return early, no PATCH). Only platform→subgroup / platform→regiongroup / entry→platform allowed.
3. A-class badge on PlatformNode: if a_class=manual, badge shows "手动(N节点)" i18n topology.aClassManualCount with {{n}} = manual_nodes.len().
4. strategy_apply: if a_class=manual, strategy_engine maps manual_nodes → regions → PATCH Resin region_filters. If a_class=region, direct region_filters PATCH. If a_class=subscription, map subscription names → regions → PATCH.

**Tests**: vitest (checkbox toggles manual_nodes + strategy_apply called), cargo test (strategy_engine manual_nodes → regions mapping).

**Est**: ~150 lines TS + ~30 lines Rust.

### Phase 5 — Config-file entry toolbar reorganization

**Files**: src/views/TopologyView.tsx (CanvasControls: remove FileCog from left-bottom, add config dropdown to right-top toolbar), src/locales/*/topology.json (topology.openPortsConfig + topology.openStrategyConfig i18n keys)

**Work**:
1. Remove FileCog button from left-bottom CanvasControls (L642-645).
2. Add right-top floating toolbar (absolute top-2 right-2 z-10) with a FileCog button + dropdown menu:
   - "打开端口配置 (egressapikey-ports.json)" → opens file in OS default editor
   - "打开策略配置 (egressapikey-strategy.json)" → opens file in OS default editor
3. Both use ipcGetConfigDir + openPath. i18n: topology.openPortsConfig / topology.openStrategyConfig.

**Tests**: vitest (dropdown renders two options + click triggers openPath).

**Est**: ~40 lines TS + 2 i18n keys × 18.

### Phase 6 — Resin restart port restore from whitebox

**Files**: src-tauri/src/main.rs (post boot_resin call), src-tauri/src/commands/mod.rs (restore_ports_from_whitebox), crates/resin-core/src/whitebox_config.rs (read entry_ports + create endpoints helper)

**Work**:
1. After boot_resin success in main.rs setup(), call restore_ports_from_whitebox(whitebox, sidecar).
2. restore_ports: for each PortMapping in whitebox.entry_ports: if enabled → POST /api/v1/endpoints {port, protocol, platform_name, account, auth_required} to Resin. If Resin returns 409 (already exists) → skip. Log tracing::info! per port.
3. This closes the Q2=A loop: whitebox is truth source, Resin rebuilds from it on restart.

**Tests**: cargo test (restore_ports_from_whitebox mockito: POST endpoint for enabled port, skip for disabled, skip on 409), integration smoke (restart Resin → ports restored).

**Est**: ~80 lines Rust.

### Phase 7 — Full gate: build + i18n + test + exe smoke

**Files**: N/A (verification only)

**Work**:
1. `pnpm i18n:check` — all new keys present in 18 locales.
2. `pnpm test` — all new vitest green.
3. `cargo test -p resin-core --lib` — all new cargo green.
4. `pnpm exec tsc --noEmit` — clean.
5. `pnpm build` → new Vite chunk hash.
6. `cargo build --release -p egressapikey-app --features custom-protocol` → exe.
7. Stage exe + resin.exe sidecar to release/windows-gui/.
8. Grep Vite chunk hash in exe bytes (stale-bundle guard).
9. Smoke launch: MainWindowTitle="EgressAPIKEY", WorkingSet ~36MB, resin.exe child alive, /healthz 200.
10. `codegraph sync .` + `git push`.

**Acceptance**: all gates green, exe fresh, smoke alive.

## Test closed-loop summary

| Phase | Cargo tests | Vitest tests | Smoke |
|-------|-------------|--------------|-------|
| 1 | batch probe + backoff math | 4-state chip + lock icon | — |
| 2 | forwarder listen/unlisten | toggle + toast | — |
| 3 | b_class_params serde | B badge params interpolation | — |
| 4 | manual_nodes → regions | checkbox + strategy_apply | — |
| 5 | — | dropdown + openPath | — |
| 6 | restore_ports mockito | — | restart → ports restored |
| 7 | all green | all green | exe alive + resin healthy |
