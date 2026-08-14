# GRILL T14 — Performance Optimization Plan

> **Status**: All Q1-Q8 decisions user-approved. Plan ready for execution.
> **Target**: Reduce memory from 171MB → 50-80MB, prevent resin.exe orphan leak, industrial-grade performance with closed-loop tests.

## Decisions (Q1-Q8)

| # | Area | Decision | Template Reference |
|---|------|----------|-------------------|
| Q1 | Sidecar orphan prevention | (A) Windows Job Object | clash-verge-rev PR #6853 |
| Q2 | WebView2 memory reduction | (A) Full optimization | clash-verge-rev + Taskbar Sentinel |
| Q3 | IPC cache strategy | (C) Introduce SWR library (4.2KB) | Vercel SWR |
| Q4 | Lightweight mode | (A) clash-verge-rev full state machine + EmptyWorkingSet | clash-verge-rev lightweight.rs + cockpit-tools PR #686 |
| Q5 | Poll pause | (A) usePoll unified hook | Yerd usePoll pattern |
| Q6 | zustand shallow selector | (A) Full migration across all 6 Views | React 19 + zustand/shallow |
| Q7 | SWR integration | (A) Unified useIpc hook layer | SWR + Tauri IPC bridge |
| Q8 | Additional perf areas | (A) Both: emit/Channel guard + NodesView virtualization | Tauri issue #13758 + @tanstack/react-virtual |

## Execution Plan (9 Phases)

### T14-1: Windows Job Object (sidecar.rs)
- Add `windows-sys` crate to `src-tauri/Cargo.toml`
- After `Command::spawn`, call `CreateJobObjectW` + `SetInformationJobObject` (JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE) + `AssignProcessToJobObject`
- Store `OwnedHandle` for job in `SidecarHandle` (RAII closes handle → OS kills child)
- On `RunEvent::Exit`: existing `child.kill()` + `child.wait()` stays; Job Object is belt-and-suspenders for Task Manager force-kill
- Known limitation: spawn→assign race window (document, same as clash-verge-rev PR #6853)
- Test: 2 cargo tests (job_handle_drops_kills_child, assign_fails_returns_err)
- Files: `src-tauri/src/sidecar.rs`, `src-tauri/Cargo.toml`
- Validation: cargo test pass

### T14-2: Lightweight mode (lightweight.rs)
- New file `src-tauri/src/lightweight.rs`: `LightweightState` enum (Normal/In/Exiting) + `try_transition` CAS
- `setup_window_close_listener`: on `tauri://close-requested` start delay timer (default 10min, configurable in Settings)
- `setup_webview_focus_listener`: on `tauri://focus` cancel delay timer
- `entry_lightweight_mode`: `window.destroy()` + tray update + `EmptyWorkingSet` (Windows) / `malloc_trim(0)` (Linux) / `malloc_zone_pressure_relief` (macOS)
- `exit_lightweight_mode`: `WebviewWindowBuilder::from_config` rebuild + restore listeners
- i18n: add `settings.lightweightMode` + `settings.autoLightweightMinutes` keys across 18 locales
- Settings: add toggle + minutes input in SettingsView
- Test: 6 cargo tests (state transitions, CAS, timer cancel)
- Files: `src-tauri/src/lightweight.rs`, `src-tauri/src/main.rs`, `src/views/SettingsView.tsx`, `src/store/appStore.ts`, locale files
- Validation: cargo test pass + pnpm i18n:check green

### T14-3: usePoll unified hook (src/hooks/usePoll.ts)
- New `src/hooks/usePoll.ts`: `usePoll(intervalMs, asyncFn)` hook
- Auto-pause on `document.hidden` → `clearInterval`
- Auto-resume on visible → immediate `asyncFn()` + restart `interval`
- Unmount → `clearInterval` + `AbortController.abort()`
- In-flight dedup via `useRef<AbortController | null>`
- Migrate all 6 Views: replace their `setInterval/useEffect` polling with `usePoll`
- Test: 6 vitest (pause on hidden, resume on visible, abort on unmount, dedup in-flight, interval cleanup, immediate sync on focus)
- Files: `src/hooks/usePoll.ts`, all 6 View files
- Validation: vitest pass + tsc green

### T14-4: zustand shallow selector migration
- `appStore.ts`: keep structure, add `export const useShallowAppStore = (selector) => useAppStore(selector, shallow)`
- All 6 Views: replace `const { x, y } = useAppStore()` with individual selectors
- Each View subscribes only to its own slice
- Test: 3 vitest (shallow skip same value, partial update triggers only relevant component, full store update still works)
- Files: `src/store/appStore.ts`, all 6 View files
- Validation: vitest pass + tsc green

### T14-5: SWR library + useIpc hook (src/hooks/useIpc.ts)
- `pnpm add swr`
- New `src/hooks/useIpc.ts`: `useIpc(key, fetcher, opts?)` wrapping `useSWR`
- Stable IPC dedup key generation + default 10s TTL (`dedupingInterval: 10000`)
- AbortController integration + Tauri-external graceful fallback
- Migrate 6 Views: replace `useEffect + invoke` with `const { data, error, mutate } = useIpc("key", ipcXxx)`
- Background refresh: `usePoll(10s, () => mutate())` for live data
- Test: 6 vitest (cache hit, cache expiry, dedup, abort on unmount, Tauri-external fallback, mutate refresh)
- Files: `src/hooks/useIpc.ts`, `package.json`, all 6 View files
- Validation: vitest pass + tsc green

### T14-6: emit/Channel guard + documentation
- Audit all `app_handle.emit()` calls: verify payload size < 1KB (small = OK)
- Document in AGENTS.md: "emit for small events (<1KB), ipc::Channel for streaming/large data"
- Add lint comment in `commands/mod.rs` at each emit site
- Test: 2 cargo tests (emit payload size check, channel streaming smoke)
- Files: `src-tauri/src/commands/mod.rs`, `src-tauri/src/sidecar.rs`, `AGENTS.md`
- Validation: cargo test pass

### T14-7: NodesView virtualization (@tanstack/react-virtual)
- `pnpm add @tanstack/react-virtual`
- Rewrite NodesView table to use `useVirtualizer` for 295+ rows
- Only visible rows (~20-50) render in DOM
- Test: 3 vitest (virtual render only visible, scroll loads more, row click still works)
- Files: `src/views/NodesView.tsx`, `package.json`
- Validation: vitest pass + tsc green

### T14-8: Build + test + smoke + i18n check
- `pnpm build` → note chunk hash
- `cargo build --release -p egressapikey-app --features custom-protocol`
- Stage exe to `release/windows-gui/`
- Grep chunk hash in exe bytes
- Smoke: MainWindowTitle + WorkingSet + resin child alive
- `pnpm i18n:check` all 18 locales
- `cargo test -p resin-core --lib`
- `pnpm test`
- Validation: ALL green

### T14-9: Doc update + ADR + push
- New `docs/adr/0035-performance-optimization.md`
- Update `AGENTS.md` with new sections: lightweight mode, usePoll, useIpc, SWR cache
- Update `docs/GRILL_T14_PERFORMANCE.md` (this file)
- Commit + push
- Validation: git push success

## Research References

- clash-verge-rev PR #6853: Windows Job Object for sidecar orphan prevention
- clash-verge-rev lightweight.rs: full state machine + auto timer + listener management
- cockpit-tools PR #686: Destroy WebView + EmptyWorkingSet/malloc_trim/malloc_zone_pressure_relief
- cc-switch: AtomicBool LIGHTWEIGHT_MODE + window.destroy()
- Taskbar Sentinel: 12MB install / 35MB idle via webview on-demand loading
- PowerShift: UI separated from background agent (346MB UI vs 13MB background)
- Yerd usePoll: document.visibilityState pause + in-flight dedup
- wry 0.35: WebViewExtWindows::set_memory_usage_level API
- Tauri issue #13758: Eval/ExecJS memory leak, use ipc::Channel for large data
- LogRocket SWR vs TanStack Query: SWR 4.2KB vs TanStack 11.4KB
- Microsoft WebView2 perf docs: MemoryUsageTargetLevel.Low + TrySuspendAsync
- Tauri IPC bandwidth wall: large Vec<u8> over invoke = JSON array (use custom protocol instead)

## New Dependencies
- `windows-sys` crate (T14-1) — Windows Job Object API
- `swr` npm package (T14-5) — ~4.2KB gzipped, Vercel SWR
- `@tanstack/react-virtual` npm package (T14-7) — ~2.5KB gzipped, list virtualization
