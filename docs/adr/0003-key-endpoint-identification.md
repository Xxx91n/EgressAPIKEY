# ADR-0003: key+endpoint identification is Resin-native (Authorization header + regex_filters)

Date: 2026-08-02
Status: PROPOSED (provisionally resolved from evidence)
Decision Type: Domain contract (no code change needed)

## Context

User requirement #1: identify each stream's (api_key + upstream v1 endpoint)
combination uniquely, millisecond-level, exact, no false positives.

## Decision

Do NOT build a shell-side identifier. Resin already identifies each
(api_key, upstream_endpoint) pair in flight at the gateway via:
- reverse_proxy_fixed_account_header: "Authorization" (the api-key lives here)
- regex_filters on the platform = upstream host patterns (the endpoint lives
  here)

The Authorization header value = the api-key. The platform whose
regex_filters match the upstream REQUEST Host header = the endpoint
identity. Resin resolves platform by regex match, account by header value,
in-process, on the hot path. Millisecond, exact, unique.

## Rationale

- Sub-millisecond HTTP header parse, not packet deep inspection.
- Exact match: HTTP headers are byte-exact; regex_filters match request Host.
- Unique: distinct (api_key, endpoint) pair -> distinct (Platform, Account).
  Two keys on the same endpoint -> same Platform, different Account -> distinct
  lease entries, distinct egress IPs by default.
- No new code: the Phase R1 platform_update IPC already exposes
  regex_filters + reverse_proxy_fixed_account_header on the PATCH surface.

## What this is NOT

- It does NOT require a packet-level deep inspector.
- It does NOT require a custom hash. Resin's Account string IS the identity.
- It does NOT mean shell has the mapping — Resin owns the full lease table
  internally; the shell only reads aggregate active-lease counts for display.

## Consequences

- The PlatformsView per-platform sub-line shows regex_filters + the auth
  header binding (topology.filters i18n key already at P17 R2).
- Key Candidates (P21) are a shell-side display convenience for manual entry,
  NOT the identification mechanism. Resin does not need a pre-registered key
  list; it auto-extracts from headers at request time.
- If the user wants true IP isolation per key, they set:
  platform (one regex endpoint) + sticky_ttl long + allocation_policy that
  prefers idle IP. Resin keeps the (Platform, Account) lease on the same
  egress IP until lease expiry, then re-binds.
