# ai-api-route Refactor Plan - Topology-driven Key-to-Egress Routing

Status: PLANNING. Authored from live Resin v1.1.2 sidecar probes
(2026-07-31), not assumption. All API shapes below were verified against
the actual binary.

## Authoritative findings (live-probed, not assumed)

### Resin v1.1.2 admin API surface (verified endpoints)

- `/api/v1/platforms` GET - List platforms (items-wrapper) - 200
- `/api/v1/platforms` POST - Create platform - 201
- `/api/v1/platforms/{id}` DELETE - Delete platform - 204
- `/api/v1/platforms/{id}` PATCH - Update platform fields (validates) - 400 on bad enum
- `/api/v1/subscriptions` GET/POST - List/create subscriptions - 200/201
- `/api/v1/subscriptions/{id}` DELETE - Delete subscription - 204
- `/api/v1/nodes` GET - List proxy nodes (items-wrapper, unique_egress_ips) - 200
- `/api/v1/metrics/realtime/leases` GET - Active leases (per-platform) - 200
- `/api/v1/metrics/snapshots/node-pool` GET - Node pool health snapshot - 200
- `/healthz` GET - Unauth health - 200

Endpoints that DO NOT exist (probed, all 404): /lanes, /routes, /keys,
/api-keys, /egress, /policies, /config, /forward, /upstream, /strategy,
/clash, /openapi.json (403).

### Platform schema (the B category - api-key group/platform)

```
id: uuid
name: string
sticky_ttl: Go duration (e.g. 1h0m0s, 168h0m0s)
regex_filters: [upstream host patterns] or null
region_filters: [region codes] or null
routable_node_count: int
reverse_proxy_miss_action: TREAT_AS_EMPTY
reverse_proxy_empty_account_behavior: ACCOUNT_HEADER_RULE
reverse_proxy_fixed_account_header: Authorization
allocation_policy: BALANCED | PREFER_LOW_LATENCY | PREFER_IDLE_IP
passive_circuit_breaker_disabled: bool
updated_at: RFC3339
```

Key insight for requirement #1 (identify each stream key+endpoint combo):
reverse_proxy_fixed_account_header Authorization + regex_filters means Resin
inspects the inbound Authorization header and routes the request to the
platform whose regex_filters match the upstream host. This IS the key+endpoint
identification mechanism. The api-key in the Authorization header identifies
the key; regex_filters on the platform identifies the endpoint. Millisecond-
level and exact because it is header inspection, not packet deep-inspection.

Key insight for requirement #3 (egress strategy): allocation_policy is the
native egress-selection policy. Resin v1.1.2 supports exactly three values:
BALANCED, PREFER_LOW_LATENCY, PREFER_IDLE_IP. These map to the user's
PREFER_LOW_LATENCY (by latency) and BALANCED (by quality weight). Resin does
NOT expose random/sequential/bandwidth - those need shell-side logic or a
Resin upgrade. The plan honors what Resin actually supports.

### Nodes schema (the C category - ip/ip channels)

GET /api/v1/nodes returns items:[...], total, limit, offset,
unique_egress_ips, unique_healthy_egress_ips. Nodes come from subscriptions
(clash YAML proxies). Each node has an egress IP. The node-pool snapshot gives
aggregate health.

## Requirement mapping

- Req #1 identify key+endpoint: Resin native via reverse_proxy_fixed_account_header + regex_filters. GUI gap: let user set regex_filters per platform (= upstream endpoint match).
- Req #2 group keys into platform, hot-control egress: Platform CRUD + PATCH allocation_policy. GUI gap: expose platform-to-node binding + live PATCH.
- Req #3 ip channels from subs/local, whitebox config: subscriptions (remote/local content), nodes list. GUI gap: node-pool management tab; config-file control deferred (Resin owns clash YAML ingest).
- Req #3 egress policies: allocation_policy enum (3 values). GUI gap: expose the 3 native policies; document gap for random/sequential/bandwidth.
- Req #4 topology-to-config layer, hot-switch: platform PATCH updates routing live. Topology canvas edges = platform-to-node bindings; drag = PATCH.
- Req #5 three columns A/B/C: A=entry port (proxy listen port), B=platforms, C=nodes. Canvas redesign: 3 columns, A-to-B default-connected, B-to-C draggable.

## Phased delivery

### Phase R1 - Platform schema surface (backend IPC)
- Add platform_update IPC command (PATCH /api/v1/platforms/{id}) exposing regex_filters, allocation_policy, sticky_ttl.
- Add node_list IPC command (GET /api/v1/nodes) returning full node list with egress IPs.
- Closed-loop test: create platform, PATCH allocation_policy, verify field changed.

### Phase R2 - Topology canvas redesign (frontend)
- Three-column layout: A (entry proxy port, single node), B (platforms from platform_list), C (nodes from node_list).
- A-to-B: always-connected edges (proxy routes all inbound to all platforms).
- B-to-C: draggable edges = platform-to-node binding. Drag = PATCH.
- Open question: how does Resin bind a platform to specific nodes/egress IPs? regex_filters matches upstream hosts, not nodes. region_filters matches node regions. The platform-to-node binding may be implicit (all healthy nodes eligible, allocation_policy picks). Probe with real nodes loaded before coding.

### Phase R3 - Node/IP-channel management tab
- New tab: node pool list with health, egress IP, protocol.
- Import from subscription URL or local clash YAML (already works via P13).
- Per-node health polling (already in node-pool snapshot).

### Phase R4 - Whitebox config layer + backup
- Export current platform+subscription config as JSON/YAML.
- Import/restore with validation + backup before apply.
- Topology edge changes = config changes = hot PATCH + config file write.

## Open questions before Phase R2 code
1. Does Resin support explicit platform-to-node binding, or is it always all eligible nodes + allocation_policy picks? Probe with loaded nodes.
2. Can regex_filters target node names/egress IPs, or only upstream hosts?
3. Do random/sequential/bandwidth policies require a Resin fork or can the shell pre-filter the node pool to simulate them?

## Constraints (AGENTS.md alignment)
- All IPC validates input (7.5), no panics (P14), loopback-only ResinClient.
- i18n: new UI strings touch all 18 locales.
- Tests: each phase ships closed-loop tests.
- Build: release exe staged + smoke after each phase.
- codegraph sync after each phase.
