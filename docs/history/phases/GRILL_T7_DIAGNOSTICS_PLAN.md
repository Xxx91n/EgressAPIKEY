# GRILL T7: Diagnostics Page Refactor — Grill Decisions & Execution Plan

Date: 2026-08-13
Status: GRILL COMPLETE — awaiting user "start building" instruction
Grill round: T7 (Q1-Q7)

## Problem Statement

Three bugs in the current Settings > Diagnostics panel:

1. **ps1 console window flash**: `check_firewall_status` IPC uses `std::process::Command::new("powershell")` without `CREATE_NO_WINDOW` flag (0x08000000), causing a PowerShell console window to flash on every call.
2. **Diagnostics panel cramped in Settings**: The diagnostic SectionCard in SettingsView.tsx (L428+) crams sidecar port/mode, firewall status, request log table, and refresh button into one card — not intuitive for a diagnostics/debug surface.
3. **Subprocess deadlock + window respawn**: `Command::new("powershell").output()` is synchronous with no timeout — can permanently block if PowerShell profile loads slowly or AV intervenes. SettingsView mount auto-triggers `refreshDiagnostics()` so every visit to Settings spawns the PS window again.

## Root Cause (code-level verified via ctx_batch_execute)

- `src-tauri/src/commands/mod.rs:1939` — `Command::new("powershell")` without `creation_flags(0x08000000)`
- `src-tauri/src/commands/mod.rs:1942` — `.output()` synchronous, no timeout wrapper
- `src/views/SettingsView.tsx:145` — mount auto-calls `refreshDiagnostics()` → triggers firewall IPC
- `src/views/SettingsView.tsx:428` — diagnostics SectionCard inside SettingsView

## Grill Decisions Summary

| Q | Topic | Decision | New code? | Test shape |
|---|-------|----------|-----------|------------|
| Q1 | Fix scope | A: unified fix in one commit | Yes | per-phase |
| Q2 | DiagnosticsView content | B: full diagnostics hub, sole entry point, no redundancy | ~200 TSX | vitest closed-loop |
| Q3 | Request log refresh | A: configurable polling (default 5s) + manual refresh + visibilitychange pause | ~30 TSX | vitest interval test |
| Q4 | Poll interval storage | A: `settings.json#diagPollInterval` via tauri-plugin-store | ~20 TS | vitest load/save |
| Q5 | Cross-platform firewall | A: Windows实测 + Linux(systemctl/firewalld/ufw) + macOS(pfconf/pfctl) — all with 5s timeout + CREATE_NO_WINDOW | ~80 Rust | cargo timeout test |
| Q6 | Nav icon/key | A: `diagnostics` key + `Stethoscope` lucide icon, position 6 (before settings) | ~5 TSX | vitest nav render |
| Q7 | SettingsView cleanup | A: delete diagnostics SectionCard, keep About port/mode overview, dynamically handle dependencies | ~-100 TSX | vitest no-render test |

## pwm pro Research Results

### Subprocess no-window + timeout (gpt56_sol)
Template: `tokio::process::Command` + `creation_flags(0x08000000)` on Windows + `tokio::time::timeout(Duration::from_secs(5), child.wait_with_output())`. Cross-platform: Linux/macOS use `sh -c`. On timeout: `child.kill().await` + return error.

### Non-root firewall detection (sonar)
- **Windows**: `Get-NetFirewallProfile` works as non-admin
- **Linux**: `ufw status` and `iptables -L` require root. `systemctl is-active ufw` / `systemctl is-active firewalld` may work as non-root (distro-dependent). Reading `/proc/net/ip_tables_names` may work but is not definitive.
- **macOS**: `pfctl -s info` requires root. Checking `/etc/pf.conf` existence is coarse but non-root safe.
- All non-Windows: best-effort, return descriptive error if permission denied.

## Execution Plan (dependency graph)

### T7-1: Fix check_firewall_status (Rust backend)
- Replace `std::process::Command` with `tokio::process::Command`
- Add `creation_flags(0x08000000)` on Windows (CREATE_NO_WINDOW)
- Add `tokio::time::timeout(5s)` wrapper
- Linux: try `systemctl is-active ufw` then `systemctl is-active firewalld` then read `/proc/net/ip_tables_names` then fallback
- macOS: try `pfctl -s info` then check `/etc/pf.conf` existence then fallback
- On timeout: kill child, return `{firewall_on: false, detail: "Detection timed out"}`
- **Acceptance**: 1 cargo test (timeout behavior with mock slow command)
- **Estimate**: ~80 lines Rust

### T7-2: Create DiagnosticsView.tsx (frontend)
- New file: `src/views/DiagnosticsView.tsx`
- Sections:
  - Sidecar status card: port, mode, PID, healthz last check, IPC latency (from `ipcGetSidecarStatus`)
  - Firewall status card (from `ipcCheckFirewallStatus`)
  - Request log table card (from `ipcRequestLogTail`) with auto-refresh polling
  - Exit IP probe card (from `ipcProbeExitIp`) — port selector + protocol selector + probe button
  - Port health check card (from `ipcPortHealthCheck`) — port input + protocol + check button
  - Log directory button (from `get_log_dir` then `openPath`)
  - Sidecar log buffer card (from `ipcGetSidecarLogs`) — scrollable ring buffer display
- Polling: `setInterval` with configurable interval (default 5000ms), `visibilitychange` pause/resume
- Manual refresh button alongside auto-polling
- **Acceptance**: 3 vitest (renders all cards, poll interval from settings, manual refresh triggers IPC)
- **Estimate**: ~200 lines TSX

### T7-3: Add nav tab in App.tsx
- Add `Stethoscope` to lucide-react imports
- Add `{ key: "diagnostics", icon: Stethoscope }` to NAV_ITEMS at position 6 (before settings)
- Add `{view === "diagnostics" && <DiagnosticsView />}` to main content
- Add i18n key `nav.diagnostics` across all 18 locales
- **Acceptance**: 1 vitest (nav renders diagnostics tab)
- **Estimate**: ~10 lines TSX

### T7-4: SettingsView cleanup
- Delete from SettingsView.tsx:
  - Diagnostics SectionCard (L428+)
  - `firewallStatus` state + `setFirewallStatus`
  - `reqLogs` state + `setReqLogs`
  - `diagBusy` state + `setDiagBusy`
  - `refreshDiagnostics` function
  - `void refreshDiagnostics()` call in mount effect
  - Imports: `ipcCheckFirewallStatus`, `ipcRequestLogTail`, `FirewallStatus`, `RequestLogEntry`
- Keep in SettingsView.tsx:
  - `sidecarStatus` state + `ipcGetSidecarStatus` (for About section port/mode display)
  - IPC commands stay registered in main.rs (DiagnosticsView calls them)
- Dynamic dependency check: if any test references removed state, adjust the test
- **Acceptance**: 2 vitest (diagnostics card NOT rendered in SettingsView, About port/mode still rendered)
- **Estimate**: ~-100 lines TSX

### T7-5: Poll interval config (settings.json)
- Add `loadDiagPollInterval` / `saveDiagPollInterval` to `src/lib/settings.ts`
- Read on DiagnosticsView mount, save on change
- Default 5000ms, range 1000-60000, clamp to range
- UI: number input or dropdown (1s/2s/5s/10s/30s/60s) in DiagnosticsView header
- **Acceptance**: 1 vitest (load + save round-trip)
- **Estimate**: ~30 lines TS

### T7-6: i18n keys for DiagnosticsView
- New keys across all 18 locales:
  - `nav.diagnostics`
  - `diagnostics.title`
  - `diagnostics.sidecarStatus`
  - `diagnostics.firewall`
  - `diagnostics.reqLog`
  - `diagnostics.exitIpProbe`
  - `diagnostics.portHealth`
  - `diagnostics.logs`
  - `diagnostics.pollInterval`
  - `diagnostics.openLogDir`
  - `diagnostics.sidecarLogs`
  - `diagnostics.refresh`
  - `diagnostics.noLogs`
- **Acceptance**: `pnpm i18n:check` green
- **Estimate**: ~13 keys x 18 locales

### T7-7: Closed-loop tests
- Update existing `SettingsView.test.tsx`: remove diagnostics panel tests (moved to DiagnosticsView)
- New `DiagnosticsView.test.tsx`:
  - Renders sidecar status card with port + mode + PID + healthz + latency
  - Renders firewall status card
  - Renders request log table with entries from IPC
  - Poll interval loaded from settings
  - Manual refresh button triggers IPC
  - Exit IP probe button works
  - Port health check button works
- `ipc.test.ts`: existing firewall/requestLog/sidecarStatus/probe/healthCheck wrappers already tested
- **Acceptance**: all tests green
- **Estimate**: ~6 vitest

### T7-8: Build + smoke + codegraph sync + push
- `pnpm build` (tsc + vite)
- `cargo build --release -p egressapikey-app --features custom-protocol`
- Stage exe to `release/windows-gui/EgressAPIKEY.exe`
- Grep new Vite chunk hash inside exe bytes
- Smoke launch: MainWindowTitle=EgressAPIKEY, no ps1 window flash
- `codegraph sync .`
- `git push`
- **Acceptance**: build green, chunk hash confirmed, no console flash, tests green

## Dependency Graph

T7-1 (Rust fix) and T7-2 (DiagnosticsView) can start in parallel.
T7-3 depends on T7-2.
T7-4 depends on T7-2.
T7-5 depends on T7-2.
T7-6 depends on T7-2.
T7-7 depends on T7-2, T7-3, T7-4, T7-5, T7-6.
T7-8 depends on all.

## Estimates

| Phase | Lines | New files | Tests |
|-------|-------|-----------|-------|
| T7-1 | ~80 Rust | 0 | 1 cargo |
| T7-2 | ~200 TSX | 1 (DiagnosticsView.tsx) | 0 (tests in T7-7) |
| T7-3 | ~10 TSX | 0 | 0 |
| T7-4 | ~-100 TSX | 0 | 0 |
| T7-5 | ~30 TS | 0 | 0 |
| T7-6 | ~234 i18n entries | 0 | 0 |
| T7-7 | ~120 TSX | 1 (DiagnosticsView.test.tsx) | 6 vitest + test updates |
| T7-8 | 0 | 0 | 0 |
| **Total** | ~250 net new | 2 | 7+ |

## ADR Consideration

ADR-0029: Diagnostics as a First-Class View (separation from Settings)
- Hard to reverse: yes (once diagnostics is its own nav tab, going back to Settings is a regression)
- Surprising without context: no (standard desktop app pattern)
- Real trade-off: yes (took the decision to split vs keep inline)
- Create ADR-0029

## Handoff Prompt

After user says "start building", execute the T7-1 through T7-8 phases in dependency order.