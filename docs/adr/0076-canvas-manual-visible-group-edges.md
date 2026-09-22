# ADR-0076: Canvas manual-mode visible group edges (revises ADR-0048 S3)

Date: 2026-09-22
Status: Accepted
Supersedes: ADR-0048 S3 (manual aClass -> NO group-level edge)
Builds on: ADR-0042 S4 (manual card checkbox), ADR-0048 (a_class-semantic
filtering), CONTEXT.md Strategy-Labeled Edge
Provenance: renumbered from branch `fix/zcode-GUI-Topology` ADR-0050
(recovered from commit `255eb7b8`, reimplemented for the CanvasV4 /
deep-IPC seam on main — the branch wrote `manual_nodes` via
`strategy_config_put`; main's sanctioned path is the new
`strategy_platform_manual_nodes_set` deep edit). Re-validated against
main 2026-09-22: main drew no manual group edges (`case "manual": break`)
and had no delete projection — the drift this ADR fixes was live.

## Context

dagre rank (column) is edge-derived (Sugiyama). ADR-0048 S3 decreed
manual = NO group-level edge because `manual_nodes` is node-level, not
group-level. That left manual-mode C-column nodes with no incoming B->C
edge -> disconnected -> rank 0 -> below-left of the platform column.

Doc tension: CONTEXT.md Strategy-Labeled Edge lists manual as a DRAWN,
DELETABLE edge ("only manual edges can be dragged/deleted"). ADR-0048 S3
over-extended "no leaf edge" (ReactFlow group nodes have no per-leaf
handles) into "no group edge" — but group nodes DO have handles; only
per-leaf edges are blocked.

## Decision

Manual aClass draws VISIBLE group-level edges:

1. **Edge target**: a manual platform draws an edge to every
   subscriptionGroup (subscription viewMode) / regionGroup (region
   viewMode) that contains >=1 of the platform's `manual_nodes`
   node_hashes — the groups the manual selection actually touches.
2. **Edge label**: `manual:N` where N = total manual_nodes on the
   platform (consistent with the A-badge "Manual (N nodes)" and the
   strategy:value label pattern).
3. **Deletability**: manual edges are the ONLY deletable canvas edges
   (per CONTEXT.md). Auto-strategy edges remain non-deletable.
4. **Deletion semantics**: deleting a platform->groupX manual edge
   removes the manual_nodes whose node_hash belongs to groupX, then
   writes `manual_nodes` via `strategy_platform_manual_nodes_set` ->
   `StrategyService::set_platform_manual_nodes` (new sanctioned deep
   write added this ticket; mirrors `set_platform_regions`).
5. **Layout consequence**: the visible B->C edge lets dagre rank
   C-column at rank 2 (right of platform) for manual platforms.

## Rejected alternatives

- Invisible dagre column anchors instead of visible edges. Rejected for
  the manual case (contradicts the glossary's drawn-edge rule) — the
  anchors DID land for the structural fallback case in ADR-0077 defect B.
- elkjs layerConstraint / fixed-column grid. Rejected: heavier, new
  dependency.

## Consequences

- ADR-0048 S3 revised: manual aClass draws visible group edges.
- `buildEdges` gains a manual branch (edge to groups containing selected
  node_hashes, label `manual:N`, `deletable: true`); group shape gains
  `nodeHashes`.
- `buildRegionViewEdges` gains the symmetric manual branch.
- `manualNodesAfterGroupEdgeDelete` pure helper + onEdgesDelete manual
  arm.
- Tests: flipped "manual = NO group-level edges" to "manual = visible
  edge manual:N deletable" + 2 edge cases + 3
  `manualNodesAfterGroupEdgeDelete` cases.
