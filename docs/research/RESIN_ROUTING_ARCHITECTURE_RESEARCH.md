# Resin Routing Architecture Research (A3 + A4)

> Sources: exa web_fetch of github.com/Resinat/Resin DESIGN.md, internal/proxy/
> forward.go, internal/proxy/reverse.go, internal/routing/router.go (master
> branch, 2026-08-02). Read at the source level, not inferred.

## 1. The actual routing input tuple

Research first corrected a wrong premise from the earlier ADR-0003 PROPOSED.
Resin does NOT treat the upstream `Authorization: Bearer <sk-xxx>` value as
the Account. The routing input is the THREE-tuple:

```text
(platformName, account, targetHost)
```

- `platformName`: from forward-proxy auth (`Proxy-Authorization: Basic
  Platform.Account:PROXY_TOKEN`), or reverse-proxy URL identity segment
  (`/<token>/<Platform.Account>/https/<host>/...`), or `X-Resin-Account`
  header (HIGHEST priority, overrides URL segment). Empty -> Default platform.
- `account`: business identity (Tom / user_1 / a lane id), NOT the upstream
  API key. Empty -> random routing (no sticky lease). The
  `reverse_proxy_fixed_account_header: "Authorization"` platform config is
  only consulted when the reverse-proxy URL has NO account segment AND no
  `X-Resin-Account` header; in that case Resin pulls the Account string from
  the listed request headers. It is a fallback extraction, not the primary
  identity, and the extracted value is treated as an opaque business-account
  string, not parsed as an OpenAI key.
- `targetHost`: the upstream site (`api.openai.com`), used by the P2C score
  (TD-EWMA per-authority latency) and by `regex_filters` (Tag match, NOT host
  match — corrected: DESIGN.md says `MatchRegexs` matches the node's
  subscription Tags, not the request Host; the request Host drives TD-EWMA
  authority latency, not platform membership).

This means:

- The upstream API key (`Authorization: Bearer sk-...`) is forwarded as an
  end-to-end header and Resin does NOT inspect it for routing. It is the
  business account, platform, and target host that drive egress selection.
- For OmniRoute/litellm that send one client key to many providers, the right
  Resin setup is: platform = upstream provider pool (e.g. "openai-pool"), and
  the OmniRoute client sets `X-Resin-Account: <omniroute-client-id>` (or uses
  the URL identity segment) so Resin anchors THAT identity to one egress IP.
  The upstream OpenAI key stays in the hop-by-hop-stripped Authorization header
  and never becomes the Resin Account.

## 2. internal/routing/router.go (verbatim shape)

```go
type Router struct {
    pool          PoolAccessor
    states        *xsync.Map[string, *PlatformRoutingState]
    authorities   func() []string
    p2cWindow     func() time.Duration
    onLeaseEvent  LeaseEventFunc
    nodeTagResolver func(node.Hash) string
}

func (r *Router) RouteRequest(platName, account, target string) (RouteResult, error)
```

- `RouteRequest` resolves platform, then either `routeRandom` (account empty)
  or `routeSticky` (account present). Sticky path uses `xsync.Map.Compute`
  on the account key for lock-free atomic lease update.
- `selectLiveRandomRoute` runs P2C: randomly pick 2 candidates from the
  platform's routable view, pick the one with the lower composite score.
- Composite score (from DESIGN.md verbatim):
  - latency missing: `Score = LeaseCount`
  - latency present + policy BALANCED: `Score = (LeaseCount + 1) * Latency`
  - latency present + policy PREFER_LOW_LATENCY: `Score = Latency`
  - latency present + policy PREFER_IDLE_IP: `Score = LeaseCount`
- The three `allocation_policy` values are an if-branch inside the P2C score
  function, NOT a Go strategy interface. There is no NodePicker/Strategy
  interface to implement against.

## 3. internal/proxy/forward.go + reverse.go (identity extraction)

- Forward proxy: `authenticate(r) -> parseProxyAuthorizationCredentialV1 ->
  (token, platName, account)`. Token compared to `RESIN_PROXY_TOKEN`.
- Reverse proxy: `parsePathV1` splits `/<token>/<identity>/<protocol>/<host>/
  <rest>`; `parseV1PlatformAccountIdentity` splits identity by the first `.`
  or `:` into (platform, account).
- Account source priority (DESIGN.md): `X-Resin-Account` header > URL identity
  segment > fixed_header / account_header_rule extraction.
- end-to-end headers (incl. upstream `Authorization: Bearer sk-...`) are
  copied through; only hop-by-hop headers are stripped. So the upstream API key
  passes through Resin untouched — Resin never sees it as a routing input.

## 4. Resin API surface for the shell (A3 sync question)

The shell CAN read back, per active lease, the resolved (platform, account,
target, node_hash, egress_ip) tuple via Resin's metrics endpoints (we already
wrap `/api/v1/metrics/realtime/leases`). What the shell CANNOT do in v1.1.2:

- It cannot inject a chosen NodeHash per request (no override header/segment).
- It cannot set a per-node bandwidth/protocol weight (the P2C score is fixed
  to the three built-in policies; no per-node config field exists in the
  `platforms` table to weight an individual node).
- It cannot get per-request (api key + body.model) identity from Resin because
  Resin never parses body.model — the routing identity is (platform, account)
  and the upstream key is opaque to Resin.

This is the A3 black-box the user is pointing at: the route_id(auth, body.model,
path) helper we shipped in P24-Q3 in crates/resin-core/src/lane.rs is in fact
the CORRECT identity for the shell to track, but only if the shell sits in the
client path BEFORE Resin — i.e. the shell must rewrite the client's request to
inject `X-Resin-Account: <route_id-derived-id>` so Resin anchors egress IP per
(unique client key + upstream endpoint) pair. Resin alone will NOT do that; it
sees only whatever account the client or the shell injects.

## 5. A4 trade matrix — modify Resin source vs shell-side decoupled decision layer

Given the research, here is the honest trade-off matrix.

| Path | A4-1: fork+modify Resin Go source | A4-2: shell-side decoupled routing layer | A4-3: shell rewrites client requests to inject X-Resin-Account |
|---|---|---|---|
| What changes | Refactor `internal/routing/router.go` to a strategy interface; add Bandwidth/Sequential/ProtocolWeight strategy impls; extend `platforms.allocation_policy` enum + DB schema; rebuild Go binary | Add a Rust module that pulls `/api/v1/nodes` + health, applies bandwidth/protocol weights, picks a node, and... has no way to tell Resin which node to use | Add a thin local HTTP interceptor (between omniroute and Resin's forward/reverse proxy) that maps each (client key + body.model + path) to an `X-Resin-Account` value and injects it; Resin's existing sticky routing does the rest |
| Upstream sync on Resin upgrade | rebase fork, re-test hot path | none (uses public API only) | none (uses X-Resin-Account, a published contract) |
| Hot path perf | Go-native P2C O(1); new strategies stay in-process | extra IPC round-trip per request + node-list sync race | one HTTP rewrite hop in Rust (axum), negligible ms |
| Can it actually deliver per-node bandwidth/protocol weighting WITHOUT a Resin API that accepts per-node override? | yes (strategy lives inside Resin, sees the full node struct) | **No** — v1.1.2 has no per-node override API; the decision is made but cannot be enforced. This route is a dead end on the current Resin version. | partially — shell can bias WHICH platform/account a request lands in (→ which region/policy pool), but inside a pool Resin still picks the node via P2C. Per-node protocol weight inside one platform is not reachable this way. |
| Maintenance cost | Go fork upkeep; diverge risk | Rust module, same language as project | Rust interceptor, same language; Resin untouched |
| Fork vs upstream tension | real fork; must publish + track Resin releases | none | none |

### Recommendation skeleton (for user decision)

- A4-2 (pure shell-side node picker) is NOT viable on Resin v1.1.2 — there is
  no per-node override API, so the shell's decision has nowhere to land.
- A4-1 (modify Resin) is the ONLY path that delivers true per-node
  bandwidth/protocol weighting on the hot path, but it forks the Go upstream
  and takes on Go maintenance.
- A4-3 (shell rewrites to inject X-Resin-Account) is the ONLY path that makes
  the existing route_id(auth, model, path) shell-side identity actually drive
  Resin's sticky routing per (client key + endpoint) pair, WITHOUT touching
  Resin. This directly closes the A3 "GUI is a toy" gap: the shell becomes the
  identification layer, Resin stays the egress-IP-sticky layer, and the GUI
  shows the live lease map (platform, account, egress_ip, target) read back
  from `/api/v1/metrics/realtime/leases` so the user sees the mapping is real.

A hybrid is possible: ship A4-3 now (closes A3, no Resin fork), and if
per-node bandwidth/protocol weighting is still wanted later, do A4-1 as a
staged fork (strategy interface refactor) — but only if A4-3's per-platform
policy biasing proves insufficient in practice.

## 6. Open v1.1.2 probe facts (already verified)

- `GET /api/v1/platforms` returns the items-wrapper with `allocation_policy`
  constrained to BALANCED | PREFER_LOW_LATENCY | PREFER_IDLE_IP (probed: invalid
  value -> 400 with that exact enum string).
- No `/lanes`, `/routes`, `/keys`, `/egress`, `/config` endpoints exist
  (probed: all 404 on v1.1.2 release binary).
- `GET /api/v1/metrics/realtime/leases` returns per-platform aggregate
  active_leases; the per-account node_hash + egress_ip is in the lease items.
- `GET /api/v1/nodes` returns the node list (items-wrapper) with node_hash,
  display_tag, has_outbound, failure_count, region, tags. No per-node
  bandwidth field. No per-node protocol field beyond what the subscription
  tags encode.

## 7. ADR-0006 item 3 live-sidecar e2e findings (new)

- **Reverse-proxy path format requires the identity segment.** The Resin
  reverse-proxy surface path is `/<token>/<identity>/<protocol>/<host>/<path>`.
  Omitting the identity segment (e.g. `/<token>/https/<host>/<path>`) yields
  `400 Protocol must be http or https` because the parser walks the segment
  that should be the identity as if it were the protocol. The interceptor
  lands `Default` in the identity slot so the URL parses; the injected
  `X-Resin-Account` header takes precedence over the URL identity segment
  per DESIGN.md, so route_id still drives Account selection.
- **Leases endpoint surfaces aggregates, not per-key rows.** The live Resin
  v1.1.2 `GET /api/v1/metrics/realtime/leases` returns an items-wrapper
  where each row is `{active_leases, ts, platform_id, step_seconds}` (an
  aggregate per platform per step) and does NOT expose the per-key
  `{account, node_hash, egress_ip, target_domain}` row the GUI lease chip
  relies on. The per-key binding is an internal Go sidecar invariant by
  design; the shell's `lease_map` IPC projects account/egress_ip only when
  the upstream field is present, otherwise renders the aggregate count.
  A future Resin API change that adds per-key fields to the leases
  endpoint would let the GUI show real per-key egress chips; until then
  the chip displays the injected X-Resin-Account id's first 12 chars.
