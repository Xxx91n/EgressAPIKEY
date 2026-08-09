# Grill T5 Issues Backlog — Trace Skeleton + IpcError Contract (2026-08-10)

> All 10 grill questions answered. ADR-0026 to be created.
> This grill round covers enterprise trace infrastructure + IPC error
> contract + 4 bug fixes + B-class strategy alignment.
> ADR-0012 thin-shell multi-port route correction is the architecture baseline.

## Decisions Summary

| Q  | Topic                           | Decision | ADR    |
|----|---------------------------------|----------|--------|
| Q1 | Trace skeleton priority         | B — enterprise trace first, bugs second | (this) |
| Q2 | Trace mechanism                 | A — lightweight trace_id passthrough (no OTel SDK) | (this) |
| Q3 | Trace injection path             | Path 甲 — __trace_id in args + Rust helper extract (path 丙 Tauri2 header not supported per PWM) | (this) |
| Q4 | Trace ID format                 | a — UUID v4 via crypto.randomUUID() | (this) |
| Q5 | Span layers                     | b — double-layer span (command info_span + ResinClient #[instrument]) | (this) |
| Q6 | IPC error contract              | C — full IpcError enum + all 44 commands + GUI variant-driven i18n | ADR-0026 |
| Q7 | IpcError serialization          | c — IpcError enum + i18n_key inline + externally-tagged serde | ADR-0026 |
| Q8 | Test coverage depth             | b — 23 new tests (14 cargo + 9 vitest) covering every error path | (this) |
| Q9 | Port health prober              | a — protocol-aware prober (SOCKS5 greeting vs HTTP CONNECT vs no-auth) + GUI conditional credentials display | (this) |
| Q10| B-class strategy alignment     | c — accountId encodes B-class strategy hint (port-based, no interceptor, no route_id) | ADR-0026 |

## Bug Root Causes (Code-Level Verified)

### Bug 1: Duplicate headless exe
**Root cause**: `scripts/build-all.ps1` Step 5 `$bundles` uses `-match "EgressAPIKEY"` (PowerShell -match is case-insensitive + substring). It catches both `target/.../release/egressapikey-headless.exe` (hyphen) AND `target/.../release/deps/egressapikey_headless.exe` (underscore in deps/). 
**Fix**: Exact match `Name -eq "egressapikey-headless.exe"` + exclude `deps/` directory.
**No grill needed — direct fix.**

### Bug 2: "strategy config invalid: unknown variant `balanced`"
**Root cause**: `crates/resin-core/src/strategy.rs` `StrategyId::parse` accepts `random/sequential/latency/quality/bandwidth/protocol_weight` but NOT `balanced` (Resin's native value). `PlatformsView.tsx` L86/L456 defaults `b_class: "balanced"`, which parse rejects.
**Fix**: Q7 IpcError::InvalidStrategy + ADR-0026 alignment of GUI default value.

### Bug 3: POST /endpoints 409 bind conflict
**Root cause**: Port already in use; raw EADDRINUSE error string bubbles up with no i18n handling.
**Fix**: Q6 map_resin_error → IpcError::BindConflict { port } → GUI i18n friendly message.

### Bug 4: HTTP port health check uses SOCKS5 greeting + shows SOCKS5 creds for all
**Root cause**: `src-tauri/src/commands/mod.rs:1757` `port_health_check` sends SOCKS5 3-byte greeting `05 01 02` for ALL protocols. `PortAuthInfo` doc says SOCKS5 username/password always filled because RESIN_PROXY_TOKEN forces auth.
**Fix**: Q9 — protocol-aware prober + GUI conditional display.

## Execution Phases

### Phase 5-1: Trace Skeleton (Q1-Q5)
- **Frontend**: `src/lib/ipc.ts` new `invokeWithTrace(cmd, args)` wrapper: generates `crypto.randomUUID()`, injects `__trace_id` into args. Each existing wrapper calls invokeWithTrace instead of raw invoke.
- **Backend**: `src-tauri/src/trace.rs` new helper: `extract_trace_id(args: &serde_json::Value) -> Option<String>` + `trace_ipc_span(cmd: &str, trace_id: &str) -> tracing::Span`. Each `#[tauri::command]` entry calls `let span = trace_ipc_span("platform_add", &trace_id).entered();`.
- **ResinClient**: each method decorated with `#[instrument(skip(self))]`.
- **Tests**: 6 cargo (trace_id extraction 4 + instrument span 2) + 3 vitest (invokeWithTrace always carries trace_id, UUID format, args injection).
- **No new crates** — tracing + uuid already in deps.

### Phase 5-2: IpcError Contract (Q6-Q8)
- **Rust**: New `IpcError` enum in `crates/resin-core/src/ipc_error.rs`:
  ```rust
  pub enum IpcError {
      BindConflict { port: u16 },
      InvalidStrategy { value: String, accepted: Vec<String> },
      ResinUpstream { status: u16, excerpt: String },
      Internal { msg: String },
  }
  ```
  Each variant carries `i18n_key: &str` + template params. Externally-tagged serde.
- **map_resin_error**: in `resin_client.rs` or `commands/mod.rs`, map Resin HTTP 409+wildcard "bind" → BindConflict, 400 "must be BALANCED" → InvalidStrategy, other 4xx/5xx → ResinUpstream.
- **All 44 commands**: return `Result<T, IpcError>` instead of `Result<T, String>`.
- **Frontend**: `src/lib/ipc.ts` new `extractIpcErr(e: unknown): IpcError` + discriminated union + i18n key routing.
- **Tests**: 8 cargo (IpcError serde round-trip 4 variants + map_resin_error 6 branches) + 6 vitest (extractIpcErr 4 variants + i18n_key + template params).
- **i18n**: add error keys to all 18 locales.

### Phase 5-3: Bug Fixes (Bug 1-4)
- Bug 1: `scripts/build-all.ps1` exact match fix (~5 lines).
- Bug 2: `PlatformsView.tsx` default `b_class` from `"balanced"` to `"random"` (aligned with strategy.rs).
- Bug 3: already handled by IpcError::BindConflict from Phase 5-2.
- Bug 4: `port_health_check` protocol branch (SOCKS5 greeting / HTTP CONNECT / no-auth probe) + GUI conditional creds display (~50 lines).

### Phase 5-4: B-Class Strategy Scheme C (Q10 / ADR-0026)
- **Architecture**: accountId encodes B-class strategy hint. When the shell creates a port mapping (port → platform + account), it encodes the B-class strategy into the account string. Resin receives the account value and creates a lease per unique account → different exit IPs for different strategies on the same platform.
- **Account format**: `<port_label>::<strategy_tag>` where strategy_tag is one of:
  - `random` — Resin allocation_policy BALANCED (random node selection)
  - `rr-N` — round-robin with N rotations (shell creates N accounts, rotates per request)
  - `latency` — Resin PREFER_LOW_LATENCY
  - `fixed` — single IP, no rotation, one account
- **Shell-side**: `strategy_engine.rs` BClassStrategy enum + account_for_bclass() function.
- **GUI**: PlatformsView B-class selector sends the strategy tag; strategy_engine generates appropriate account string when creating port mappings.
- **Tests**: 4 cargo (account generation + strategy mapping + lease isolation + round-robin rotation) + 3 vitest (GUI B-class selector + account display).
- **No fork of Resin** (ADR-0012). No interceptor, no route_id (ADR-0014). Account string is opaque to Resin; the shell owns strategy semantics.

## Resin v1.2.0 Source Reference (Verified)
- `resin/internal/proxy/identity_v1.go`: parseV1PlatformAccountIdentity parses `Platform.Account` or `Platform:Account`. Account is an opaque string to Resin.
- `resin/internal/routing/router.go:89`: `if account == "" { random route } else { routeSticky(plat, state, account, target) }`.
- `resin/internal/platform/policy.go:7-9`: AllocationPolicy = BALANCED | PREFER_LOW_LATENCY | PREFER_IDLE_IP (3 values only).
- `resin/internal/routing/lease.go`: LeaseTable key = account. Different accounts get different leases → different exit IPs.
- `resin/internal/service/control_plane_endpoint.go:48-57`: CreateEndpointRequest has NO account field. Port IS the entry point; account comes from Proxy-Authorization credential.
