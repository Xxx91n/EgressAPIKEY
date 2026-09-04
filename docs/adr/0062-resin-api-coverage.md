# ADR-0062: Resin API coverage mapping (five-bucket consumption ledger)

- **Status**: ACCEPTED
- **Date**: 2026-09-04 (round5 T13 / crack #2)
- **Companion doc**: [RESIN_API_COVERAGE.md](../architecture/RESIN_API_COVERAGE.md) (the row set this ADR legislates)

## Context

Upstream Resin registers 57 authenticated control-plane routes plus unauthenticated
`/healthz` (`resin/internal/api/server.go:65-157`, verified against
`resin/cmd/resin/main.go` wiring). The shell-side `ResinClient`
(`crates/resin-core/src/resin_client.rs`) wraps 28 endpoint-hitting public methods, of
which 23 are in active use and 5 are orphans; 29 further routes have no client method at
all. Until T13 there was no document listing this surface, so "why is endpoint X not
wired" was unanswerable from the repo (round5 crack #2, reports/03 §2 #2).

## Decision

**D1 — The coverage table is the single row set.** `docs/architecture/RESIN_API_COVERAGE.md`
holds exactly one row per upstream control-plane route (58 rows: 1 healthz + 57 authed),
each with: HTTP method + path, Resin handler file:line, ResinClient method (resin_client.rs
line), IPC command / consumer, five-bucket status, doc location, introduced version, and a
remark. Row ids `R01..R58` are stable anchors; ResinClient public endpoint methods carry
`/// @see docs/architecture/RESIN_API_COVERAGE.md #R<NN>` in their doc comments.

**D2 — Five-bucket semantics (legislated).**

1. 已对接 wired — a ResinClient method exists AND is called by shell code (an IPC command
   in `src-tauri/src/commands/` or a resin-core service such as `strategy_service::apply`).
2. 装饰 decorative — an IPC command is defined + registered + manifest-listed but its body
   does not actually call the upstream endpoint it is named after (a "fake" wire). The
   bucket is EMPTY as of T13: reports/03 §2 #3 claimed `system_config_get/patch` were
   decorative, but the bodies have called `client.system_config_*` since T8-1 (856a0ff).
   T15 must re-verify its premise against the table instead of trusting the survey.
3. 孤儿 orphan — a ResinClient method exists with zero external callers (client unit tests
   only). Current orphans: `get_platform` (R10), `get_endpoint` (R17), `get_request_log`
   (R45), `get_request_log_payloads` (R46), plus the `list_nodes_for_platform` variant
   method on the already-wired R36.
4. 空白 blank — no client method, no IPC command; upstream-only surface (today: 29 rows).
5. 故意不接 deliberate — unwired BY DECISION with the reason recorded here or in the
   cited ticket. Deliberate-ness requires a written rationale, not absence of code.

**D3 — Deliberate non-adoption list (with reasons).**

- `GET /system/config/default` (R04) — factory defaults are Resin-internal; the shell has
  no UI to display immutable values.
- `GET /system/config/env` (R05) — the env block is injected by `boot_resin` itself
  (including `RESIN_ADMIN_TOKEN`); reading it back adds no information and must never
  cross toward the webview (AGENTS.md §7.6 server-side trust).
- `DELETE /platforms/{id}/leases` (R21) — bulk lease revocation is destructive and has no
  UI; the equivalent "close everything" semantic is `close_all_connections`, which chose
  a sidecar restart (settings.rs:98) precisely because Resin v1.2.0 has no global
  close-all API. Single-lease rows R22/R23 stay 空白 (candidates, not decisions).
- The three platform actions `preview-filter` (R09), `reset-to-default` (R13),
  `rebuild-routable-view` (R14) are deliberate non-adoptions per
  [ADR-0066](0066-platform-actions-not-integrated.md) (classified by round5 T22):
  reset-to-default would rewrite L3 platform config from env defaults behind the
  L2 whitebox authority; rebuild-routable-view re-derives Resin-internal pool
  state that ADR-0057 diff-then-skip already keeps converged; preview-filter
  (read-only dry-run) duplicates the ADR-0054 shell-side in-memory snapshot and
  stays the one sanctioned wiring candidate for a later ticket (T23).
- The four GeoIP endpoints `GET /geoip/status` (R40), `GET|POST /geoip/lookup`
  (R41/R42), `POST /geoip/actions/update-now` (R43) are deliberate
  non-adoptions per [ADR-0065](0065-geoip-provider-selection.md) (classified
  by round5 T20): they return a pure geographic region with no fraud/abuse
  dimension, while the shell's IP-reputation need is served by third-party
  providers (`crates/resin-core/src/ip_reputation.rs`, L1 keys in
  `settings.json`).

**D4 — Next-round wiring roadmap (shell side, not an upstream upgrade list).** T16
(account-header-rules CRUD ×4: R32-R35), T19 (12 metrics endpoints: R47-R58 except the
wired R49/R56), T20 (GeoIP rationale ADR: R40-R43), T21 (expose R45/R46 + R22 detail),
T17 (resolve or delete the 5 orphan methods). T22 classified R09/R13/R14 as
deliberate non-adoptions (ADR-0066), closing its roadmap entry. T20 documented the GeoIP selection
([ADR-0065](0065-geoip-provider-selection.md)), flipping R40-R43
空白 → 故意不接 and closing its roadmap entry. This section
is the "what to wire next" ledger; it deliberately does NOT track upstream Resin API
changes (that is outside T13 scope per the issue 不做清单).

**D5 — Maintenance discipline.** Any commit that adds, removes, or reshapes an upstream
route (vendored `resin/` change), a `ResinClient` public method, or an IPC command that
consumes one of these endpoints MUST update the coverage row(s) and this ADR in the same
commit; `verify-build.sh` + markdownlint remain the gates. Bumping the bundled sidecar
version in `docs/RESIN_UPSTREAM_MANIFEST.yaml` requires re-auditing the row set against
the new binary (route strings / contract tests) before landing. Doc-comment `@see` lines
in `resin_client.rs` are part of the lockstep: adding a client method without a row
anchor (or vice versa) is a review blocker.

**D6 — Refresh-completion hook shape (spec D-34 output).** Resin exposes no webhook, no
event push, and no refresh-completion callback (`cmd/resin/main.go` wires bootstrap only;
grep over `resin/internal/api/*.go` + `main.go` for webhook/callback/notify: zero hits).
`POST /api/v1/subscriptions/{id}/actions/refresh` is synchronous and returns
`{"status":"ok"}` with no `changed` flag in v1.2.0. Consequence for T01/T14: the refresh
hook form is "synchronous POST return + poll `GET /subscriptions` for field deltas" (the
round5-T03 30s edge poll), and the G4 narrative must describe poll-based observation,
not push. The `changed` output extension from spec D-24 is a Resin-side contract change
and is out of T13 scope.

**D7 — Out-of-table surfaces.** The embedded WebUI routes (`/`, `/ui`, `/ui/`,
`webui.go`) and the data-plane token action `POST
/{proxy_token}/api/v1/{platform}/actions/inherit-lease` (`handler_token_action.go:22`,
mounted via `cmd/resin/inbound_mux.go`) are NOT part of the 58-row control-plane table.
`ResinClient` must never construct `/{token}/api/v1/...` paths; the loopback + admin
token guard stays the only trust seam.

## Consequences

- Positive: any agent can answer "is endpoint X wired, and why / why not" in under five
  minutes; new IPC work starts from a bucket decision instead of an ad-hoc grep.
- Cost: the table must move with every endpoint-affecting commit (D5); drift is caught by
  review, not by a machine check (the row set spans Go + Rust + TS, so automating full
  lockstep is out of scope for this round).
- Neutral: bucket counts are a point-in-time audit stamp (2026-09-04), not an invariant;
  the semantics (D2) are the invariant.

