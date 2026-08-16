# ADR-0041: Canvas v4 node pool + single aggregation column + MiniMap ariaLabel + toolbar merge

Date: 2026-08-17
Status: ACCEPTED
Supersedes: T14-4 region viewMode dual-list pattern, T16-3 MiniMap div[title] wrapper, T16-4 standalone openStrategyConfig button position

## Context

Four bugs surfaced after T16 commit (745837e):

1. **Entry-port column stray nodes**: in `viewMode === "region"` the rawNodes builder keeps
   the `subGroups.forEach((g) => { ... list.push({ type: "subscriptionGroup" ... }) }`` loop
   (L697-731) running unconditionally. Because edges in region viewMode are
   `platform-* -> regiongroup-*` NOT `platform-* -> subgroup-*`, the subGroup nodes
   have no incoming/outgoing edges. dagre's `rankdir: "LR"` layout places edgeless nodes
   into upper-left free space (nearest to the entry-port column due to rank balance),
   so the user sees a pile of "subscription" cards under or beside the entry-port
   column. This is the visual "端口下面莫名多出一堆节点的卡片" the user reported.
   T16-1's `if (!selectedRegions.has(region)) continue` guard only filters
   regionGroup pushes, not subGroup pushes — the root cause is the unconditional
   subGroup loop.

2. **openStrategyConfig button overlaps**: T16-4 placed the button at
   `absolute bottom-20 left-4 z-10` directly between CanvasControls (bottom-2 left-2)
   and the MiniMap (bottom-0 right-0). Three elements stack in one corner; button
   overlaps CanvasControls hover/click area on narrow viewports.

3. **MiniMap tooltip not i18n-realized**: T16-3 wrapped `<MiniMap>` in
   `<div title={t("topology.minimapHint")}>`. The wrapper is a static block whose
   only child is absolutely positioned; the wrapper therefore has height 0 and its
   native `title` tooltip never fires a hit-test. Source verification on
   @xyflow/react 12.11.2 dist shows `<MiniMap>` already renders
   `<svg role="img" aria-labelledby={id}><title>{ariaLabel}</title></svg>` when given
   the `ariaLabel` prop; this SVG `<title>` provides both the accessible name and a
   native hover tooltip in Chromium / WebView2. The div wrapper is dead weight.

4. **Node row duplication across subscriptions**: in region viewMode, the
   `for (const g of subGroups) for (const n of g.nodes) { entry.nodeRows.push(...) }`
   loop copies every node into the region bucket without dedup-by-node_hash. If the
   same node_hash appears under subscription A and subscription B (Resin deduplicates
   the upstream but not across different subscription names that reference the same
   proxy), the user sees identical rows twice inside one regionGroup. Kiali's
   canonical "group by X / group by Y" mental model has a single node instance per
   underlying entity; viewMode only changes the grouping dimension, not cardinality.

## Decision

### S1 — Single aggregation column + viewMode guard on subGroup

The subGroup push block (L697-731) MUST be gated by `if (viewMode !== "subscription") return;`
early in its body. Symmetrically, the regionGroup push block (L735-767) is already gated
by `if (viewMode === "region")` — leave it. Result: in any viewMode, exactly one C-column
node type (subscriptionGroup OR regionGroup) is constructed. No edgeless nodes leak into
dagre's free-space fallback.

Kiali reference: "group by app / group by version" preserves one node-per-entity invariant
by construction; the viewMode dimension is the only thing that changes. Same invariant here.

### S2 — Cross-subscription node dedup by node_hash

Both the region viewMode route (L744-751 for loop) and the subscription viewMode route
(L718-726 nodeRows map) MUST dedup source nodes by a stable identity key BEFORE building
the display rows. Resin v1.2.0 `/api/v1/nodes` returns a `node_hash` field per node;
we use it as the dedup key. When iterating `for (const g of subGroups) for (const n of
g.nodes)` in region viewMode, maintain a `Set<string> seen = new Set()` and skip when
`seen.has(n.node_hash)`. Similarly for subscription viewMode inside one subscription
group: dedup by node_hash within that group (this was previously implicit because one
subscription rarely echoes nodes; we now guard it explicitly).

### S3 — MiniMap ariaLabel replaces div[title]

Replace the ``<div title={...}> <MiniMap ... /> </div>` footer (L955-972) with
`<MiniMap ariaLabel={t("topology.minimapHint")} pannable zoomable nodeColor={...} maskColor={...} />`.
Drop the wrapper div entirely. `minimapHint` key stays across 18 locales. The ariaLabel
prop is the official xyflow 12 surface (verified by reading dist + intent through issue
#2858 where a maintainer documents setting `ariaLabel={null}` removes the default
"Mini Map" tooltip — proving the default IS surfaced as a native tooltip).

### S4 — openStrategyConfig merged into CanvasControls toolbar

Move the openStrategyConfig onClick block from the standalone `<button>` at L942-954 into
the CanvasControls component at L478-509 as a third row / a new button next to the existing
viewMode segmented control + zoom + fit + lock buttons. The toolbar group becomes one
absolute-positioned component at `bottom-2 left-2`; the standalone L942-954 block is
deleted entirely. No overlap with MiniMap.

## Rejected alternatives

- S1-reject: keep subGroup + regionGroup both rendered in region viewMode but add
  invisible dagre `rank=same` constraints to pin subGroups to C column. Rejected: it
  doubles the C-column cardinality in region viewMode and requires hand-maintained
  invisible-edge bookkeeping. The viewMode switch already implies which grouping the
  user wants; a one-line early-return guard is strictly simpler.

- S2-reject: dedup by `display_tag` instead of `node_hash`. Rejected: two unrelated
  proxies can share `display_tag` (e.g. both "Japan-01" from different providers);
  display_tag is not a stable identity.

- S3-reject: wrap MiniMap with Radix/Floating UI Tooltip. Rejected as the primary fix:
  MiniMap's own `ariaLabel` already gives native hover tooltip with zero dependencies.
  Radix/Floating would be a future enhancement only if brand-styled tooltips are needed.

- S4-reject: standalone button moved to `top-4 right-4` with lucide FileCog icon.
  Rejected: the user explicitly chose "merge into CanvasControls toolbar" (Q3 = A). Two
  tool groups in the corners is the anti-pattern the user called out as "occlusion".

## Consequences

- TopologyView.tsx: +6 lines of guard (S1) + ~10 lines of dedup Set (S2) + delete 18 lines
  of div[title] wrapper (S3) + delete 13 lines of standalone button (S4) + add ~15 lines
  inside CanvasControls = net ~0 lines change.
- No new i18n keys. `topology.minimapHint` + `topology.openStrategyConfig` already exist
  in 18 locales and are reused unchanged.
- No new dependencies (Radix/Floating explicitly rejected).
- Tests: +8 vitest = 225 -> 233 total. See plan for the 8 test names.
- ADR-0040 T16-3 + T16-4 partially superseded in implementation; ADR-0040 text stays as
  history (per AGENTS §10 audit-trail protection).
