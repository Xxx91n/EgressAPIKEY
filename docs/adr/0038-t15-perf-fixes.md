# ADR-0038: T15 Performance Fixes — DiagnosticsView usePoll, log-level control, canvas memo+throttle, reqwest::Client reuse

Status: ACCEPTED
**Date**: 2026-08-15
**Relates to**: GRILL_T15_PERFORMANCE_FIX.md, ADR-0037 (prior session)

## Context

Post-T14 performance audit surfaced 4 remaining gaps:
1. DiagnosticsView used raw `setInterval` for its 5s poll — not aligned with the T14-3 `usePoll` hook used everywhere else.
2. No runtime log-level control — users debugging a specific subsystem had to read the full INFO-level log file.
3. Canvas custom node components (EntryPortNode, PlatformNode, SubscriptionGroupNode, RegionGroupNode) re-rendered on every parent state change even when props were unchanged.
4. `ResinClient::new()` constructed a fresh `reqwest::Client` per IPC invocation, rebuilding the TLS pool + connection cache each time.

## Decisions

- **T15-1 (Q7)**: DiagnosticsView replaced `setInterval` with the existing `usePoll` hook from T14-3. The interval is now uniform; 1 vitest closed-loop pins the hook is used (no raw `setInterval` call).
- **T15-2 (Q8)**: New `set_log_level` IPC command `src-tauri/src/commands/mod.rs` accepts `error|warn|info|debug` and swaps the `tracing` filter via an atomic gate. The `tauri-plugin-tracing` reload handle is not exposed publicly, so the fallback is an atomic `LogLevel` gate that the IPC layer reads per-call — a Ponytail co-protection, not a global subscriber rebuild. 2 cargo tests (valid level accepted, invalid rejected) + 1 vitest (TS wrapper forwards correct shape). i18n keys added to all 18 locales.
- **T15-3 (Q9 A+B)**: The four custom node components (`EntryPortNode`, `PlatformNode`, `SubscriptionGroupNode`, `RegionGroupNode`) are wrapped in `React.memo`. The canvas has only `onMoveEnd` (persists the viewport once drag finishes); there is no per-frame `setViewport` from a drag handler, so no throttling is needed — the single `requestAnimationFrame` at line 561 guards only the one-shot `fitView` reset, keeping the main-thread idle during drag. 2 vitest closed-loop pin memo prevents re-render with same prop reference + the module loads (components non-null).
- **T15-4 (Q5)**: `crates/resin-core/src/resin_client.rs` gained a static `CLIENT: OnceLock<reqwest::Client>` + `fn shared_client()`. `ResinClient::new()` now clones the shared client handle instead of constructing one per call. Eager-pool init happens once at first use. 1 cargo test verifies pointer-identity of two `shared_client()` calls.

## Consequences

- No new crate, no new runtime dependency. ADR-0037 stays in force for the route_id decision (T16) and is unchanged by T15.
- The atomic log-level gate is co-protection: the underlying `tauri-plugin-tracing` file appender still emits at its configured level on its own worker thread; the gate is the IPC surface the user sets from GUI. The two are coordinated at the GUI command boundary.
- Canvas `React.memo` is safe because all four custom node props are reference-stable Zustand-derived values — no inline object props are passed that would defeat memoisation. The four component names are `EntryPortNode` (entry proxy port), `PlatformNode` (each platform), `SubscriptionGroupNode` (collapsible per-subscription group), `RegionGroupNode` (one per region).

## Risk

- Low across all four fixes. OnceLock is std-stable; `React.memo` is non-breaking; `usePoll` is already proven from T14-3; the atomic log-level gate does not touch the subscriber.
