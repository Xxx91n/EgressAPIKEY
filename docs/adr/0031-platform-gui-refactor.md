# ADR-0031: Platform GUI Refactor — Split Card, Chip Selectors, Smart Port, Thin Scrollbar

Status: ACCEPTED (2026-08-14)

## Context
ADR-0029 (T8) planned chip-based multi-select for A-class strategy selectors but the implementation left text inputs in place. The platform card A/B strategy areas are visually asymmetric (B-class `<select>` top-right, A-class text panel below). Manual A-class mode has zero selection UI. Port auth toggle forgets user preference. Scrollbars are platform default (17px, ugly). Port number defaults to hardcoded 17990 with no system occupancy check.

User requested (2026-08-14): A/B strategy side-by-side display, industry-grade chip selectors for all A-class modes (clash-verge-rev pattern), auth toggle memory at settings.json level, thin transparent scrollbar like Codex app / env-manager, remove B-class name from card text summary, smart port number suggestion.

## Decision

1. **A/B split card (Q1=A)**: Platform card lower half divided 50/50. Left = A-class ToggleGroup chip (multi-select). Right = B-class ToggleGroup single-select chip (random/sequential/latency/quality). Both styled identically. Delete old `<select>` B-class and old A-class text panel.

2. **Chip-based A-class multi-select (Q2=A)**: manual=checkbox chips from ipcNodeList, region=toggle chips from distinct regions in nodeList, subscription=toggle chips from ipcSubscriptionList, quality=top_n input + live Top-3 preview sorted by node latency. Replace all 4 text-input selectors.

3. **Port auth persistence (Q3=A)**: New loadPortAuthDefault/savePortAuthDefault in settings.ts using tauri-plugin-store settings.json#portAuthDefault. newAuthRequired initialized from load on mount, saved on toggle change. Same persistence level as splitRatio.

4. **Thin scrollbar CSS (Q4=A)**: 6 lines in src/styles.css: `scrollbar-width:thin` + `scrollbar-color` + `::-webkit-scrollbar{width:6px,height:6px}` + thumb + dark variant. Zero new deps. Covers Firefox (scrollbar-width) + WebView2/Chromium (::-webkit-scrollbar).

5. **Remove B-class text summary (Q5=A)**: Delete L434 summary line (strategyToI18nKey(p.allocationPolicy) + leases + routableNodes). Move leases.length + routableNodeCount to right-top pill badge alongside delete button. B-class name only appears in lower chip UI.

6. **Smart port suggestion (Q6=A)**: New #[tauri::command] port_suggest(db) in mod.rs — read port_mappings used ports, from 17990 increment + skip + TcpListener::bind probe, return first available u16. New ipcPortSuggest in ipc.ts. Frontend useEffect on mount auto-fills newPort.

## Consequences

- Platform card grows taller (A/B split uses more vertical space) but information is cleaner and more discoverable.
- ipcNodeList return type needs a TS interface (NodeEntry) — currently `Promise<unknown>`. Add interface in ipc.ts, no Rust change.
- settings.ts gains 2 new functions (loadPortAuthDefault/savePortAuthDefault) following existing loadSplitRatio/saveSplitRatio pattern.
- styles.css gains 6 lines of global CSS — affects all scrollable areas in the app (App.tsx main, PlatformsView lists, NodesView, DiagnosticsView).
- port_suggest IPC adds 1 command to mod.rs (55 → 56 commands). Needs validator guard.
- i18n: 9 new keys across all 18 locales.
- Supersedes ADR-0029 T8-6 (chip selectors) which was planned but never implemented.

## Ponytail assessment

- No new dependencies (ToggleGroup is shadcn/Radix pattern re-implemented with Tailwind classes, matching existing badge chip pattern already in PlatformsView).
- Reuses existing ipcNodeList + ipcSubscriptionList IPC (already shipped, just not imported in this view).
- Reuses existing loadSplitRatio/saveSplitRatio patterns for new settings persistence.
- Reuses existing sidecar.rs pick_free_loopback_port logic concept for port_suggest.
- Total estimated: ~200 lines TS refactor + ~25 lines Rust + ~6 lines CSS + i18n.
