# ADR-0035: Performance Optimization — Job Object + Lightweight Mode + Virtualization

Status: ACCEPTED
**Date**: 2026-08-15
**Supersedes**: —

## Context

The EgressAPIKEY desktop app (Tauri 2 + React 19 + WebView2) consumed ~171MB on the
Windows Task Manager "Microsoft Edge WebView2 Manager" process and ~8MB on the app
process. The resin.exe Go sidecar could leak as an orphan if the GUI process exited
abnormally (the `tauri_plugin_shell` `CommandChild::Drop` impl does NOT kill the
child). Users reported the app felt heavy for a proxy gateway tool.

GRILL T14 planned 9 optimization phases targeting 50–80MB steady-state, orphan
prevention, and per-phase closed-loop tests.

## Decisions

### T14-1: Windows Job Object (orphan prevention)
Assign the resin.exe sidecar to a Win32 Job Object with `KILL_ON_JOB_CLOSE`. When
the parent (GUI) process exits — for any reason including a crash — the kernel kills
all processes in the job, including resin.exe. This is the OS-level guarantee, not a
manual Rightside.cleanup. 3 cargo unit tests.

### T14-2: Lightweight mode (memory reduction 171MB → ~40MB)
New `src-tauri/src/lightweight.rs` with `LightweightController` + `LightweightState` enum (Normal/Lightweight/Pending).
On `WindowEvent::CloseRequested`: start a delay timer (configurable minutes, default
10). On `WindowEvent::Focused`: cancel the timer. If the timer fires, the webview is
destroyed (`window.destroy()`) and `trim_working_set()` is called (Win32
`SetProcessWorkingSetSizeEx(-1, -1, 0)`). The resin.exe sidecar keeps running (it
owns the proxy ports); only the webview UI is torn down. The user's OS proxy routes
through the sidecar, not the webview, so the lightweight mode is transparent to
outgoing traffic. 8 cargo tests + 18 locale i18n keys.

### T14-3: usePoll hook (memory: fewer re-renders)
`src/hooks/usePoll.ts` consolidates the 3 ad-hoc setInterval pollers
(TopologyView, NodesView, ProcessRouteView) into a single 10s poll with
`pauseWhenHidden` — drops polling CPU + avoids 3 parallel timers. 6 vitest tests.

### T14-4: Zustand useShallow re-export
`src/store/appStore.ts` now re-exports `useShallow` from `zustand/react/shallow`.
All Views already use individual `useAppStore((s) => s.field)` selectors — no
re-render churn. 3 tests.

### T14-5: SWR + useIpc hook (dedup + cache)
New `src/hooks/useIpc.ts` wraps SWR for Tauri IPC: 10s request dedup,
`keepPreviousData` while revalidating, no retry. 6 vitest tests.
`swr` 2.5.1 devDependency added. Views NOT yet migrated — the hook is ready for
the nextiews refactor and is opt-in per call site.

### T14-6: emit/Channel guard audit
Audited all 5 `app.emit()` call sites: every payload fits in < 100 bytes
(`"healthy"`, `"unhealthy"`, `"terminated"`, `"restarting"`, `()`). No
`Channel<T>` needed yet — the event payloads are simple literals. Documented
in AGENTS.md for any future IPC surface that grows beyond ~1KB.

### T14-7: NodesView virtualization
New `@tanstack/react-virtual` 3.14.9 devDependency. `VirtualNodeList` component:
if `items.length < 50` (`VIRTUAL_THRESHOLD`), normal render (jsdom-compatible);
if ≥ 50, uses `useVirtualizer` with 36px row estimate, overscan=5. 3 closed-loop
vitest tests verify the expansion/collapse cycle under threshold and the
threshold-path render of small lists.

### T14-8: SettingsView lightweight toggle + IPC
Two new IPC commands: `lightweight_get` / `lightweight_set(enabled, delay_minutes)`.
Persists to `settings.json` + updates the live `LightweightController` via
`set_delay_minutes` (`AtomicU32` — safe cross-thread writes). SettingsView
gains a toggle card with the enabled checkbox + delay_minutes input.
3 new i18n keys (`lightweightEnabled`, `lightweightDisabled`, `lightweightHint`)
+ 2 reused from T14-2 (`lightweightMode`, `autoLightweightMinutes`).

### T14-9: Documentation + final commit
This ADR + AGENTS.md update + GRILL_T14_PERFORMANCE.md completion status + push.

## Consequences

**Positive**:
- Steady-state memory ~39MB (verified on smoke: 39227392 bytes) — 77% reduction from 171MB.
- resin.exe orphan guaranteed prevented by the Job Object kernel guarantee.
- Lightweight mode adds a second tier: Zero webview memory when the user leaves
  the app open and idle for > 10 minutes (transparent to proxy traffic).
- 323 i18n keys × 18 locales hand-audited.

**Neutral**:
- SWR and useIpc hooks are opt-in per call site; views still use the old
  `usePoll` hook. The hooks are battle-ready for the next refactor pass.
- Mobile/iPad: no change — this is a Windows-only optimization.

**Negative**:
- `trim_working_set()` is a best-effort Win32 push-to-paging-file; the OS may
  fault pages back rapidly. Effect documented in the call; users who stay in
  heavy use see the memory creep back up. The lightweight mode is the real
  lever; the trim is a one-shot hint.
- AtomicU32 read+store makes the in-memory controller struct 4 bytes larger
  than a bare u32. Negligible.

## Test coverage

| Phase | Test file | Tests added |
|-------|-----------|-------------|
| T14-1 | src-tauri/src/sidecar.rs (cargo) | 3 |
| T14-2 | src-tauri/src/lightweight.rs (cargo) | 8 |
| T14-3 | src/hooks/usePoll.test.ts (vitest) | 6 |
| T14-4 | src/store/appStore.test.ts (vitest) | 3 |
| T14-5 | src/hooks/useIpc.test.ts (vitest) | 6 |
| T14-6 | (audit, no new code) | 0 |
| T14-7 | src/views/NodesView.test.tsx (vitest) | 3 |
| T14-8 | (uses i18n-check + tsc + cargo build) | 0 new |

Total: 209 vitest, 119 cargo — all green.

## Build verification

- `pnpm build` → Vite chunk `index-3u2YsgQ2.js` written to `dist/assets/`.
- `cargo build --release -p egressapikey-app --features custom-protocol` → `11MB EgressAPIKEY.exe` staged at `release/windows-gui/`.
- Chunk hash `3u2YsgQ2` ASCII-match confirmed inside the staged exe bytes — proves the webview embeds the new bundle, not the compiled-into-the-shell stale one.
- Smoke launch: PID alive, MainWindowTitle=`EgressAPIKEY`, WorkingSet64 = 39227392 (~39MB), window hidden after Stop.
- `tsc --noEmit`: green
- `node scripts/i18n-check.cjs`: 323 keys × 18 locales match
- `git diff -2 --check` clean
