# ADR-0050: Canvas manual-mode visible group edges (revises ADR-0048 S3)

> Status: ACCEPTED (decision recorded via grill; code applied + verified 2026-08-23)
> Date: 2026-08-23
> Supersedes: ADR-0048 S3 (manual aClass -> NO group-level edge)
> Builds on: ADR-0042 S4 (manual card checkbox), ADR-0048 (a_class-semantic filtering), CONTEXT.md Strategy-Labeled Edge
> Sources: atomcode research 2026-08-23 (dagre issue #54, idootop/reactflow-auto-layout #root anchor, Kiali BoxLayout synthetic-edge pattern) + atomcode re-verification 2026-08-24 (dagre PR #271 merged 2025-06-17 -> @dagrejs/dagre >=1.1.5 ships official manual ranking; xyflow v12 `deleteElements -> getElementsToRemove` cascade + `onBeforeDelete`; muthub-ai/aac YAML projection store); local dagre CASE-1 reproduction

## Context

Bug: in manual aClass (and the strategy_config_get-failure fallback to manual), the canvas C-column nodes (subscriptionGroup/regionGroup) render below-left of the platform instead of to its right. Empirically reproduced with the project dagre@0.8.5: with only an A->B edge and no B->C edge, dagre assigns the disconnected C node rank 0 (leftmost), stacking it below the entry-port column (CASE 1: a=120/70, b=400/70, c=120/210).

Root cause: dagre rank (column) is edge-derived (Sugiyama; dagre issue #54 confirms no per-node rank API). ADR-0048 S3 decreed manual = NO group-level edge because manual_nodes is node-level, not group-level. That left manual-mode C-column nodes with no incoming B->C edge -> disconnected -> rank 0 -> below-left.

Doc tension surfaced via grill-with-docs: CONTEXT.md Strategy-Labeled Edge (L259-264) explicitly lists manual as a DRAWN, DELETABLE edge ("Auto-strategy edges are non-deletable; only manual edges can be dragged/deleted"). ADR-0048 S3 no-manual-edge drifted from the domain model. The rule traced to a technical-constraint cascade, not a domain decision: ADR-0042 S4 chose card-checkbox for manual because ReactFlow group nodes have no per-leaf handles (cannot drag platform->individual node) and Resin has no per-node API. ADR-0048 S3 then over-extended "no leaf edge" into "no group edge" -- but group-no-handle only blocks leaf edges, not group edges (platform->subscriptionGroup has handles).

## Decision

Revise ADR-0048 S3: manual aClass draws VISIBLE group-level edges.

1. Edge target: a manual platform draws an edge to every subscriptionGroup (subscription viewMode) / regionGroup (region viewMode) that contains >=1 of the platform manual_nodes node_hashes. Edge to the groups the manual selection actually touches, not all groups.
2. Edge label: manual:N where N = total manual_nodes on the platform. Consistent with the A-badge "Manual (N nodes)" and the strategy:value edge-label pattern (region:US / quality>75 / subscription:subA).
3. Deletability: manual edges are the ONLY deletable canvas edges (per CONTEXT.md). Auto-strategy edges (subscription/region/quality) remain non-deletable.
4. Deletion semantics: deleting a platform->groupX manual edge removes the manual_nodes whose node_hash belongs to groupX from the platform manual_nodes (clears that group manual selection; other groups keep their selections), then PATCHes strategyConfig. Mirrors the existing subgroup-edge deletion (delete platform->subgroup = remove that subscription regions from region_filters).
5. Layout consequence: the visible B->C edge lets dagre rank C-column at rank 2 (right of platform) for ALL aClass including manual, fixing the below-left bug with no layout-engine change. (Research alternative if a future aClass still yields no edge: dagre #root anchor / Kiali BoxLayout synthetic edges -- ADR-0051 ended up needing exactly this for port-less platforms and strategyConfig-load-failure; see ADR-0051 Defect B + "Upgrade path" for the @dagrejs/dagre >=1.1.5 official manual-ranking route.)

## Rejected alternatives

- A) Invisible dagre column anchors (minlen:0 same-rank) to force C right without visible edges (research-recommended). Rejected: contradicts CONTEXT.md "manual edges are drawn"; leaves the doc/code drift. Validated empirically (works) but domain-wrong.
- B) Switch layout engine to elkjs layerConstraint / fixed-column grid (Grafana NodeGraph). Rejected: heavier, new dependency; the visible-edge fix is sufficient and domain-correct.
- C) Keep "no manual edge", file a CONTEXT.md erratum. Rejected: the glossary is the source of truth and is correct; the code was the drift.

## Consequences

- ADR-0048 S3 revised: manual aClass now draws visible group edges (was: none). ADR-0048 S1 (per-node filtering), S2 (new-platform auto-default), S4 (getSelectedRegions removal), S5 (tests) unchanged.
- CONTEXT.md Strategy-Labeled Edge glossary confirmed as source of truth; sharpened with the manual:N label form and per-group deletion semantics.
- buildEdges (subscription viewMode) + the region-viewMode edge loop gain a manual branch: edge to groups containing selected node_hashes, label manual:N, deletable: true.
- onEdgesDelete gains a manual branch: remove groupX manual_nodes from manual_nodes, PATCH strategyConfig manual_nodes (node_hash -> group mapping at deletion time).
- Layout bug fixed for manual aClass. The strategy_config_get-failure fallback (empty manual_nodes) yields no C-column nodes (filteredNodes empty in buildCColumnGroups) -- no below-left bug, acceptable.
- Tests: the T22-3 assertion "manual = NO group-level edges" (TopologyView.test.tsx L452-458) is SUPERSEDED -- flip to "manual = visible edge to groups containing manual_nodes, label manual:N, deletable: true". Add an onEdgesDelete manual-branch test.

## Implementation status

Decision recorded via grill-with-docs (Q1-Q4). CODE APPLIED + VERIFIED 2026-08-23:
- TopologyView.tsx: buildEdges manual branch + adapted nodeHashes + region-viewMode manual branch + patchManualNodesViaStrategyConfig helper + manualNodesAfterGroupEdgeDelete pure fn + onEdgesDelete manual branch (8 edits).
- Tests: flipped T22-3 "manual=NO edges" -> "manual=visible edge manual:N deletable" + 2 edge-case tests; added manualNodesAfterGroupEdgeDelete describe (3 cases). Full vitest 300/300 green.
- Build: pnpm build green (tsc -b typecheck pass; Vite chunks TopologyView-B05WeKKD + index-CO3BJZPp). cargo build --release -p egressapikey-app --features custom-protocol green (13m58s).
- Staged release/windows-gui/EgressAPIKEY.exe (13.8MB); both Vite chunk hashes found in exe bytes (non-stale verified per AGENTS.md S5 step 4).
- Smoke-test: MainWindowTitle=EgressAPIKEY, WorkingSet ~38MB, alive 16s+, stdout "tracing initialized", no panic.
