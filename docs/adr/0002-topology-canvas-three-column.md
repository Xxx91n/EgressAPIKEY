# ADR-0002: Three-column topology canvas (A/B/C) maps to Resin region_filters

Date: 2026-08-02
Status: PROPOSED (provisionally resolved from evidence; pending user grill)
Decision Type: UX + Architecture

## Context

User requirement #5: three columns A (entry proxy port), B (api-key
platforms), C (ip/ip channels). A-to-B always connected; B drags to C to bind
egress. The topology canvas is the visual hot-edit surface for routing.

## Live-probed Resin behaviour (2026-07-31, 33 real nodes)

- Platform-to-node binding is via region_filters (region codes), NOT per-node.
  Default platform (no filters): routable = all healthy.
  Platform with region_filters:["hk"]: routable = 3 HK nodes only.
  Platform with regex_filters:["api.openai.com"] but no region_filters:
  routable = 0 (regex matches upstream REQUEST hosts, NOT nodes).
- A non-Default platform with no region_filters gets 0 routable nodes — it
  must specify a region to be useful.

## Decision

Keep the three-column A/B/C canvas:
- A = single conceptual "entry proxy port" node (the shell forwards all
  traffic to a single Resin forward proxy listener; visually one node).
- B = one node per live Resin platform from GET /api/v1/platforms.
- C = node-region GROUPS from GET /api/v1/nodes (collapsed by region tag).
- B-to-C edges = platform.region_filters includes that region code.
- Dragging a new B-to-C edge = PATCH the platform region_filters live.
- Deleting an edge = PATCH region_filters to remove that region.

## Rationale

This maps 1:1 to Resin's native contract. Every visual edge IS a real config
field on a real Resin entity. Hot PATCH on drag = immediate config edit.
Backup issued before every drag PATCH (P19 item 4) so changes are reversible.

## What this is NOT

- It is NOT a per-node binding. Resin does not expose per-node selection.
- It is NOT a key-to-lane-slot mapping. The old Lane concept is a shell-only
  UI artefact; the actual routing is Platform.Account->Lease->Node, owned
  entirely by Resin's in-process Go runtime.
- It does NOT promise bandwidth/sequential/random egress policies. Resin
  v1.1.2 exposes exactly three allocation_policy values (BALANCED,
  PREFER_LOW_LATENCY, PREFER_IDLE_IP); see ADR-0003.

## Consequences

- The canvas UI is honest: every visible edge corresponds to a Resin config
  field. No visual element floats without backend semantics.
- The user's requested "single IP / multi IP strategy" maps to:
  - single IP = region_filters with one region, sticky_ttl long enough.
  - multi IP = region_filters with multiple regions, allocation_policy =
    BALANCED or PREFER_LOW_LATENCY (Resin rotates within eligible nodes).
