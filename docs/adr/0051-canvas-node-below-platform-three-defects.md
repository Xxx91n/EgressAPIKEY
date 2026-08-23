# ADR-0051: Canvas node-below-platform bug — three real-runtime defects (diagnosing-bugs)

> Status: ACCEPTED + APPLIED + VERIFIED 2026-08-23
> Supersedes: none (extends ADR-0048 region viewMode edge logic; ADR-0050 manual-edge fix was unrelated to this bug — it corrected a separate doc/code drift)
> Builds on: ADR-0002 (three-column canvas), ADR-0048 (a_class-semantic filtering)
> Method: diagnosing-bugs skill (Phase 1 feedback loop -> Phase 2 real-data minimal repro -> Phase 3 root cause -> Phase 5 fix + regression test)

## Context

User report (reproduced against the staged release exe): selecting a subscription or region in the bottom-left always shows the resulting nodes BELOW the middle platform card, not to its RIGHT — in both subscription and region viewMode.

ADR-0050 (manual visible group edges) did NOT fix it, because the user platform is not manual mode. A feedback loop was built: the live sidecar was booted on a copy of the real state/cache DBs with a known admin token, and /api/v1/nodes + /api/v1/platforms + strategyConfig + settings.json were inspected directly (no code-gazing).

## Root cause (three defects, all required to reproduce both viewModes)

### Defect A — subscription name parsing mismatched the Resin API (primary cause)

Resin /api/v1/nodes returns node tags as snake_case: `{ subscription_id, subscription_name, tag }`. `parseSubscriptionGroups` looked up ONLY `t.subscriptionName` (camelCase), which never matched, so it fell back to `tags[0].tag` (the per-node tag string, e.g. "1/node-01"). The group was therefore named by the per-node tag, and strategyConfig `subscriptions: ["1"]` never matched any group name -> zero B->C edges -> the C group was disconnected and dagre stacked it at rank 0 (below/left of the platform).

Evidence (live): FIRST NODE tag = `{"subscription_id":"38c31eff-...","subscription_name":"1","tag":"1/node-01"}`; the only SUB NAMES the frontend produced was the tag string, never "1". state.db confirms the subscription with name "1" exists, so the strategyConfig value is correct; the parser was the break.

### Defect B — port-less platform amplified the stack (layout guard missing)

Resin had a second platform "2" with no entry ports bound to it. T22-4 auto-defaulted it to `subscription` + empty subscriptions (subscription:all). With no A->B edge, platform-2 sat at rank 0 (entry column), and its subscription:all edges dragged every C group into rank 1 — the same column as the WITH-ports Default platform — so visually the C groups appeared directly under the Default card. This is why the symptom was identical in both viewModes.

### Defect C — region viewMode skipped subscription-with-specific-subs

The region-viewMode edge loop only handled `subscription` aClass when `subscriptionNames` was empty (subscription:all). A platform with `subscriptions: ["1"]` (non-empty) drew NO region edges, leaving the regionGroup disconnected in region viewMode — same rank-0 stack.

## Decision

1. **Defect A fix**: `NodeItem.tags` interface accepts `subscription_name` (snake) alongside `subscriptionName`; `parseSubscriptionGroups` reads `t.subscriptionName ?? t.subscription_name`. Now the group is named "1", matching strategyConfig.
2. **Defect B fix**: `layoutNodesViaDagre` adds three invisible dagre-only column anchors (`__col0 -> __col1 -> __col2`, minlen:0 same-rank binding to entryPort/platform/C-group). Anchors are never returned as ReactFlow nodes. Even when real edges are missing (port-less platform, unmatched subscription, strategyConfig load failure), dagre now ranks C at col2 (right of platform). Pattern verified against idootop/reactflow-auto-layout #root + Kiali synthetic edges (atomcode research).
3. **Defect C fix**: the region-viewMode edge loop is extracted to an exported pure helper `buildRegionViewEdges`; its `subscription` branch now also handles non-empty subscriptions (edges to the regions of the named subscriptions), mirroring `buildEdges` subscription semantics.

## Rejected alternatives

- Switch layout engine to elkjs layerConstraint / Grafana grid. Rejected: heavier, new dependency; the snake-case fix + column anchors + region branch together are sufficient and domain-correct.
- Hide port-less platforms from the canvas. Rejected: loses real topology info; the column anchor is the correct structural fix and also covers strategyConfig-load-failure and unmatched-subscription cases.

## Consequences

- Canvas three-column contract (ADR-0002) now holds structurally: entry | platform | C-group, for ALL aClass/viewMode/edge combinations, including degenerate (no edges, port-less platforms, strategyConfig load failure).
- `buildRegionViewEdges` is exported and unit-tested (was inline, untestable).
- `layoutNodesViaDagre` no longer depends on edges existing to place columns — anchors guarantee column rank.

## Verification

- vitest: TopologyView 81/81 green (5 new ADR-0051 regression tests: snake-case render loop, buildRegionViewEdges subscription+specific-subs + subscription+empty, column-anchor no-edges case, column-anchor port-less-platform case). Full suite 305/305 green, no regressions.
- Real-data minimal repro (Phase 2): the live diagnosis data (Default subscription+["1"], platform "2" auto-default, 4 ports, sub "1") fed through the fixed logic -> C.x=723 > platform.x=442 in BOTH subscription and region viewMode (was below before).

## Implementation

`src/views/TopologyView.tsx`: F1 NodeItem snake tags + F2 parseSubscriptionGroups snake lookup + F3 region-viewMode edges useMemo delegates to buildRegionViewEdges + F4 buildRegionViewEdges exported pure helper (with subscription+specific-subs branch) + F5 layoutNodesViaDagre column anchors.
`src/views/TopologyView.test.tsx`: import buildRegionViewEdges; 3 new describes (Defect A render loop, Defect C pure-helper, Defect B column anchors).
