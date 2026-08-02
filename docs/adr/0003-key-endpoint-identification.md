# ADR-0003: key+endpoint identification = three-tuple (auth + body.model + path)

Date: 2026-08-02 (revised 2026-08-02 after 1mcp web research)
Status: ACCEPTED (supersedes the original PROPOSED which assumed Resin-native was sufficient)
Decision Type: Domain contract + shell-side identification helper

## Context

User requirement: identify each stream's (api_key + upstream v1 endpoint)
combination uniquely, millisecond-level, exact, no false positives. The user
flagged the existing implementation as a black box with no closed-loop test,
and asked for web research into the literal request-header formats used by
real upstream providers (NVIDIA build GLM-5.2, OmniRoute, OpenAI, Anthropic,
Azure) before committing to an identification mechanism.

## Research findings (1mcp perplexity + exa, 2026-08-02)

Surveyed literal request headers across providers + the OmniRoute gateway
source (docs/architecture/AUTHZ_GUIDE.md, src/sse/handlers/chat.ts):

- OpenAI: `Authorization: Bearer sk-xxx` (OpenAI compatible).
- NVIDIA build.nvidia.com GLM-5.2: OpenAI SDK with
  `base_url=https://integrate.api.nvidia.com/v1` + `Authorization: Bearer $NVIDIA_API_KEY`.
  So NVIDIA is OpenAI-compatible and uses the standard Bearer scheme.
- Anthropic: `x-api-key: <key>` + `anthropic-version: 2023-06-01` (NOT Bearer,
  though some hybrid proxies also send a Bearer at the same time).
- Azure OpenAI: `api-key: <key>` (custom header, not Bearer).
- OmniRoute: client calls OmniRoute at `/v1/chat/completions` with a Bearer;
  OmniRoute extracts the key via `extractApiKey()`, then resolves the upstream
  `(provider, model)` from the JSON body `model` field (e.g.
  `openai/gpt-5.6`), then emits trusted internal headers
  `x-omniroute-auth-{kind,id,label,scopes}` to downstream. The Authorization
  value is the **gateway-side** key; the upstream target is **not** in that
  header, it is in the body.

Key conclusion: the Authorization header value is the **client** identity, not
the **upstream endpoint** identity. A single gateway-side key legitimately
reaches many upstream endpoints (OmniRoute: one client key -> 290+ providers).
Relying on Authorization alone aliases distinct (key, endpoint) pairs into one
Account, breaking the per-(key,endpoint) IP isolation the project is built for.

## Decision (corrected)

The unique identity for a (api_key + upstream v1 endpoint) combination is the
**three-tuple**:

```
route_id = hash( normalize_auth(authorization_value), body.model, request.path )
```

- `normalize_auth` strips the auth scheme prefix (Bearer/bearer/x-api-key/
  api-key/Ocp-Apim-Subscription-Key) and ASCII-lowercases it, so the same key
  presented under different schemes by different SDKs collapses to one
  identity.
- `body.model` is the OpenAI-compatible JSON body `model` field (the routing
  target, e.g. `openai/gpt-5.6`, `claude-sonnet-5`). This is what OmniRoute
  and litellm actually use to pick the upstream provider.
- `request.path` is the upstream path tail (`/v1/chat/completions` vs
  `/v1/responses` vs `/v1/messages`).

Implementation: `crates/resin-core/src/lane.rs` exports
`pub fn route_id(auth_value, body_model, request_path) -> u64` and
`pub fn normalize_auth(raw) -> String`. FxHash, stable for the same triple,
distinct for any differing component. Sub-millisecond (single hash over three
short strings). The shell uses this as the display identity; Resin's own
Account string stays auth-value-only internally.

## What this is NOT

- It does NOT replace Resin's sticky-session machinery. Resin still owns the
  in-process token->account->IP mapping; `route_id` is the shell-side
  composition that keeps the (key, model, path) three-tuple honest so the GUI
  canvas can show distinct route identities per (key, endpoint) even when
  Resin's auth-only Account would alias them.
- It does NOT require packet deep inspection. All three inputs (Authorization
  header, JSON body, request path) are available at the HTTP boundary without
  TLS termination of the upstream.
- It is NOT crypto-secure. FxHash is a non-cryptographic hash chosen for
  speed; the threat is collision-induced misrouting inside one host, not a
  preimage attack across hosts.

## Closed-loop tests

`cargo test -p resin-core --lib lane` = 27 pass, including:
- route_id_is_idempotent_for_same_triple
- route_id_distinct_for_different_key_same_endpoint
- route_id_distinct_for_same_key_different_model   (the Resin-native gap)
- route_id_distinct_for_same_key_same_model_different_path
- normalize_auth_makes_route_id_scheme_invariant  (Bearer vs bare collapse)

## Consequences

- PlatformsView key candidates and the Topology canvas B-column should display
  the composite (auth, model, path) identity, not auth alone. A later phase
  wires route_id into the canvas node id so dragging a (key+model) to an IP
  channel is a distinct binding from dragging (key, different-model) to the
  same channel.
- Key Candidates (P21) are now grounded: the int32 display hash of
  (endpoint, apiKey) the GUI uses is the public face of this same tuple; the
  real identity used for routing is route_id(auth, model, path).
