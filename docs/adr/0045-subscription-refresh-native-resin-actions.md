# ADR-0045: Subscription refresh sink to Resin native /actions/refresh (source_type=remote)

Date: 2026-08-18
Status: ACCEPTED
Supersedes: ADR-0044 S2 (subscription refresh path only; S1/S3/S4 unchanged)
Canon: GRILL_T20_SUBSCRIPTION_REFRESH_NATIVE_PLAN.md
Fix branch: T20 (parallel to T17/T19 audit branches)

## Context

ADR-0044 S2 decided the shell should refresh a subscription by re-fetching the
remote Clash YAML itself (clash-family UA rotation), converting flow-style inline
mappings to block-style, then PATCHing `/api/v1/subscriptions/{id}` with the
new `content` + `source_type:"local"`. The rationale was two-fold, both asserted
in the P13 B4 commit comment (`src-tauri/src/commands/mod.rs` L513-519):

1. "Resin's own remote-fetch uses a default HTTP UA that many subscription
   providers reject with 403" → shell must do clash-family UA rotation.
2. "Resin's Go YAML parser chokes on flow-style inline mappings" → shell must
   rewrite flow-style to block-style before POST.

ADR-0044 S2 also asserted: "Resin `/actions/refresh` on a local source is a
no-op re-parse of in-memory content, so this shell path is necessary for real
remote updates." This last assertion is correct (verified in Resin v1.2.0
`internal/topology/subscription_scheduler.go`):
```go
if attemptSourceType == subscription.SourceTypeLocal {
    body = []byte(attemptContent)            // re-parse in-memory, zero network IO
} else {
    body, err = s.Fetcher(attemptURL)        // HTTP fetch the url
}
```

## Evidence that re-revises premises 1 and 2 (Resin v1.2.0 source, fetched 2026-08-18)

### Premise 1 (UA) is FALSE

`cmd/resin/main.go` line ~const:
```go
const downloadUserAgent = "clash.meta"
```

`internal/netutil/downloader.go`:
```go
func NewDirectDownloader(timeoutFn func() time.Duration, userAgentFn func() string) *DirectDownloader { ... }

// inside Download():
userAgent := d.currentUserAgent()
if userAgent != "" {
    req.Header.Set("User-Agent", userAgent)
}
```

Resin v1.2.0 **already** ships `User-Agent: clash.meta` on every subscription
fetch. `clash.meta` is a clash-family UA; subscription providers that 403 the
default Go UA return 200 to `clash.meta` (same UA family as the shell's
`clash-verge/v2.0.0` rotation). The P13 B4 assertion "Resin uses a default UA"
misread the source — Resin never used the default UA. Any historical 403 / 525
the shell saw on Resin's fetch was due to a different cause (most likely the
test host's CDN returning 525 for an unrelated reason), not UA.

### Premise 2 (flow-style YAML) is UNVERIFIED in v1.2.0

Resin's parser (`internal/subscription/parser.go`) uses `gopkg.in/yaml.v3`,
which **does** support flow-style inline mappings (`MappingNode` with
`Style == FlowStyle`). The P13 B4 comment "Resin's Go YAML parser chokes on
flow-style inline mappings" was likely a misdiagnosis of a different corner
case (unicode group names, or a flow-style scalar inside a block sequence
under a specific indent). Even if a real flow-style parser bug exists in
`yaml.v3`, Ponytail mental model says: let Resin parse the original remote
content and **fail loudly upstream**, so the user sees the real Resin error
and can report it. Sanitizing the YAML in the shell hides upstream bugs and
makes the shell responsible for a parser contract it does not own.

### `RefreshSubscription` semantics (also verified)

`internal/service/control_plane_subscription.go`:
```go
// RefreshSubscription triggers an immediate subscription refresh (blocks).
func (s *ControlPlaneService) RefreshSubscription(id string) error {
    sub := s.SubMgr.Lookup(id)
    if sub == nil { return notFound("subscription not found") }
    s.Scheduler.UpdateSubscription(sub)  // synchronous, blocks until diff/apply done
    return nil
}
```

`POST /api/v1/subscriptions/{id}/actions/refresh` → `RefreshSubscription(id)` →
synchronous `UpdateSubscription(sub)`. For `source_type=remote` it reimplements
`s.Fetcher(url)` (HTTP GET with `User-Agent: clash.meta`), re-parses, diffs
hashes, applies. For `source_type=local` it just re-parses the in-memory
`content` field. The endpoint is documented in DESIGN.md and has an e2e test
(`internal/api/subscription_refresh_e2e_test.go`).

## Decision

**Sink subscription refresh to Resin native `POST /api/v1/subscriptions/{id}/actions/refresh`** and migrate shell subscription storage from `source_type:"local"` to `source_type:"remote"`. This requires three coupled changes:

1. **`subscription_add` IPC**: drop the shell-side `fetch_clash_subscription` + `clash_yaml_to_proxies_block` + inline-content POST. Instead POST `{name, url, source_type:"remote", update_interval:"30s"}` directly. Resin's own DirectDownloader (User-Agent `clash.meta`) handles the remote fetch. Resin's own `yaml.v3` parser handles the YAML. The shell never touches the YAML bytes.

2. **`subscription_refresh` IPC**: drop the shell-side fetch+convert+patch chain entirely. The new command resolves `name → id` via `GET /api/v1/subscriptions` and then POSTs to `/api/v1/subscriptions/{id}/actions/refresh`. Returns the post-refresh node_count from the updated subscription object.

3. **Delete three shell helpers**: `crates/resin-core/src/resin_client.rs::fetch_clash_subscription`, `crates/resin-core/src/resin_client.rs::clash_yaml_to_proxies_block`, `crates/resin-core/src/resin_client.rs::refresh_subscription_content`. Estimated ~200 lines Rust removed (P13 B4 debt), plus their vitest and cargo unit tests where they exist.

Error mapping: `subscription_refresh` and `subscription_add` errors go through the existing `map_resin_error(status, body)` path (HTTP status from Resin). Non-HTTP fetch failures inside Resin are already converted by Resin to a 5xx with a descriptive body before returning, so the shell's `map_resin_error` never sees a status=0 body.

`subscription_refresh` returns `u64` (post-refresh node_count) unchanged; the Ui juste calls `refresh()` to repoll and shows the updated count. If Resin's refresh failed (eg 5xx upstream), the IPC surfaces the `IpcError`; the frontend `translateError` maps it.

## Rejected alternatives

1. **A-2 (local + `/actions/refresh` only)** — rejected by user: "refresh real semantic is re-pull remote url". `/actions/refresh` on local is a no-op re-parse, so A-2 cannot pull new nodes from upstream. Stale.

2. **A-3 (hybrid: try `/actions/refresh` first, fall back to shell fetch+convert+patch if local)** — rejected: doubles the test surface, keeps 200 lines of dead code as a fallback path, and the fallback's existence implies the primary path is unreliable. Ponytail: pick one path, own it.

3. **Keep `subscription_add` shell-side fetch+convert (only refresh goes native)** — rejected: refresh on a local source is a no-op re-parse of the stale content the shell fetched at import time, so the user never sees new upstream nodes. Self-defeating.

4. **Fork Resin to expose a "shell-injected UA" hook** — rejected: `NewDirectDownloader` already takes a `UserAgentFn` callback, and `cmd/resin/main.go` hardcodes `clash.meta`. If a future test host rejects `clash.meta`, we can send a one-line upstream patch or add `RESIN_DOWNLOAD_USER_AGENT` env var — no fork needed. The shell can also set it via `src-tauri/src/sidecar.rs` env injection (we already inject `RESIN_NODE_DNS_UPSTREAMS` per ADR-0042 S1).

5. **Continue supporting `source_type:"local"` import for users who paste raw YAML instead of a URL** — **NOT rejected**. This is a separate UX path (manual YAML paste). It is out of scope for T20. If a future user pastes raw YAML, we POST `{name, content, source_type:"local"}` and the refresh button on that group becomes a no-op (Resin re-parses in-memory). The T20 plan keeps the `subscription_add` IPC body-shaped so it later can branch on `url vs content`; for T20 we only wire the `url` path.

## Consequences

- **Net code reduction**: ~200 lines of P13 B4 shell fetch/convert/patch Rust deleted, plus their unit tests in `crates/resin-core/src/resin_client.rs` and `src-tauri/src/commands/mod.rs`. The shell becomes strictly a thin IPC forwarder on the subscription path — consistent with how platforms/nodes/leases already work (ADR-0042, ADR-0044 S3).
- **Mental model alignment**: refresh = "re-pull remote url + re-parse" now holds end-to-end. Resin's `yaml.v3` parser becomes the single source of YAML truth. If a provider returns flow-style YAML that `yaml.v3` genuinely cannot parse, the shell surfaces the real Resin 400/500 error to the user — no more silent shell-side sanitization hiding upstream bugs.
- **UA resilience**: Resin's `clash.meta` UA is an upstream constant; if a site ever rejects it, the fix is a one-line upstream change or a `RESIN_DOWNLOAD_USER_AGENT` env var injected from `sidecar.rs` (same pattern as `RESIN_NODE_DNS_UPSTREAMS`). The shell no longer carries a 4-UA rotation list.
- **Existing local subscriptions in the user's state.db** will need a one-time reconciliation: their `source_type` is `local` and they have no `url`, so `/actions/refresh` on them is a no-op. T20 migration step: on first `subscription_refresh` failure (Resin returns 400 "remote subscription requires url") the shell surfaces the error and instructs the user to re-import. We do NOT silently re-POST as remote because that would re-fetch a url we don't have for legacy local subs.
- `subscriptionRefresh` CONTEXT.md term description changes from "shell re-fetches, converts, PATCHes" to "shell POSTs `/actions/refresh`; Resin re-pulls remote url, re-parses, diffs, applies". The term stays in CONTEXT.md with an updated definition pointing at ADR-0045.
- ADR-0044 S2 is **superseded for the refresh path only**. ADR-0044 S1/S3/S4 (collapse, probe, whitebox knobs) are untouched and still authoritative.
