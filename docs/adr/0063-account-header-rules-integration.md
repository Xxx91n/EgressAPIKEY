# ADR-0063: account-header-rules integration — coexistence with process routes

> Status: ACCEPTED (2026-09-05, round5-config-authority ticket 16)
> Extends (reopens none): ADR-0055 (process routes as an L2 whitebox field),
> ADR-0062 (Resin API coverage ledger: R32-R35 wiring roadmap).
> Legislation: D-35 coexistence, not replacement.

## Context

Resin exposes four account-header-rules control-plane endpoints
(`resin/internal/api/handler_rules.go:40-122`, registered at
`server.go:112-116`): list / upsert / resolve / delete. They configure the
reverse-proxy data plane's third-phase account resolution: when a platform's
`empty_account_behavior` is `ACCOUNT_HEADER_RULE` and neither the
`X-Resin-Account` header nor a path account is present, the matcher picks
the longest-prefix rule for (host, path) and Resin extracts the account
identity from the first listed request header that carries a value
(`reverse.go:495-504`), then routes to that account's sticky lease.

Before this ticket the shell had zero encapsulation and zero documentation
of that surface (coverage rows R32-R35 were 空白), while the shell-side
`process_route_*` family (ADR-0055) covers a DIFFERENT axis: OS process
name → entry port. The two look like competitors ("both route traffic by a
key") but operate at different layers with different dynamism, so the round5
spec (D-35) mandated a comparison doc and an explicit coexistence decision.

## Decision

### D1. Coexist, never replace

`process_route_*` stays exactly as ADR-0055 legislated (L2 whitebox field,
single write entry, snapshot/reconcile/acknowledgement machinery). The
account-header-rules family adds a PARALLEL, disjoint mechanism. Nothing is
deleted, migrated, or aliased between them. The six-dimension comparison
lives in
[PROCESS_ROUTE_VS_HEADER_RULES.md](../architecture/PROCESS_ROUTE_VS_HEADER_RULES.md).

### D2. Scope: four endpoints, thin facades

- `resin_core::ResinClient` gains `list_account_header_rules(keyword)`,
  `put_account_header_rules(url_prefix, headers)`,
  `resolve_account_header_rule(url)`,
  `delete_account_header_rule(url_prefix)` (R32-R35). The url_prefix is
  path-carried and percent-encoded with encodeURIComponent semantics
  (`encode_uri_component` in resin_client.rs) — `/` MUST become `%2F`
  so the Go 1.22 ServeMux `{prefix...}` wildcard receives one segment;
  the bundled WebUI rules client is the proven-good reference.
- Four IPC commands in `commands/platform.rs` are thin L3 pass-throughs:
  NO L2 whitebox file, NO snapshot field, NO reconcile action, NO backup
  ring involvement (the rules live in Resin's own state, rebuildable at any
  time by re-PUT; they are not config authority material).
- TS wrappers `ipcListAccountHeaderRules` / `ipcPutAccountHeaderRules` /
  `ipcResolveAccountHeaderRule` / `ipcDeleteAccountHeaderRule` in
  `src/lib/ipc.ts` validate-then-invoke (§7.5).

### D3. Input validation contract (§7.5)

`url_prefix` and each header name: 1..253 chars, no NUL/control characters
(DNS-host-style cap; Resin additionally enforces RFC 7230 token validity for
header names and non-empty normalization for prefixes — the shell mirrors
the cheap checks so garbage never leaves the process). `resolve` URL:
1..2048 chars, absolute http(s) scheme checked on both the TS and Rust
boundaries. `headers` array: 1..64 entries. `keyword`: <=253 chars.

### D4. GUI exposure: IPC-only in this round

No view, no tab, no i18n keys: the family is reachable from the devconsole /
future UI. F5's "GUI 暴露最小集" is satisfied by the four TS wrappers;
a ProcessRoutesView tab remains future work and is NOT legislated as a
promise.

### D5. Semantics correction recorded

The research sketch (reports/03 §2 #4, issue F3) called the rule headers an
"X-Resin-Account value" mechanism. Upstream source shows the rule's
`headers` array holds header NAMES from which the account is extracted;
`X-Resin-Account` itself is phase 1 and is stripped before forwarding. The
comparison doc carries the corrected semantics.

## Consequences

- IPC manifest 70 → 74 (four additions; no renames/removals). The
  ipc-manifest block + this ADR + the coverage rows updated in the same
  commit (ADR-0062 D5 discipline).
- Coverage buckets move: 已对接 22 → 26, 空白 29 → 25.
- resin-core test count +8 (four methods × happy/edge mockito cases).
- The rules are shell-invisible to snapshot/reconcile BY DESIGN: they are
  L3 control-plane state, not one of the three config-authority layers.
  A future round may surface them in Effective Config as a read-only
  section; that would be a new decision, not this one.

## Verification (ticket 16)

- 8 mockito unit tests in `resin_client.rs` cover list (paging +
  keyword), put (201 created + 400 invalid header), resolve (matched +
  no-match empty), delete (204 Null + 404 not found), each asserting the
  encoded path (`api.example.com%2Fv1`) and the Bearer header.
- `pnpm ipc:check`: manifest equals the command set + registry (74+).
- CI-only mandate (2026-09-04): local build/test runs are forbidden in this
  window; the gate evidence is the CI run of verify-build.sh on the pushed
  branch.
