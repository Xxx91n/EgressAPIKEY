# HANDOFF T5 — Trace Skeleton + IpcError Contract + Bug Fixes (2026-08-10)

> All 10 grill questions answered. ADR-0026 ACCEPTED.
> CONTEXT.md updated with 3 new terms: Trace ID, IpcError, Account Strategy Tag.
> Domain audit: docs/GRILL_T5_DOMAIN_AUDIT.md.
> Decisions: docs/GRILL_T5_TRACE_IPC_BACKLOG.md.
> ADR: docs/adr/0026-accountid-bclass-strategy-ipc-trace.md.

## Total Plan Table

| Phase | Items | Scope | Est. Lines | Tests | Dependencies |
|-------|-------|-------|-----------|-------|---------------|
| 5-1 | Q1-Q5 trace skeleton | invokeWithTrace TS + trace.rs Rust + ResinClient #[instrument] | ~70 Rust + ~20 TS | 6 cargo + 3 vitest | None |
| 5-2 | Q6-Q8 IpcError contract | IpcError enum + map_resin_error + 44 command refactor + TS extractIpcErr | ~150 Rust + ~50 TS | 8 cargo + 6 vitest | Phase 5-1 |
| 5-3 | Bug 1-4 fixes | build-all.ps1 exact match + b_class default + bind conflict + protocol-aware prober | ~60 | 4 vitest | Phase 5-2 |
| 5-4 | Q10 B-class scheme c | account_for_bclass + strategy_engine B-class wiring + GUI B-class selector | ~100 Rust + ~30 TS | 4 cargo + 3 vitest | Phase 5-2 + 5-3 |
| 5-5 | i18n + build + push | 18-locale keys + build-all.ps1 + smoke + push | ~0 | i18n:check | All prior |

## Phase 5-1: Trace Skeleton (Q1-Q5)

### Frontend (src/lib/ipc.ts)
- new `invokeWithTrace(cmd: string, args?: Record<string, unknown>): Promise<unknown>`
  - generates `const trace_id = crypto.randomUUID()`
  - injects `__trace_id: trace_id` into args
  - calls `invoke(cmd, args)`
- console.log `[trace_id]` for frontend-side correlation
- each existing wrapper (ipcPlatformAdd, ipcPlatformRemove, ...) replaced to call invokeWithTrace

### Backend (src-tauri/src/trace.rs)
- `pub fn extract_trace_id(args: &serde_json::Value) -> Option<String>`
  - reads `args["__trace_id"]`, validates UUID v4 format, returns Some(id) or None
- `pub fn trace_ipc_span(cmd: &str, trace_id: &str) -> tracing::Span`
  - returns `tracing::info_span!("ipc.<cmd>", trace_id = %trace_id)`
- each #[tauri::command] entry: `let _span = trace_ipc_span("platform_add", &extract_trace_id(&args)?.unwrap_or("none")).entered();`
- ResinClient methods: add `#[instrument(skip(self), fields(trace_id))]` (tracing auto-propagates)

### Tests
- cargo: extract_trace_id_valid_uuid, extract_trace_id_rejects_garbage, extract_trace_id_none_when_missing, trace_ipc_span_carries_id (4)
- cargo: instrument_span_propagates_to_subspan, instrument_span_on_error_logs_trace_id (2)
- vitest: invokeWithTrace_always_carries_trace_id, invokeWithTrace_generates_uuid_v4, invokeWithTrace_injects_into_args (3)

## Phase 5-2: IpcError Contract (Q6-Q8)

### Rust (crates/resin-core/src/ipc_error.rs)
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum IpcError {
    BindConflict { port: u16, i18n_key: String },
    InvalidStrategy { value: String, accepted: Vec<String>, i18n_key: String },
    ResinUpstream { status: u16, excerpt: String, i18n_key: String },
    Internal { msg: String, i18n_key: String },
}
```
- `pub fn map_resin_error(status: u16, body: &str) -> IpcError`
- each #[tauri::command] return type: `Result<T, IpcError>`
- `impl From<serde_json::Error> for IpcError` (Internal)
- `impl From<reqwest::Error> for IpcError` (ResinUpstream)

### Frontend (src/lib/ipc.ts)
```typescript
type IpcError =
  | { kind: "BindConflict"; data: { port: number; i18n_key: string } }
  | { kind: "InvalidStrategy"; data: { value: string; accepted: string[]; i18n_key: string } }
  | { kind: "ResinUpstream"; data: { status: number; excerpt: string; i18n_key: string } }
  | { kind: "Internal"; data: { msg: string; i18n_key: string } };

function extractIpcErr(e: unknown): IpcError { ... }
```
- GUI catches IPC errors, calls extractIpcErr, routes to i18n key

### Tests
- cargo: IpcError serde round-trip 4 variants (4)
- cargo: map_resin_error bind_conflict, map_resin_error invalid_strategy, map_resin_error upstream_5xx, map_resin_error unknown_fallback, map_resin_error 403_forbidden, map_resin_error 400_validation (6)
- vitest: extractIpcErrBindConflict, extractIpcErrInvalidStrategy, extractIpcErrResinUpstream, extractIpcErrInternal, extractIpcErrI18nKeyPresent, extractIpcErrTemplateParams (6)

## Phase 5-3: Bug Fixes (Bug 1-4)

- Bug 1: scripts/build-all.ps1 Step 5 `$bundles` filter: `-match "EgressAPIKEY"` -> `Name -eq "egressapikey-headless.exe"` + exclude deps/
- Bug 2: src/views/PlatformsView.tsx L86 default `b_class: "balanced"` -> `b_class: "random"`; L456 same
- Bug 3: covered by IpcError::BindConflict (Phase 5-2)
- Bug 4: src-tauri/src/commands/mod.rs port_health_check: add protocol param, branch on socks5/http/none; GUI PortAuthInfo: conditional SOCKS5 creds display based on protocol

## Phase 5-4: B-Class Strategy Scheme C (Q10)

### Rust (crates/resin-core/src/strategy_engine.rs)
- new `pub fn account_for_bclass(strategy: StrategyId, port_label: &str) -> String`
  - Random -> `port_label::random`
  - Sequential -> `port_label::rr-1` (shell rotates account index)
  - Latency -> `port_label::latency`
  - Quality -> `port_label::quality`
  - Bandwidth -> `port_label::bandwidth`
  - ProtocolWeight -> `port_label::proto`
- new `pub fn account_for_fixed(port_label: &str) -> String` -> `port_label::fixed`
- round-robin: shell maintains an `AtomicUsize` counter per port, appends `::rr-{N}` where N rotates 0..max_nodes

### GUI (src/views/PlatformsView.tsx)
- B-class selector sends strategy tag to port creation
- port creation embeds account string into Entry Port Mapping
- display: show account strategy tag in port card

### Tests
- cargo: account_for_bclass_random, account_for_bclass_round_robin_rotation, account_for_bclass_latency_maps_to_prefer_low_latency, account_for_fixed_single_lease (4)
- vitest: bclass_selector_sends_tag, port_card_displays_strategy_tag, bclass_selector_changes_reflect_in_port_mapping (3)

## Phase 5-5: i18n + Build + Push

- Add i18n keys to all 18 locales:
  - error.bindConflict (with {{port}})
  - error.invalidStrategy (with {{value}} {{accepted}})
  - error.resinUpstream (with {{status}})
  - error.internal
- pnpm i18n:check
- pnpm build (tsc + vite)
- cargo build --release -p egressapikey-app --features custom-protocol
- scripts/build-all.ps1
- Smoke: MainWindowTitle + resin.exe child + release exe
- codegraph sync .
- git add + commit + push

## Total Test Budget: 23 new tests
- cargo: 18 (6 trace + 8 IpcError + 4 B-class)
- vitest: 12 (3 trace + 6 IpcError + 3 B-class)
- 23 tests matches Q8b estimate