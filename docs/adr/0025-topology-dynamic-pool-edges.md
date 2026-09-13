# ADR-0025: Topology Canvas C-Column Dynamic Pool + Strategy-Labeled Edges

Status: ACCEPTED
**Date**: 2026-08-09

## Context

C-column groups by region. After ADR-0022 strategy engine, platforms select
nodes via multiple A-class strategies, not just region. Region grouping no
longer represents real B->C relationship.

## Decision

### C-column: subscription-folded node pool (aligned with ADR-0023)
- Nodes grouped by subscription source (collapsible)
- Each node: display_tag + latency color + health status

### B->C edges: strategy-driven
- Edge drawn when platform A-class strategy selects a node
- Edge label: "manual", "region:US", "quality>75"
- Multiple edges = multiple strategies coexist
- Drag B->C to node = add to platform A-class manual list
- Auto-strategy edges non-deletable

### B-column: show B-class strategy name on platform nodes

## Scope
~200 lines TopologyView refactor + tests. Depends on ADR-0022 + ADR-0023.