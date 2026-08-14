# ADR-0034: Canvas V2 — dagre + strategy-driven layout + zustand cache

Status: ACCEPTED
Date: 2026-08-14

## Context

T9 (commit c45334c) introduced subscription-folded C column + strategy-labeled edges
+ dual A+B badges + full-area Handle. User testing surfaced 4 bugs:
1. C column displays all nodes flat; no strategy-driven auto-layout
2. Canvas laggy + edge anchors illogical (hardcoded positions)
3. Empty-state messages flash on entry before sync() resolves
4. No caching; every view entry triggers 4 IPC calls + full re-render

## Decision

1. C column: strategy-driven filter — only show nodes targeted by at least one
   platform's region_filters or manual selection. Unselected nodes collapse as
   "+N unbound". (Kiali Service Graph pattern)
2. Layout: dagre auto-layout (official @xyflow/react recommendation) replaces
   hardcoded x:0/300/640 positions. rankdir=LR, ranksep=80, nodesep=40.
3. Handle: revert from full-area overlay to fixed edge midpoint (right/left center),
   connectionRadius=40 for snap zone. Full-area Handle was eating mousedown.
4. Flash fix: move 3 empty-state messages into the opacity-0/100 gate div;
   setReady(true) after sync() resolves, not just onInit.
5. Cache: new topologyStore (zustand create + shallow compare) holds
   platforms/subGroups/leases/ports. sync() writes via store setState;
   components subscribe via useStore(selector, shallow). 5s poll restored
   — shallow skip prevents re-render when data unchanged.

## Consequences

- New dep: dagre + @types/dagre (~60KB)
- T9's full-area Handle (ADR-0025 Q2) superseded by fixed-position Handle
- TopologyView refactored from local useState to zustand store selectors
- 5s poll + visibilitychange re-added (removed in T9 rewrite, now safe with shallow)
