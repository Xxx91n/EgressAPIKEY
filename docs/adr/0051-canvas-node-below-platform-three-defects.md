# ADR-0051: Canvas node-below-platform bug — three real-runtime defects (diagnosing-bugs)

> Status: ACCEPTED + APPLIED + VERIFIED 2026-08-23
> Supersedes: none (extends ADR-0048 region viewMode edge logic; ADR-0050 manual-edge fix was unrelated to this bug — it corrected a separate doc/code drift)
> Builds on: ADR-0002 (three-column canvas), ADR-0048 (a_class-semantic filtering)
> Method: diagnosing-bugs skill (Phase 1 feedback loop -> Phase 2 real-data minimal repro -> Phase 3 root cause -> Phase 5 fix + regression test)
> Atomcode re-verification (2026-08-24, high-confidence, 13 searches / 11 source fetches): see "Upgrade path" below.

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
2. **Defect B fix**: `layoutNodesViaDagre` adds three invisible dagre-only column anchors (`__col0 -> __col1 -> __col2`, minlen:0 same-rank binding to entryPort/platform/C-group). Anchors are never returned as ReactFlow nodes. Even when real edges are missing (port-less platform, unmatched subscription, strategyConfig load failure), dagre now ranks C at col2 (right of platform). Pattern verified against idootop/reactflow-auto-layout #root + Kiali BoxLayout synthetic edges (atomcode research, sources fetched).
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

## Upgrade path (atomcode re-verification 2026-08-24)

Confidence high; 13 searches across Exa/Tavily/AnySearch; 11 source fetches (9 OK). Corrections to assumptions made at decision time:

1. **dagre manual ranking is now officially supported upstream.** When this ADR was written, we said "dagre has no per-node rank API; issue #54 unresolved". That was only true for the legacy `dagre@0.8.5` line. Source-read of v0.8.5 `lib/rank/index.js`: rank is computed from edge `minlen/weight` (Gansner); a node `rank` field is pure output. **PR #271 (manual ranking: `g.graph().ranker` can be a custom function, `rank` becomes a settable node attribute) was merged 2025-06-17** and shipped as `@dagrejs/dagre` 1.1.5 the same day (latest 3.1.1; 2.0.0 fixed rank-constraint corner cases; 3.0.0 is a TS rewrite). If we ever want first-class column pinning, the supported path is `pnpm add @dagrejs/dagre@^1.1.5` (same `dagre.layout()` call site) — do NOT hand-patch 0.8.5 (PR #271 comments report "layers[node.rank] is undefined" crashes when patched manually).
2. **The anchor/synthetic-edge hack we shipped aligns with the recognized best practice for 0.8.5.** (a) invisible anchor + `minlen:0` same-rank binding = idootop/reactflow-auto-layout `#root` pattern AND Kiali `BoxLayout` synthetic-edge pattern — two independent production implementations agree this is the right move on the legacy line. (b) post-`layout()` position override is the standard companion step used by the React Flow official Dagre Tree example and erda-ui production (`react-flow-renderer` + dagrejs LR + `dagre.layout(graph, {weight:2})`, with a horizontal/vertical normalization pass). Combined `(a)+(b)` is the production shape; we currently only do `(a)` and let dagre write positions directly. If we ever see cross-column y-drift after future edge shapes, add step (b): `x = rank * columnWidth`, y from dagre.
3. **Production-attribution corrections** (so future readers do not cargo-cult wrong): Kiali — v2.8 (2025-03-27) removed Cytoscape; the new PatternFly graph offers `kiali-dagre` as one of three layouts, but Kiali itself is not a React Flow app. Cilium Hubble UI — current master package.json has NO react-flow/dagre/elkjs; it is SVG+d3 with a hand-rolled card layout. The "Kiali uses React Flow + dagre synthetic edges" lore in our CONTEXT/decision threads was about its historical Cytoscape-era BoxLayout, not its current PatternFly graph. A real current React Flow + dagre production reference is Microsoft conductor PR #153 (2026-05) — uses `findBackEdges()` DFS to pre-classify back edges and feed them to dagre reversed to repair rank.
4. **Cascading delete (edge -> clean up member list in config): there IS an official pattern; do not grow custom helpers.** `@xyflow/react` v12: `useReactFlow().deleteElements({edges:[{id}]})` routes through `getElementsToRemove` (xyflow `packages/system/src/utils/graph.ts`) — deleting a node cascades to children via `parentId` and to connected edges via `getConnectedEdges`; `deletable:false` exempts; `onBeforeDelete` (v12, xyflow issue #3722 / PR #3741) can veto or rewrite the delete set so the strategyConfig patch ships in the same transaction. Gotcha: `setEdges(eds => eds.filter(...))` does NOT fire `onEdgesDelete` (xyflow #4737) — must go through `deleteElements`. For modeling the member hash list as derived state projected from edges (so deleting an edge auto-empties the projection with no helper), see Zustand derived-state discussions pmndrs/zustand #1809 / #108 and the React Flow Zustand integration guide; a close reference implementation is muthub-ai/aac `use-graph-store.ts` (single-source YAML, canvas `updateFromCanvas` re-projects back into config, `syncSource: 'yaml'|'canvas'` mutex). erda-ui's v9 `onElementsRemove -> removeElements(elementsToRemove, els)` is the same declarative cascade (SQL `ON DELETE CASCADE` semantics).

Decision for this ADR stays: keep the 0.8.5-style anchor fix (it is the recognized best practice for the shipped dependency line) and the explicit `manualNodesAfterGroupEdgeDelete` helper (it is small, pure, and unit-tested). Record the upgrade path so future A-class additions can migrate to first-class rank + official cascade without re-deriving this research.
