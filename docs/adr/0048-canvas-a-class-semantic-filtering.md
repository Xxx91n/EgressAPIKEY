# ADR-0048: Canvas C-column a_class-semantic filtering + new-platform default strategyConfig entry

Status: ACCEPTED
> Date: 2026-08-19
> Supersedes: T13-1 getSelectedRegions global-union filtering (L104-110), buildCColumnGroups selectedRegions param (L185-203), buildEdges region_filters-only B->C edge (L620-641)
> Builds on: ADR-0036 strategyConfig single source, ADR-0039 canvas fold, ADR-0041 canvas v4, ADR-0042 canvas pipeline sync

## Context

Bug 1: Canvas C-column stuck showing only FI region nodes. Root cause: buildCColumnGroups (L203) filters nodes via `selectedRegions.has(getNodeRegion(n))` where selectedRegions is the global union of all platforms' region_filters. strategyConfig merge (L784) overwrites region_filters from ps.regions, so a config with `"regions": ["fi"]` makes C-column show only FI nodes regardless of subscription switches or a_class semantics.

Bug 2: New platforms appear below nodes instead of in B column. Root cause: buildEdges (L620-641) only draws B->C edges for platforms with non-empty region_filters. New platforms not in strategyConfig have empty region_filters -> 0 B->C edges -> dagre assigns lowest rank (bottom of graph).

## Decision

### S1 — C-column filtering by a_class semantics (Q1+Q3+Q4)

buildCColumnGroups signature changes from `selectedRegions: Set<string>` to `platforms: PlatformFull[]`. Node filtering logic per platform:
- a_class=subscription: show nodes whose subscriptionName is in platform.subscriptionNames; empty subscriptions = show all (new platform default)
- a_class=region: show nodes whose region is in platform.region_filters
- a_class=quality: show all healthy nodes (Top-N ranking is card-internal)
- a_class=manual: show nodes whose node_hash is in platform.manualNodes

Both subscription and region viewMode share the same per-node filter helper `isNodeSelectedByAnyPlatform(node, platforms, groupKey)`.

### S2 — New-platform default strategyConfig entry (Q2)

In sync() merge block (L774-792), if Resin returns a platform not in strategyConfig, auto-create default entry: `{ platform_name, a_class: "subscription", b_class: "random", subscriptions: [], regions: [] }`. This auto-entry is ephemeral (written to in-memory merge only, not persisted to strategyConfig JSON until user explicitly configures).

### S3 — buildEdges B->C edge by a_class semantics (Q5)

buildEdges B->C edge logic per platform:
- a_class=subscription + subscriptions=[] -> edge to ALL subscription/region groups (unconfigured = unrestricted)
- a_class=subscription + subscriptions=["subA"] -> edge to subA groups only
- a_class=region + regions=["fi"] -> edge to fi region groups only
- a_class=quality -> edge to ALL groups (quality considers all healthy nodes)
- a_class=manual -> NO group-level edge (manual_nodes is node-level, not group-level)

### S4 — getSelectedRegions removal (Q7)

Delete getSelectedRegions (L104-110). Remove selectedRegions useMemo (L859). buildCColumnGroups receives platforms directly. No dead code residue.

### S5 — Test closed-loop (Q6)

12 new vitest assertions:
- isNodeSelectedByAnyPlatform: 4 a_class branches + empty subscriptions = all + missing aClass default
- buildEdges a_class: subscription+empty = all edges, subscription+specific = selective, region = selective
- sync merge: Resin platform not in strategyConfig -> default entry auto-created

## Rejected alternatives

- B) C-column shows all nodes, region filtering only on edge connection (rejected: visual clutter, user loses at-a-glance "what does this platform see" information)
- C) region_filters authoritative source stays Resin backend, not strategyConfig overlay (rejected: breaks ADR-0036 single-source-of-truth for strategy)

## Consequences

- Canvas C-column reflects per-platform a_class strategy semantics, not global region union
- New platforms appear in B column with edges to all subscription groups (unconfigured = unrestricted)
- strategyConfig JSON remains the whitebox source of truth; sync merge auto-fills missing entries in-memory
- Manual mode platforms have no B->C group edges (manual selection is node-level via card checkbox, ADR-0042 S4)
