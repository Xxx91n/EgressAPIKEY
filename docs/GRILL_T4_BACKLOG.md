# Grill T4 Issues Backlog (2026-08-09)

> All 5 grill questions answered. ADRs 0015-0019 created.
> CONTEXT.md updated with 6 new domain terms.

## Decisions Summary

| Q  | Topic                        | Decision | ADR    |
|----|------------------------------|----------|--------|
| Q1 | SOCKS5 port connectivity     | Expose proxy_token to GUI + port_health_check IPC | ADR-0021 |
| Q2 | Platform egress strategies   | Shell-side strategy_engine.rs (A-class + B-class) | ADR-0022 |
| Q3 | Node pool display            | Collapsible tree by subscription (clash-verge-dev pattern) | ADR-0023 |
| Q4 | Settings dead field          | Delete legacy dead code (laneCount + SharedGateway) | ADR-0024 |
| Q5 | Topology canvas C-column     | Dynamic pool + strategy-labeled edges (Kiali pattern) | ADR-0025 |

## Execution Order (dependency graph)

```
Q4 (delete dead code) --- no deps, do first
Q1 (SOCKS5 auth + health) --- no deps, parallel with Q4
Q3 (node pool tree view) --- no deps, parallel with Q4/Q1
Q2 (strategy engine) --- depends on Q1 (port health) for liveness gate display
Q5 (topology refactor) --- depends on Q2 (strategy data) + Q3 (node grouping)
```

## Phases

### Phase T4-1: Dead Code Cleanup (Q4 / ADR-0024)
- Delete laneCount from appStore.ts + SettingsView.tsx
- Delete SharedGateway / GatewayState if no other callers
- Delete AI_API_ROUTE_LANES env var + build_shared_gateway
- Test: cargo check + vitest

### Phase T4-2: SOCKS5 Auth + Port Health (Q1 / ADR-0021)
- New IPC: port_auth_info(port) -> { username, password, auth_required }
- New IPC: port_health_check(port) -> { reachable, auth_required, latency_ms }
- GUI: port cards show auth credentials + copy button + health indicator
- Test: cargo test (Rust) + vitest (GUI)

### Phase T4-3: Node Pool Tree View (Q3 / ADR-0023)
- Refactor NodesView.tsx: flat table -> collapsible tree by subscription
- Add reference_latency_ms to NodeItem interface
- Latency color coding: <200ms green, 200-500ms yellow, >500ms red, timeout gray
- Search box + row expand for details
- Test: vitest

### Phase T4-4: Strategy Engine (Q2 / ADR-0022)
- New module: crates/resin-core/src/strategy_engine.rs
- A-class: poll /nodes -> filter by strategy -> PATCH /platforms
- B-class: subscribe leases -> bias selection (random/round_robin/low_latency)
- Whitebox: egressapikey-strategy.json (hotswap-config)
- GUI: strategy selection panel per platform
- Test: cargo test (strategy logic) + vitest (GUI)

### Phase T4-5: Topology Canvas Refactor (Q5 / ADR-0025)
- C-column: subscription-folded node pool (aligned with Q3)
- B->C edges: strategy-driven with labels
- B-column: show B-class strategy name on platform nodes
- buildEdges() rewrite for strategy-aware edges
- Test: vitest (edge builder + rendering)

## Resin v1.2.0 Source Reference
Local clone at `resin/` (gitignored). Key files:
- internal/proxy/socks5.go - SOCKS5 handler with auth
- internal/service/control_plane_endpoint.go - endpoint CRUD + validation
- internal/service/control_plane_platform.go - platform CRUD + NodeSummary
- internal/service/control_plane_nodes.go - node listing + filters
- internal/probe/manager.go - active probing (egress + latency)
- internal/platform/policy.go - AllocationPolicy enum (3 values)
- cmd/resin/endpoint_runtime.go - listener binding + demux
- cmd/resin/inbound_demux.go - first-byte protocol sniffing