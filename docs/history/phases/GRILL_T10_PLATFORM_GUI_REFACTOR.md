# GRILL T10 — Platform GUI Refactor (2026-08-14)

> Branch: codex/rust-port (parallel fix branch)
> Date: 2026-08-14
> Status: PLANNED (all Q1-Q6 decisions confirmed)
> Budget: 100,000,000 tokens
> Skills: ponytail:full, grill-with-docs, domain-modeling
> Supersedes: ADR-0029 T8 partial implementation (chip selectors never landed)

## Q&A Decision Summary

| Q | Topic | Decision | ADR |
|---|---|---|---|
| Q1 | A/B strategy layout on platform card | A — Left/right split card: A-class and B-class each occupy 50% width of lower half, both use ToggleGroup chip | 0031 |
| Q2 | A-class chip-based multi-select | A — Full chip implementation: manual=nodeList checkbox chips, region=distinct regions toggle chips, subscription=subscriptionList toggle chips, quality=top_n input + live Top-3 preview | 0031 |
| Q3 | Port auth toggle persistence | A — Persist to settings.json via loadPortAuthDefault/savePortAuthDefault, same level as splitRatio | 0031 |
| Q4 | Global scrollbar styling | A — Pure CSS scrollbar-width:thin + ::-webkit-scrollbar (6px, semi-transparent, overlay), in src/styles.css, zero deps | 0031 |
| Q5 | Remove B-class text summary from card top | A — Delete L434 summary line, move leases/nodes counts to right-top pill, B-class name only appears in lower chip UI | 0031 |
| Q6 | Smart port number suggestion | A — New IPC port_suggest (from 17990 increment + skip used + TcpListener::bind probe), frontend auto-fills on mount | 0031 |

## Root cause analysis (code-level)

### Current state (PlatformsView.tsx 539 lines)

| Area | Current Code | Problem |
|------|-------------|---------|
| A/B layout | B-class `<select>` right-top, A-class panel below `mt-2.5 border-t pt-2` | Not side-by-side, visual asymmetry |
| A-class manual | `<option value="manual">` only, no input UI | Zero selection capability — selecting manual shows nothing |
| A-class region | Text input + "+" button | User must hand-type region codes |
| A-class subscription | Text input + "+" button | User must hand-type subscription names |
| A-class quality | `top_n` number input only | No live preview of which nodes are Top-3 |
| Port auth toggle | `useState(true)`, no persistence | Forgets user's choice every page mount |
| Scrollbar | No CSS in styles.css | Platform default 17px scrollbars, ugly |
| B-class summary | `t(strategyToI18nKey(p.allocationPolicy)) + " · " + leases + nodes` in L434 text | Redundant with lower chip UI after Q1=A split |
| Port number default | `useState("17990")` hardcoded | Never changes, no system-level occupancy check |

### IPC availability (already exist, not yet imported in PlatformsView)

| IPC | Location | Returns |
|-----|----------|---------|
| `ipcNodeList()` | `src/lib/ipc.ts:141` | `Promise<unknown>` (raw JSON: node_hash, display_tag, region, tags) |
| `ipcSubscriptionList()` | `src/lib/ipc.ts:237` | `SubscriptionSnapshotEntry[]` (name, node_count) |
| `loadPortAuthDefault/savePortAuthDefault` | NOT YET — new in settings.ts | boolean persist via tauri-plugin-store |
| `port_suggest` | NOT YET — new IPC in commands/mod.rs | u16 first available port |

## T10 Execution Plan

| Step | Task | Files | ADR | Test |
|------|------|-------|-----|------|
| T10-1 | A/B left-right split card layout: platform card lower half divided 50/50, left=A-class ToggleGroup chip, right=B-class ToggleGroup single-select chip, both styled identically. Delete old `<select>` B-class and old A-class text panel. Remove L434 summary text line, add leases/nodes pill to card right-top. | src/views/PlatformsView.tsx | 0031 | 2 vitest (renders split layout, B-class chip changes allocationPolicy) |
| T10-2 | A-class chip-based multi-select: (a) manual → import ipcNodeList, render each node as checkbox chip (blue=selected, click=remove), selected stored in strategyConfig.platforms[].manual_nodes (b) region → from ipcNodeList extract distinct regions, toggle chips (click highlight/cancel) (c) subscription → from ipcSubscriptionList render toggle chips (d) quality → top_n input + live preview "Top-3: HK(100ms), JP(150ms), US(200ms)" sorted from nodeList | src/views/PlatformsView.tsx, src/lib/ipc.ts (add NodeEntry TS interface) | 0031 | 4 vitest (one per a_class type chip render + selection round trip) |
| T10-3 | Port auth toggle persistence: new loadPortAuthDefault/savePortAuthDefault in settings.ts, newAuthRequired initialized from load, saved on toggle change, useEffect on mount loads, onChange saves | src/lib/settings.ts, src/views/PlatformsView.tsx | 0031 | 1 vitest (persist + restore) |
| T10-4 | Global thin scrollbar CSS: 6 lines in src/styles.css — `scrollbar-width:thin` + `scrollbar-color` + `::-webkit-scrollbar{width:6px}` + thumb + dark variant | src/styles.css | 0031 | 0 (CSS-only, visual) |
| T10-5 | Remove B-class text summary from card top: delete L434 line, move leases.length + routableNodeCount to right-top pill badge alongside delete button | src/views/PlatformsView.tsx | 0031 | 1 vitest (no B-class name in summary, counts in pill) |
| T10-6 | Smart port suggestion: new #[tauri::command] port_suggest(db, used_ports) in mod.rs — from 17990 increment + skip port_mappings used + TcpListener::bind probe, return first available. New ipcPortSuggest in ipc.ts. Frontend useEffect on mount calls ipcPortSuggest and fills newPort. | src-tauri/src/commands/mod.rs, src/lib/ipc.ts, src/views/PlatformsView.tsx | 0031 | 1 cargo (port_suggest skips used + returns free) + 1 vitest (auto-fills on mount) |
| T10-7 | i18n: new keys for strategy chips across all 18 locales. Keys: strategy.aClassTitle, strategy.bClassTitle, strategy.manualSelect, strategy.regionSelect, strategy.subscriptionSelect, strategy.qualityPreview, platform.leasesPill, platform.nodesPill, platform.portSuggested | src/locales/*/common.json (18 files) | 0031 | i18n:check green |
| T10-8 | Build + smoke: pnpm build + cargo build --release, stage exe, grep chunk hash, smoke launch | scripts/build-all.sh | 0031 | hard close-loop per AGENTS §5 |

## Execution dependencies

```
T10-4 (CSS scrollbar) — parallel, no dependencies
T10-3 (auth persist) — parallel, no dependencies
T10-6 (port_suggest) — parallel (Rust IPC + TS wrapper)
T10-1 (split card layout) — must complete before T10-2
T10-2 (chip multi-select) — depends on T10-1 layout + ipcNodeList
T10-5 (remove summary) — depends on T10-1 layout
T10-7 (i18n) — after all UI steps (T10-1, T10-2, T10-3, T10-5, T10-6)
T10-8 (build+smoke) — last, after all steps
```

## Research references

- shadcn ToggleGroup: https://ui.shadcn.com/docs/components/toggle-group — chip-based multi-select industry standard
- Radix ToggleGroup: https://www.radix-ui.com/primitives/docs/components/toggle-group — supports single/multiple selection, keyboard nav
- shadcn Badge: https://ui.shadcn.com/docs/components/badge — removable chip variant
- MDN scrollbar-width: https://developer.mozilla.org/en-US/docs/Web/CSS/scrollbar-width — Baseline 2024, `thin` keyword
- clash-verge-rev ProxyGroup pattern: chip-based region/node selection (not text input)
- env-manager scrollbar: thin overlay (visual reference)

## Verification criteria (per step)

| Step | Verify |
|------|--------|
| T10-1 | tsc green + vitest renders A/B side-by-side + B chip click fires ipcPlatformUpdate |
| T10-2 | tsc green + 4 vitest (manual chip from nodeList, region toggle, subscription toggle, quality preview) |
| T10-3 | tsc green + vitest auth toggle persists across remount |
| T10-4 | tsc green (CSS-only, no test needed) |
| T10-5 | tsc green + vitest no B-class name in summary text |
| T10-6 | cargo test green + tsc green + vitest newPort auto-filled on mount |
| T10-7 | i18n:check = previous key count + 9, all 18 locales match |
| T10-8 | pnpm build green + cargo build --release green + chunk hash grep in exe + smoke launchMainWindowTitle=EgressAPIKEY |
