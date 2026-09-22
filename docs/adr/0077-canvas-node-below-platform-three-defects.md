# ADR-0077: Canvas node-below-platform bug — three real-runtime defects

Date: 2026-09-22
Status: Accepted
Supersedes: none (extends ADR-0048 region-viewMode edge logic; the
ADR-0076 manual-edge fix is a separate doc/code drift)
Builds on: ADR-0002 (three-column canvas), ADR-0048 (a_class-semantic
filtering)
Provenance: renumbered from branch `fix/zcode-GUI-Topology` ADR-0051
(recovered from commit `255eb7b8`; the original diagnosis was made via a
live sidecar on real state/cache DBs). Re-validated against main
2026-09-22: all three defects were live on main (camelCase-only tag
lookup, no column anchors, missing subscription+specific-subs region
branch).

## Context

Symptom: selecting a subscription or region renders the C-column group
nodes BELOW the platform card instead of to its right — in both
subscription and region viewMode.

## Root cause (three defects, all required to reproduce both viewModes)

### Defect A — subscription name parsing mismatched the Resin API

Resin `/api/v1/nodes` returns node tags as snake_case:
`{ subscription_id, subscription_name, tag }`.
`parseSubscriptionGroups` looked up ONLY `t.subscriptionName`
(camelCase), which never matched, so groups fell back to `tags[0].tag`
(the per-node tag string, e.g. "1/node-01"). strategyConfig
`subscriptions: ["1"]` then never matched any group -> zero B->C edges
-> the C group disconnected -> dagre stacked it at rank 0 (below/left).

### Defect B — port-less platform amplified the stack (layout guard missing)

A platform with no entry ports bound (auto-default `subscription` +
empty subscriptions) has no A->B edge -> sits at rank 0, and its
unconditional edges drag every C group into rank 1 — the same column as
the WITH-ports platform — so C groups appear directly under the Default
card in both viewModes.

### Defect C — region viewMode skipped subscription-with-specific-subs

The region-viewMode edge loop only handled `subscription` aClass when
`subscriptionNames` was empty; a platform with `subscriptions: ["1"]`
drew NO region edges -> regionGroup disconnected -> same rank-0 stack.

## Decision

1. **Defect A**: `NodeItem.tags` accepts `subscription_name` (snake)
   alongside `subscriptionName`; `parseSubscriptionGroups` reads
   `t.subscriptionName ?? t.subscription_name`.
2. **Defect B**: `layoutNodesViaDagre` adds three invisible dagre-only
   column anchors (`__col0 -> __col1 -> __col2`, `minlen:0` same-rank
   binding to entryPort/platform/C-group). Anchors are never returned as
   ReactFlow nodes. Even with no real edges (port-less platform,
   unmatched subscription, snapshot load failure) dagre now ranks C at
   col2. Pattern: idootop/reactflow-auto-layout `#root` anchor + Kiali
   BoxLayout synthetic edges.
3. **Defect C**: the region-viewMode edge loop is extracted to the
   exported pure helper `buildRegionViewEdges`; its `subscription`
   branch also handles non-empty subscriptions (edges to the regions of
   the named subscriptions), mirroring `buildEdges` semantics.

## Rejected alternatives

- elkjs layerConstraint / Grafana grid — heavier, new dependency.
- Hide port-less platforms — loses real topology info; the anchor is the
  structural fix and also covers load-failure/unmatched cases.

## Consequences

- The three-column contract (ADR-0002) now holds structurally for ALL
  aClass/viewMode/edge combinations, including degenerate cases.
- `buildRegionViewEdges` is exported and unit-tested (was inline).

## Upgrade path (carried from branch, atomcode re-verification)

`@dagrejs/dagre >=1.1.5` ships official manual ranking (PR #271), but
the merged feature is thin (+7/-1 lines, zero tests/docs) with known
crash modes (same-rank real edges, missing `rank` attrs). Stay on
`dagre@0.8.5` + the anchor pattern; if rank pinning is ever needed,
spike on a separate branch — do not hand-patch 0.8.5. For cascading
delete there is an official path: `useReactFlow().deleteElements`
routes through `getElementsToRemove` (respects `deletable:false`) +
`onBeforeDelete` veto — prefer that over growing custom helpers if edge
deletion semantics ever expand.
