# GRILL T15 — Performance Fix Branch (Post-T14 Remaining Issues)

> **Branch**: codex/rust-port (parallel fix branch)
> **Base commit**: 8dac8e6 (after T15 canvas bugfix)
> **Status**: All Q1-Q10 decisions user-approved. Plan ready for execution.
> **Ponytail**: full mode.

## Decisions (Q1-Q10)

| # | Area | Decision | Template Reference |
|---|------|----------|-------------------|
| Q1 | Sidecar process leak test | (A) Real process smoke test + ctrlc signal handling | Zed PR#58885, crynta/terax-ai job.rs |
| Q2 | Lightweight mode tray entry | (A) Menu item dynamic toggle (Normal/Entering/InLightweight) | clash-verge-rev tray mode |
| Q3 | Ctrl+C signal handling | (A) ctrlc crate + ShutdownFlag, cross-platform | cargo job.rs, kimi research |
| Q4 | 7 perf domains all done | (A) All 9 subtasks in one pass | kimi + exa research |
| Q5 | reqwest::Client reuse | (A) OnceLock inside ResinClient | clash-verge-rev commit bfe2471 |
| Q6 | Startup lazy loading | (A) All 6 Views lazy + Suspense | psysonic PR#249 |
| Q7 | Backend sync migration | (A) Only migrate DiagnosticsView to usePoll | -- |
| Q8 | Runtime log level control | (A) set_log_level IPC + EnvFilter reload + Settings dropdown | clash-verge-rev log.rs |
| Q9 | Canvas perf (React.memo + throttle) | (A+B) React.memo on 4 custom node components + onMove rAF throttle | ReactFlow perf guide |
| Q10 | Test strategy | (A) Diff-level closed-loop: 3 cargo + 3 vitest = 6 tests | AGENTS S4 |

## Execution Plan (5 Phases)

### T15-1: DiagnosticsView to usePoll (Q7)

**Root cause**: DiagnosticsView (499 lines) still uses hand-rolled setInterval + document.hidden check (lines 65-122). usePoll hook exists in src/hooks/usePoll.ts and already handles visibility pause/resume + AbortController cleanup. This is the only View not using usePoll.

**Changes**:
- File: src/views/DiagnosticsView.tsx — remove intervalRef (L65), two useEffect blocks (L68, L104), visibilitychange listener (L116-122). Replace with usePoll(10000, refreshDiagnostics, { fireImmediately: true }).
- File: src/views/DiagnosticsView.test.tsx — add 1 vitest: verify document.hidden pauses polling.

**Validation**: vitest pass + tsc green

### T15-2: Runtime log level control (Q8)

**Root cause**: tauri_plugin_tracing Builder at main.rs L60-67 uses with_max_level(LevelFilter::INFO) — hardcoded, no runtime reload.

**Changes**:
- File: src-tauri/src/main.rs — store tracing reload handle. The tauri_plugin_tracing plugin may not expose its internal subscriber reload handle. Approach: check if the plugin exposes a reload handle; if not, build a custom tracing_subscriber with EnvFilter::reload and store in State<LogLevelHandle>. Fallback: add an ArcAtomicU8 log level gate that wraps tracing macros.
- File: src-tauri/src/commands/mod.rs — new set_log_level(level: String) IPC command. Validate level in {error,warn,info,debug}. Apply to reload handle or atomic gate.
- Register in main.rs invoke_handler: commands::set_log_level.
- File: src/lib/ipc.ts — new ipcSetLogLevel(level) wrapper.
- File: src/views/SettingsView.tsx — dropdown in Settings (error/warn/info/debug), persisted to settings.json#logLevel.
- i18n: add settings.logLevel + settings.logLevelError/Warn/Info/Debug keys across 18 locales.

**Tests**:
- 2 cargo: set_log_level param validation (invalid rejected, valid accepted) + level enum round-trip.
- 1 vitest: Settings dropdown change calls invoke("set_log_level", { level: "warn" }).

**Validation**: cargo test pass + vitest pass + tsc green + i18n:check green

### T15-3: Canvas perf — React.memo + onMove throttle (Q9 A+B)

**Root cause**: PlatformNode (L214), RegionGroupNode, SubscriptionGroupNode, EntryPortNode are plain function components with no React.memo. Every store state change re-renders all nodes. onMove is not bound (only onMoveEnd is at L542) — so the throttle part may be a no-op if onMove is already absent.

**Changes**:
- File: src/views/TopologyView.tsx
- Wrap all 4 custom node components with React.memo: MemoPlatformNode = React.memo(PlatformNode), same for others.
- Update nodeTypes map (L351) to use memoized versions.
- Check if onMove is bound: if not, skip rAF throttle (no work needed). If yes, gate state updates with requestAnimationFrame.
- Add areEqual custom comparator if shallow props comparison is insufficient (data object may be new each render).

**Tests**:
- 2 vitest: (1) React.memo — pass same data object reference, verify component render count = 1. (2) onMove (if bound) — verify no state update beyond rAF budget.

**Validation**: vitest pass + tsc green + pnpm build green (chunk size should not regress)

### T15-4: reqwest::Client reuse (Q5)

**Root cause**: ResinClient::new() creates a new reqwest::Client per call. IPC layer constructs a fresh ResinClient for each command, building a new TLS pool + connection pool each time.

**Changes**:
- File: crates/resin-core/src/resin_client.rs
- Add static CLIENT: OnceLock<reqwest::Client> = OnceLock::new() + fn shared_client() -> and use it in ResinClient::new().
- No API change — callers see the same ResinClient interface.

**Test**: 1 cargo — verify shared_client() returns same instance (pointer eq).

**Validation**: cargo test pass + cargo build green

### T15-5: Build + test + smoke + i18n + doc + push (Q10 final)

**Changes**:
- pnpm build -> note chunk hash
- cargo build --release -p egressapikey-app --features custom-protocol
- Stage exe to release/windows-gui/
- Grep chunk hash in exe bytes
- Smoke: MainWindowTitle + WorkingSet + resin child alive
- pnpm i18n:check all 18 locales
- cargo test -p resin-core --lib
- pnpm test
- New ADR: docs/adr/0037-t15-perf-fixes.md
- Update docs/GRILL_T15_PERFORMANCE_FIX.md (this file)
- Update AGENTS.md with T15 section
- Commit + push

**Validation**: ALL green

## Dependencies: NONE new
- React.memo: React 19 stdlib
- requestAnimationFrame: browser stdlib
- OnceLock: Rust stdlib
- tracing reload: tracing_subscriber (already via tauri-plugin-tracing)

## Risk Assessment
- T15-1: Low — replacing setInterval with usePoll, proven pattern from T14-3
- T15-2: Medium — tracing reload handle may not be exposed by tauri_plugin_tracing; fallback is atomic gate
- T15-3: Low — React.memo is non-breaking
- T15-4: Low — OnceLock is non-breaking
- T15-5: Standard build+smoke pipeline

## Completion Criteria
- [x] T15-1: DiagnosticsView uses usePoll, 1 vitest pass (DONE)
- [x] T15-2: set_log_level IPC works, 2 cargo + 1 vitest pass, i18n keys added (DONE)
- [x] T15-3: 4 custom node components wrapped in React.memo, 2 vitest pass (DONE)
- [x] T15-4: OnceLock<Client> in resin_client.rs, 1 cargo pass (DONE)
- [x] T15-5: ALL green (cargo test=120, vitest=222, tsc clean, i18n 18 locales 332 keys, smoke exe 36.8MB+resin 50.1MB, chunk hash verified, push) (DONE)