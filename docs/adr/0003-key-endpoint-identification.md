# ADR-0003: identity = (platform, account, targetHost) in Resin; shell injects account from (auth, body.model, path)

Date: 2026-08-02 (revised twice: first PROPOSED wrong, then ACCEPTED three-tuple, now corrected again after source-level Resin research)
Status: SUPERSEDED by ADR-0012 (route correction — thin-shell multi-port forwarder, port = identity; decision body kept for history)
Decision Type: Domain contract + shell-side identification + shell-side account injection

## Context

User flagged the Q3 identification work as black-box / toy: the route_id
helper in crates/resin-core/src/lane.rs had NO caller wired into the GUI or
into the Resin IPC surface, and there was no closed loop showing the
identification actually drives egress selection. User also asked for a source-
level trade study for A4 (modify Resin vs shell-side decoupled decision layer).

## Source-level research findings (Resin master, 2026-08-02)

Read at source: DESIGN.md, internal/proxy/forward.go, internal/proxy/reverse.go,
internal/routing/router.go.

1. Resin routing input is `(platformName, account, targetHost)`. The upstream
   `Authorization: Bearer <sk-xxx>` is an end-to-end header Resin copies
   through and does NOT inspect for routing. The earlier PROPOSED premise
   ("reverse_proxy_fixed_account_header: Authorization makes Resin use the
   OpenAI key as the Account") is WRONG. The fixed_account_header is a fallback
   extractor used only when the reverse-proxy URL has no account segment and
   no X-Resin-Account header; the extracted value is treated as an opaque
   business account string.
2. `account` is a business identity (Tom / user_1), not the upstream API key.
   Source priority: X-Resin-Account header > URL identity segment > fixed_header
   / account_header_rule.
3. allocation_policy (BALANCED | PREFER_LOW_LATENCY | PREFER_IDLE_IP) is an
   if-branch inside the P2C composite score, NOT a Go strategy interface. There
   is no NodePicker/Strategy interface to implement against.
4. Resin v1.1.2 has NO per-node override API. The shell cannot tell Resin
   "route this specific request to node hash X". Resin picks the node inside
   the platform pool via P2C.

## Decision (corrected)

The shell is the identification layer; Resin stays the egress-IP-sticky layer.
The unique identity the shell tracks per request is:

```
route_id = hash(normalize_auth(authorization_value), body.model, request.path)
```

This helper already exists in crates/resin-core/src/lane.rs (P24-Q3) with 8
unit tests. The fix for the toy/black-box gap is to WIRE it: the shell sits in
the client path and rewrites each request to inject
`X-Resin-Account: <route_id-derived-id>` (or the URL identity segment), so
Resin anchors egress IP per (unique client key + upstream endpoint) pair. The
GUI then reads back the live lease map from
`GET /api/v1/metrics/realtime/leases` (platform, account, egress_ip, target)
and renders it on the topology canvas, so the user SEES the mapping is real.

This is A4-3 in the trade matrix (see docs/RESIN_ROUTING_ARCHITECTURE_RESEARCH.md
section 5). It needs NO Resin source modification.

## What this is NOT

- It does NOT rely on Resin parsing body.model. Resin never sees body.model;
  the shell does, before the rewrite.
- It does NOT require a per-node override API. Resin's existing sticky routing
  on the injected account is the mechanism; per-node selection inside a
  platform stays Resin's P2C job.
- It is NOT a shell-side node picker. A4-2 (pure shell decision layer) is a
  dead end on v1.1.2 because the chosen node has nowhere to land.

## Closed-loop tests already in place

- crates/resin-core/src/lane.rs: route_id + normalize_auth (8 tests,
  `cargo test -p resin-core --lib` green).

## Still-to-build closed loop (the work this ADR authorizes next)

- A local HTTP interceptor (axum) between omniroute/litellm and Resin that:
  1. parses Authorization + JSON body.model + path,
  2. calls route_id + normalize_auth,
  3. injects X-Resin-Account: <stable id derived from route_id>,
  4. forwards to Resin reverse-proxy.
- A GUI view that reads /api/v1/metrics/realtime/leases and shows the live
  (platform, account, egress_ip, target) tuple so the mapping is observable.
- A vitest + cargo integration test that asserts: send a request with (sk-A,
  gpt-5.6) vs (sk-A, claude-sonnet-5) -> two distinct X-Resin-Account values ->
  two distinct lease entries -> two distinct egress IPs (when pool allows).

## A4 follow-up

Per-node bandwidth / protocol weighting on the hot path requires modifying
Resin's Go source (refactor allocation_policy into a strategy interface). This
is A4-1 and is a real Go fork. Defer until A4-3 proves insufficient in
practice; the trade matrix in docs/RESIN_ROUTING_ARCHITECTURE_RESEARCH.md
section 5 is the decision record.
