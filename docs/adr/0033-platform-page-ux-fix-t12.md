# ADR-0033: Platform Page UX Fix (T12 Fix Branch)

**Date**: 2026-08-14
Status: ACCEPTED
**Supersedes**: ADR-0032 partially (adds context-menu disable, hover expand, subscription-folded manual list, label compact)

## Context

T11 (ADR-0032) shipped collapsible cards, chip selectors, per-platform sync, port compact form, bootstrap gate. User testing surfaced 5 remaining issues: port selected gray conflict, WebView2 context menu, no hover expand on port cards, ugly "Selected (N)" in Manual mode, label input too wide squeezing out auth/add controls.

## Decision

1. **Q1 — No fix**: drag opacity-50 is transient (pointerdown to pointerup); selected ring-2 ring-primary ring-offset-1 is persistent after click. No real conflict.
2. **Q2 — CSS + config double guard**: App root <div> onContextMenu={(e) => e.preventDefault()} + body CSS user-select:none (exempt input/textarea/select) + tauri.conf.json if Tauri 2 supports it.
3. **Q3 — hoveredPort state**: onMouseEnter/onMouseLeave drive a transient hoveredPort state. isExpanded = expandedPortCards.has OR hoveredPort. Manual chevron expand persists in expandedPortCards.
4. **Q4 — Reuse NodesView groupBySub**: add tags field to NodeEntry, inline subName + groupBySub logic, replace flat Selected(N) + unselected list with subscription-folded collapsible groups.
5. **Q5 — min-w-0 max-w-[200px] flex-1**: label input has elastic width with upper bound so auth checkbox and add button survive narrow middle pane.

## Consequences

- **Positive**: WebView2 native context menu fully suppressed; hover expand improves discoverability; subscription-folded manual list aligns with NodesView mental model; label compact prevents layout overlap
- **Negative**: hoveredPort state adds one more useState; groupBySub logic duplicated inline (NodesView already has it, but import across views would be over-engineering)
- **Risk**: hover expand + pointer-drag drag start on same <li> — onMouseEnter fires before onPointerDown, so hover expand happens first, then drag starts; should not conflict

## Status: ACCEPTED
