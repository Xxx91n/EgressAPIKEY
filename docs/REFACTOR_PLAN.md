# ai-api-route Refactor Plan - Topology-driven Key-to-Egress Routing

Status: R1 DONE (1fe2ce7) + R2 DONE (2fd8c5a) + R3 DONE (this commit). R4 pending. Authored from live Resin v1.1.2 sidecar probes
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
## RESOLVED open questions (live-probed 2026-07-31 with 33 real nodes loaded)

1. **Platform-to-node binding is via region_filters, NOT per-node.** Probed:
   - Default platform (no filters): routable_node_count = 12 (all healthy nodes).
   - Platform with region_filters:["hk"]: routable_node_count = 3 (HK nodes only).
   - Platform with regex_filters:["api.openai.com"] but no region_filters: routable_node_count = 0.
   So region_filters (lowercase ISO 3166-1 alpha-2, e.g. "hk","us","jp", or negation "!hk") selects
   which nodes are eligible. The Default platform is special: no filters = all healthy nodes.
   A non-Default platform with no region_filters gets 0 routable nodes (it must specify a region).

2. **regex_filters matches upstream REQUEST hosts, NOT nodes.** regex_filters:["api.openai.com"]
   means "this platform handles requests to api.openai.com" — it is the key+endpoint identification
   mechanism (req #1), not a node filter. It does NOT affect routable_node_count.

3. **Node schema has NO per-node egress_ip field.** GET /api/v1/nodes returns node_hash,
   display_tag, has_outbound, circuit_open_since, failure_count, latency probe timestamps, tags.
   The egress IP is only exposed as an aggregate count in the node-pool snapshot
   (egress_ip_count, unique_egress_ips). The topology canvas CANNOT show per-node exit IPs
   because Resin does not expose them. The canvas shows node display_tag + health instead.

4. **allocation_policy is the ONLY egress selection knob.** Resin v1.1.2 does not expose
   random/sequential/bandwidth. The canvas exposes the 3 native policies; random/sequential
   are documented as "not supported by Resin v1.1.2" in the UI.

## Topology canvas design (revised from findings)

The user's "drag line from B (platform) to C (specific node)" model maps to Resin's actual
behavior as follows:
- A (entry proxy port): single node, always-connected to all platforms. This is the Resin
  forward-proxy listen port.
- B (platforms): one box per platform. Each platform shows: name, regex_filters (which upstream
  hosts it handles), region_filters (which node regions it routes to), allocation_policy.
- C (nodes): one box per node, grouped by region. Each node shows: display_tag, health
  (failure_count, has_outbound), latency.
- B->C edges: NOT per-node. A platform's edges go to all nodes matching its region_filters.
  The canvas draws edges from a platform to the node-group whose region matches. Dragging a
  new edge from B to a C node-group = PATCH the platform's region_filters to include that region.
  This is the closest faithful mapping of the user's drag-to-connect intent to Resin's API.

Alternative considered: per-node binding (drag to a single node) would require a Resin fork
to add a platform->node_id filter. Out of scope for v1; documented as a limitation.

## Constraints (AGENTS.md alignment)
- All IPC validates input (7.5), no panics (P14), loopback-only ResinClient.
- i18n: new UI strings touch all 18 locales.
- Tests: each phase ships closed-loop tests.
- Build: release exe staged + smoke after each phase.
- codegraph sync after each phase.
