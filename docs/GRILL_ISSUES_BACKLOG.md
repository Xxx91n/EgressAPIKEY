# GRILL ISSUES BACKLOG (Route-corrected — 2026-08-05)

> ADR-0012 (route correction) + ADR-0013 (rename to EgressAPIKEY) + ADR-0014
> (dead code deletion) are ACCEPTED. This backlog reflects the post-correction
> state. Items marked SUPERSEDED are kept for audit trail only.

## Q1-Q11 Decision Summary (this grill round)

| Q | Topic | Decision | ADR |
|---|---|---|---|
| Q1 | Reuse Resin vs build new | Reuse Resin (Ponytail) | ADR-0012 |
| Q2 | Architecture: thin shell A vs fork C | Path A (thin shell, not fork) | ADR-0012 |
| Q3 | HTTP vs SOCKS5 identity | HTTP for unified port; multi-port for per-key isolation | ADR-0012 |
| Q4 | GUI + whitebox config sync | A+B both (GUI for masses, whitebox for power users, atomic sync) | ADR-0012 |
| Q5 | Config hot-reload mechanism | hotswap-config (community-vetted wheel) | ADR-0012 |
| Q6 | Commit granularity | Small step commits (Ponytail) | — |
| Q7 | Strategy layer + AI stream sensor + IP reputation | Modular pluggable strategies (not global); independent AI stream sensor board; IP reputation via external API wheels (IPQualityScore/AbuseIPDB/ip-api) | ADR-0012 |
| Q8 | IP reputation scoring | Use open-source mature wheels, not self-built; IPQualityScore (5K/mo free) + AbuseIPDB (1K/day) + ip-api.com (45/min) | ADR-0012 |
| Q9 | Project rename | EgressAPIKEY (no collision, audited 5 sources) | ADR-0013 |
| Q10 | Dead code: delete vs keep vs refactor | (a) Delete all dead code (Ponytail clean break) | ADR-0014 |
| Q11 | Backlog audit:失效/保留/重定义 | See tables below | — |

## 失效项 (SUPERSEDED — kept for audit trail only)

| Item | Reason |
|---|---|
| C1-1 (observed_keys SQLite + route_id GUI chips) | route_id based on reading Authorization header; HTTPS makes this impossible. SUPERSEDED by ADR-0012. |
| ADR-0003 (key+endpoint identification via header) | Same premise error. SUPERSEDED by ADR-0012. |
| ADR-0011 (B-B-3 route_id-derived key identity) | Same. SUPERSEDED by ADR-0012. SQLite infra reused; observed_keys schema deleted. |
| P24-A4-3 (axum interceptor) | Entire file deleted per ADR-0014. |
| C2-3 (keyCandidates cross-view persistence) | keyCandidates concept (manual endpoint+apiKey input) abolished. Replaced by Entry Ports. |
| C2-4 (auto vs manual platform card visual) | key candidate concept gone; platform card visual stays but binds to ports not keys. |
| TopologyView B-column route_id chip rendering | Deleted. B column now renders port-based identity. |

## 保留项 (unaffected by route correction)

| Item | Status | Notes |
|---|---|---|
| C1-2 (hot-connect atomic PATCH + state refresh) | confirmed | Topology edge PATCH semantics unchanged |
| C1-3 (drag-delete edge = remove region_filters) | confirmed | Idempotent removal unchanged |
| C1-4 (viewport memory + initial fitView) | confirmed | Pure frontend, identity-agnostic |
| C2-1 (subscription drag reorder persists) | done=607460a | Unaffected |
| C2-2 (subscription rename/delete coexistence) | done=192bc82 | Unaffected |
| C2-5 (tray i18n real-time sync) | done=55f6583 | Unaffected |
| C2-7 (Settings locale switch re-renders canvas) | done=d109e86 | Unaffected |
| C2-8 (Settings dirty-state sticky save bar) | done=f359c82 | Unaffected |
| C2-9 (log system maturation) | pending | More important now (new architecture needs debug) |
| C2-10 (path protection / privilege escalation) | pending | Unaffected |
| C2-11 (env var / OS side-effect protection) | pending | Unaffected |
| C2-12 (second instance guard + focus) | pending | Unaffected |
| C2-13 (close GUI keeps tray icon) | pending | Unaffected |
| P25burst-1 (reset-sort duplicates) | done (P20 fix) | Unaffected |
| P25burst-2 (node pool stats vs table mismatch) | done (P20 fix) | Unaffected |
| P25burst-3 (import 0 nodes) | done (P13 clash UA fix) | Unaffected |
| P25burst-4 (platform add OK but not shown) | done (C2-14) | Unaffected |
| Subscriptions CRUD + clash UA fetch | live | Unaffected |
| Node pool / IP channels | live | Unaffected |
| Ghost safety net | live | Unaffected |
| Sidecar lifecycle | live | Unaffected |
| Backup/config export-import | live | Unaffected |

## 重定义项 (concept survives, binding changes)

| Item | Old | New |
|---|---|---|
| TopologyView A column | Single conceptual entry node | Multiple actual entry ports (socks5/http), each port = one identity |
| TopologyView B column | Platforms with route_id chips | Platforms with port-based identity chips (port number + protocol) |
| PlatformsView left pane | keyCandidates (endpoint+apiKey) | Entry Ports (port number + protocol + status) |
| PlatformsView right pane | Platforms (drag key -> platform) | Platforms (drag port -> platform) |
| ProcessRouteView | Process -> lane mapping | Process -> port mapping (lane concept deprecated) |
| gateway_reserve/release/evict_lane IPC | Lane-based echo commands | Port-based or deleted (Resin owns semantics) |
| REFACTOR_PLAN.md Phase R2-R4 | R2 canvas + R3 node mgmt + R4 config | R2 canvas (A col = ports) + R3 node mgmt (unchanged) + R4 config (add port->platform mapping) |
| C2-14 (platform create dialog) | Create platform + drag key candidate | Create platform + drag entry port to bind |

## 新增项 (did not exist before route correction)

| Item | Description | Priority |
|---|---|---|
| NEW-1 | Multi-port socks5/http listener core (tokio TcpListener + protocol detection) | blocking — the core of the new architecture |
| NEW-2 | Port -> (platform, account) mapping table (SQLite, reuses DbPool) | blocking — identity layer |
| NEW-3 | X-Resin-Account injection on forward (port-based, not header-based) | blocking — replaces interceptor |
| NEW-4 | Modular strategy layer (pluggable: liveness, latency, bandwidth, quality, protocol weight) | high — Q7 decision |
| NEW-5 | AI stream sensor module (SSE/WS awareness, independent board) | high — Q7 decision |
| NEW-6 | IP reputation integration (IPQualityScore + AbuseIPDB + ip-api, pluggable) | medium — Q8 decision |
| NEW-7 | hotswap-config (whitebox config layer, atomic backup, hot-reload) | high — A5 decision |
| NEW-8 | Project rename to EgressAPIKEY (repo, Cargo.toml, tauri.conf, package.json, README, AGENTS) | high — Q9 decision |
| NEW-9 | Dead code deletion (interceptor.rs, route_id, observed_keys, ADR-0003/0011 superseded) | blocking — Q10 decision |
| NEW-10 | MEMORY_REUSE_DECISION.md update (route correction conclusion固化) | medium — documentation |
| NEW-11 | New HANDOFF document (post-correction execution plan) | medium — handoff |

## Execution order (dependency graph)

NEW-9 (delete dead code) -> NEW-8 (rename) -> NEW-1 (multi-port listener) ->
NEW-2 (port->platform mapping) -> NEW-3 (X-Resin-Account injection) ->
NEW-7 (hotswap-config) -> NEW-4 (strategy layer) -> NEW-5 (AI stream sensor) ->
NEW-6 (IP reputation) -> 重定义项 (TopologyView/PlatformsView rewrite) ->
保留 pending 项 (C2-9/10/11/12/13)

## Open Questions (pending grill — none at this time)

All Q1-Q11 resolved. Next grill round starts when execution hits a new
ambiguity or the user raises a new concern.
