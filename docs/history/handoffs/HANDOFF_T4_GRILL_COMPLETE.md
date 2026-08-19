# HANDOFF_T4_GRILL_COMPLETE.md

## Status: Grill T4 Complete - Ready for Execution

All 5 grill questions answered by user. ADRs 0015-0019 created.
CONTEXT.md updated with 6 new domain terms. Resin v1.2.0 source cloned
to `resin/` (gitignored) for local reference.

## Decisions

1. **Q1 (SOCKS5)**: Expose proxy_token + port_health_check IPC (ADR-0015)
2. **Q2 (Strategies)**: Shell-side strategy_engine.rs, A-class + B-class (ADR-0016)
3. **Q3 (Node pool)**: Collapsible tree by subscription (ADR-0017)
4. **Q4 (Dead code)**: Delete laneCount + SharedGateway (ADR-0018)
5. **Q5 (Topology)**: Dynamic pool + strategy-labeled edges (ADR-0019)

## Execution Plan

Phase T4-1 (Q4) -> T4-2 (Q1) + T4-3 (Q3) parallel -> T4-4 (Q2) -> T4-5 (Q5)

## Key Files

- docs/adr/0015-socks5-auth-and-port-health.md
- docs/adr/0016-strategy-engine.md (indexed in ctx as adr-0016-strategy-engine)
- docs/adr/0017-node-pool-tree-view.md (indexed in ctx as adr-0017-node-pool-tree-view)
- docs/adr/0018-delete-lanecount-deadcode.md (indexed in ctx as adr-0018-delete-lanecount-deadcode)
- docs/adr/0019-topology-dynamic-pool-edges.md (indexed in ctx as adr-0019-topology-dynamic-pool-edges)
- docs/GRILL_T4_BACKLOG.md
- CONTEXT.md (updated with 6 new terms)
- resin/ (local clone of Resinat/Resin, gitignored)

## Total Goal Prompt

Follow all grill decisions above. Set goal, start large build, complete
all T4 phases per the backlog. Always follow AGENTS.md, use ctx_* plugins,
use 1mcp exa/perplexity for web research, no hallucination. Ponytail full mode.
Use ultragoal to set goal for tracking. Research existing mature templates
instead of building from scratch. All features must have closed-loop tests.