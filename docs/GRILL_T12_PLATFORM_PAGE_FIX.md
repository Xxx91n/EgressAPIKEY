# GRILL T12 — Platform Page UX Fix (Fix Branch)

**Date**: 2026-08-14
**Status**: PLANNING COMPLETE, AWAITING EXECUTION
**Prerequisite**: T11 complete (ADR-0032), PlatformsView 660 lines

## Q1-Q5 Decision Summary

| Q | Topic | Decision | Key Change |
|---|-------|----------|-------------|
| Q1 | Port selected gray opacity | C — No fix | Drag opacity-50 is transient; selected ring-primary is persistent. No real conflict. |
| Q2 | Disable WebView2 right-click context menu | C — CSS + config double guard | App root oncontextmenu=preventDefault + global CSS user-select:none on body + tauri.conf.json if available. |
| Q3 | Port card hover auto-expand | A — hoveredPort state | onMouseEnter sets hoveredPort, onMouseLeave clears. Expand = expandedPortCards.has OR hoveredPort. Manual expand persists; hover expand is transient. |
| Q4 | Manual node Selected(N) refactor | OK — Reuse NodesView pattern | Add tags field to NodeEntry. Reuse subName + groupBySub from NodesView. Manual mode shows nodes folded by subscription source. Delete Selected(N) section. Selected nodes highlighted blue in-place within their subscription group. |
| Q5 | Port form label input compact | C — min-w-0 max-w-200px | Change label input from flex-1 to min-w-0 max-w-[200px] flex-1. Ensures auth checkbox + add button always visible when middle pane is narrow. |

## Execution Plan (5 Steps)

| Step | Task | Files | Test | Deps |
|------|------|-------|------|------|
| T12-1 | Disable WebView2 context menu (CSS + config) | App.tsx, index.css, tauri.conf.json | 1 vitest | none |
| T12-2 | Port card hover auto-expand | PlatformsView.tsx | 2 vitest | none |
| T12-3 | Manual mode: subscription-folded node list | PlatformsView.tsx (NodeEntry tags) | 2 vitest | none |
| T12-4 | Port form label input compact | PlatformsView.tsx | 1 vitest | none |
| T12-5 | Build + smoke + chunk hash | build-all.ps1 | hard close-loop | all |

Dependencies: T12-1/2/3/4 can all parallel. T12-5 last.

## Detailed Specs

### T12-1: Disable WebView2 context menu
- App.tsx: root <div> gets onContextMenu={(e) => e.preventDefault()}
- src/index.css or tailwind: body { -webkit-user-select: none; user-select: none; } but exempt input/textarea
- tauri.conf.json: check if Tauri 2 has windows[].disableContextMenu; if not, CSS+JS is sufficient
- 1 vitest: verify App root div has onContextMenu handler

### T12-2: Port card hover auto-expand
- New state: const [hoveredPort, setHoveredPort] = useState<number | null>(null)
- onMouseEnter={() => setHoveredPort(p.port)} on <li>
- onMouseLeave={() => setHoveredPort(null)} on <li>
- Expand condition: const isExpanded = expandedPortCards.has(p.port) || hoveredPort === p.port
- Manual expand (chevron click) adds to expandedPortCards (persistent); hover is transient
- 2 vitest: (1) hover expands port card, (2) manual expand persists after mouse leave

### T12-3: Manual mode subscription-folded node list
- PlatformView NodeEntry interface: add tags?: { subscription_name?: string; subscriptionName?: string; tag: string }[]
- nodeList mapping (L218): add tags: n.tags ?? []
- Inline subName + groupBySub logic from NodesView (or import)
- Manual mode replaces flat Selected(N) + flat unselected list with:
  - Group nodes by subName(n) || "untagged"
  - Each group: collapsible header (subscription name + count + chevron)
  - Default collapsed, click to expand
  - Within expanded group: node chips with blue bg if selected, normal if not
  - Search input still filters across all groups
- 2 vitest: (1) manual mode shows subscription group headers, (2) selected nodes highlighted in their group

### T12-4: Port form label compact
- Change L395 from: <input className="flex-1 rounded border ..."
  to: <input className="min-w-0 max-w-[200px] flex-1 rounded border ..."
- 1 vitest: verify label input has max-w-[200px] class

### T12-5: Build + smoke + chunk hash
- pnpm build -> note chunk hash
- cargo build --release -p egressapikey-app --features custom-protocol
- Stage exe to release/windows-gui/EgressAPIKEY.exe
- Grep chunk hash in exe bytes
- Smoke: launch, verify MainWindowTitle=EgressAPIKEY, WS 30-40MB

## Root Cause Analysis

### Bug 1 (port selected gray)
Not a bug. opacity-50 is only during drag (pointerdown to pointerup). selected ring-primary is after click. No real overlap. No fix needed (Q1=C).

### Bug 2 (WebView2 context menu)
No oncontextmenu handler anywhere in the app. WebView2 shows native right-click menu. Fix: global preventDefault + CSS user-select:none (Q2=C).

### Bug 3 (no hover expand)
expandedPortCards only toggled by chevron click. No hover behavior. Fix: add hoveredPort transient state (Q3=A).

### Bug 4 (Manual Selected(N) ugly)
PlatformsView NodeEntry missing tags field. NodesView already has groupBySub pattern. Fix: add tags, reuse grouping, delete Selected section (Q4=OK).

### Bug 5 (label input too wide)
L395 flex-1 with no max-w. Narrow middle pane squeezes out auth checkbox + add button. Fix: min-w-0 max-w-[200px] flex-1 (Q5=C).

## Acceptance Criteria

1. Right-click anywhere in app triggers no WebView2 context menu
2. Hover port card auto-expands showing auth details; mouse away auto-collapses; chevron click persists expand
3. Manual mode shows nodes grouped by subscription source, each group collapsible, selected nodes blue-highlighted, no Selected(N) section
4. Port form label input never wider than 200px; auth checkbox and + button always visible when middle pane narrow
5. pnpm test all pass; npx tsc --noEmit clean; cargo test -p resin-core --lib pass; i18n:check pass
6. Release exe embeds fresh Vite chunk hash; smoke green
