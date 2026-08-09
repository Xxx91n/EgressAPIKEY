# Domain-Modeling Audit — Grill T5 Round (2026-08-10)

## Context
Audit performed after grill Q1-Q10 decisions recorded.
Purpose: verify which decisions are already landed in code vs pending.

## CONTEXT.md Terms Audit

| Term | In CONTEXT.md | In Code | Status |
|------|---------------|---------|--------|
| Entry Port | Yes | Resin /api/v1/endpoints (create_endpoint, list_endpoints) | ✅ Aligned |
| Platform | Yes | Resin /api/v1/platforms (CRUD via resin_client.rs) | ✅ Aligned |
| Account | Yes | Resin identity_v1.go parseV1PlatformAccountIdentity | ✅ Aligned |
| Lease | Yes | Resin routing/lease.go LeaseTable (key=account, val=lease+NodeHash) | ✅ Aligned |
| Subscription | Yes | resin_client.rs create_subscription/list_subscriptions + clash YAML fetch | ✅ Aligned |
| Node | Yes | resin_client.rs list_nodes / list_nodes_for_platform | ✅ Aligned |
| Ghost Safety Net | Yes | sidecar.rs spawn_health_poll (3-fail → tray red + clear OS proxy) | ✅ Aligned |
| Sidecar | Yes | sidecar.rs boot_resin + SidecarHandle + RunEvent::Exit kill | ✅ Aligned |
| Egress IP Policy | Yes | strategy.rs StrategyId::to_resin_allocation_policy | ✅ Aligned |
| Topology Canvas | Yes | TopologyView.tsx 3-column A/B/C canvas | ✅ Aligned |
| Strategy Layer | Yes | strategy_engine.rs AClassStrategy (4 variants) + BClassStrategy | ⚠️ Partial — B-class not fully wired to Resin |
| AI Stream Sensor | Yes | No code yet (planned as independent pluggable module) | ❌ Not landed |
| IP Reputation Provider | Yes | No code yet (planned: IPQualityScore/AbuseIPDB/ip-api) | ❌ Not landed |
| Protocol Weight | Yes | strategy.rs protocol_weight() static table | ✅ Aligned |
| Backup | Yes | commands/mod.rs backup_create/backup_upload (P14 path-traversal fixed) | ✅ Aligned |
| Entry Port Mapping | Yes | DbPool + SQLite (PRAGMA user_version) for port→platform mapping | ⚠️ Schema exists, port_forwarder wiring partial |
| hotswap-config | Yes | No code yet (ADR-0012 decided, not implemented) | ❌ Not landed |
| RunningMode | Yes | sidecar.rs RunningMode enum + ArcSwap<State> + crash restart (T2) | ✅ Aligned |
| Ring Buffer | Yes | sidecar.rs CommandEvent ring buffer (T2-Q1) | ✅ Aligned |
| Crash Restart | Yes | sidecar.rs crash restart with bounded retry (T2-Q3) | ✅ Aligned |
| Two-Phase Shutdown | Yes | sidecar.rs RunEvent::Exit → child.kill() (P22 orphan fix) | ✅ Aligned |
| Port Cleanup | Yes | sidecar.rs cleanup on exit | ✅ Aligned |
| Upstream Manifest | Yes | docs/RESIN_UPSTREAM_MANIFEST.yaml (ADR-0017) | ✅ Aligned |
| Read Retry | Yes | resin_client.rs send_read() with auto-retry (ADR-0018) | ✅ Aligned |
| A-Class Strategy | Yes | strategy_engine.rs AClassStrategy enum (Manual/Region/Quality/Subscription) | ✅ Aligned |
| B-Class Strategy | Yes | strategy.rs StrategyId enum (6 variants → 3 Resin policies) | ⚠️ 6→3 mapping documented but scheme c (Q10) not landed |
| Strategy Engine | Yes | strategy_engine.rs (393 lines, 9 cargo tests) | ✅ Aligned |
| Port Auth Info | Yes | commands/mod.rs port_auth_info IPC (T4-2) | ✅ Aligned |
| Port Health Check | Yes | commands/mod.rs port_health_check IPC (T4-2) | ⚠️ Protocol-aware prober (Q9) not landed |
| Strategy-Labeled Edge | Yes | TopologyView.tsx buildEdges with strategy labels (T4-5) | ✅ Aligned |

## ADR Audit

| ADR | Title | Status | Code Landed? |
|-----|-------|--------|-------------|
| 0001 | Shell forwarding vs Resin webui port | SUPERSEDED | N/A |
| 0002 | Topology canvas three-column | ACCEPTED | ✅ TopologyView.tsx |
| 0003 | Key+endpoint identification | SUPERSEDED by 0012 | Code deleted per ADR-0014 |
| 0004 | Egress policy surface | SUPERSEDED | N/A |
| 0005 | Q5-Q9 cluster | SUPERSEDED | N/A |
| 0006 | Mainline A then B | ACCEPTED | ✅ Executed |
| 0007 | Subscription last_error surface | ACCEPTED | ✅ SubscriptionsView refreshWithRetry |
| 0008 | Item3 live sidecar e2e | ACCEPTED | ✅ release exe smoke |
| 0009 | CLI-GUI alignment status | ACCEPTED | ⚠️ Partial (no npm install path yet) |
| 0010 | Release strategy B3 dual track | ACCEPTED | ⚠️ CI manual trigger only (A7) |
| 0011 | Observed key pool SQLite B-B-3 | SUPERSEDED by 0012 | Code deleted per ADR-0014 |
| 0012 | Route correction thin-shell multi-port | ACCEPTED | ⚠️ port_forwarder exists, endpoint API wired, no multi-port listener yet |
| 0013 | Project rename to EgressAPIKEY | ACCEPTED | ✅ All renamed |
| 0014 | Dead code deletion interceptor/route_id | ACCEPTED | ✅ interceptor.rs deleted, route_id gone |
| 0015 | Resin v1.2.0 upgrade port_forwarder downgrade | ACCEPTED | ⚠️ port_forwarder downgraded to 5 IPC endpoint API |
| 0016 | Hand-rolled sidecar lifecycle | ACCEPTED | ✅ sidecar.rs (T2) |
| 0017 | Upstream Resin version manifest | ACCEPTED | ✅ RESIN_UPSTREAM_MANIFEST.yaml |
| 0018 | Read-only IPC auto-retry | ACCEPTED | ✅ resin_client.rs send_read() |
| 0019 | build-all.ps1 + release standardization | ACCEPTED | ⚠️ build-all.ps1 exists but bug 1 (duplicate) |
| 0020 | Test per capability closed loop | ACCEPTED | ✅ Enforced |
| 0021 | SOCKS5 auth + port health | ACCEPTED | ✅ port_auth_info + port_health_check |
| 0022 | Shell-side strategy engine | ACCEPTED | ✅ strategy_engine.rs |
| 0023 | Node pool tree view | ACCEPTED | ✅ NodesView.tsx |
| 0024 | Delete laneCount dead code | ACCEPTED | ✅ laneCount removed from appStore + Settings |
| 0025 | Topology dynamic pool edges | ACCEPTED | ✅ TopologyView.tsx |
| 0026 | (pending) accountId-encoded B-class + IpcError trace | TO CREATE | ❌ Not landed |

## Code-Level Verification Summary

### Already Landed (T4 + earlier):
- T4-1: Dead code cleanup (laneCount, SharedGateway) ✅
- T4-2: port_auth_info + port_health_check IPC ✅
- T4-3: NodesView collapsible tree by subscription ✅
- T4-4: strategy_engine.rs A-class 4 variants ✅
- T4-5: TopologyView strategy-labeled edges ✅
- T2: Sidecar lifecycle (RunningMode, RingBuffer, CrashRestart, Two-Phase Shutdown) ✅
- T1: Port forwarding endpoints IPC ✅
- P22: Orphan sidecar fix ✅
- P13: Subscription fetch UA + flow→block convert ✅

### NOT Landed (This Grill Round Q1-Q10):
1. **Q1-Q5 trace_id infrastructure**: 0 occurrences of trace_id, invokeWithTrace, #[instrument], info_span
2. **Q6-Q8 IpcError contract**: 0 occurrences of IpcError, BindConflict, InvalidStrategy
3. **Q9 port_health_check protocol-aware**: exists but sends SOCKS5 greeting for ALL protocols
4. **Q10 B-class scheme c**: accountId encoding B-class strategy hint not implemented
5. **Bug 1**: build-all.ps1 duplicate headless exe
6. **Bug 2**: PlatformsView b_class default "balanced" (should be "random")
7. **Bug 3**: bind conflict raw error (will be handled by IpcError::BindConflict)
8. **Bug 4**: HTTP health check + SOCKS5 creds display for all

### Gaps in CONTEXT.md:
- No term for "trace_id" / "trace span" — should add when infra lands
- No term for "IpcError" — should add when contract lands
- "AI Stream Sensor" term exists in CONTEXT.md but NO code — accurate (planned, not started)
- "hotswap-config" term exists but NO code — accurate (planned, not started)

## Conclusion
The grill T5 round (Q1-Q10) is fully planned but NONE of the execution has started.
All 4 bugs + trace infrastructure + IpcError contract + B-class scheme c are pending.
The plan table below captures the full execution scope.
