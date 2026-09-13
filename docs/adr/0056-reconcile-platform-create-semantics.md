# ADR-0056: reconcile platform-create semantics — apply fulfills the preview's "will be established" promise

Status: ACCEPTED
> Date: 2026-09-02
> Ticket: architecture-recovery 22 (b1-reconcile-platform-semantics) — checkpoint A.
> Extends (reopens none): ADR-0054 §A (one-way reconcile), ADR-0036
> (strategy whitebox single write entry), ADR-0051 (authoritative snapshot
> read model). Refines the auto-clean behavior inherited from the ticket-10
> StrategyService deep module (ADR-0052).

## Context

The reconcile preview (`compute_reconcile_plan`,
`crates/resin-core/src/strategy_service.rs`) reports a whitebox platform
absent from the Resin runtime as `action: "create_platform"`, rendered in
the Effective Config view with the copy "exists only in the desired state —
will be established on Resin" (locale key
`effectiveConfig.reconcileCreatePlatform`, all 18 locales). The actual apply
path contradicted that promise: `StrategyService::apply` collected
`live_names` from `GET /api/v1/platforms`, passed them to `clean_stale`,
which DELETED the very same missing-on-resin platform entries from the
whitebox (persisting the cleaned document via
`persist_cleaned_if_changed`), and the per-platform PATCH loop then reported
`"platform not found"` for names that had already been wiped from the plan's
input. A user who saw "will be established" and clicked Apply watched the
platform disappear from the whitebox instead — the last active
say/do contradiction on the authority chain (Round 4 spec problem
statement, ticket 22).

This is not a merge-semantics change: the three-state snapshot of ADR-0051,
the one-way direction of ADR-0054 §A (L2 whitebox always wins; no L3->L2
write path), and the single write entry of ADR-0036 are all untouched.

## Decision

**Fulfill the promise (primary route).** When `StrategyService::apply`
finds a whitebox platform whose name is absent from Resin's live platform
list, it now creates the platform on Resin through the existing
`ResinClient::create_platform_from_name` seam (`POST /api/v1/platforms`,
body `{"name": ...}`, only `name` required by Resin v1.2.0 — the seam is
already exercised in production paths `platform_add` and the backup-restore
flow, and pinned by the mockito test
`mockito_create_platform_happy_path`), then continues the normal flow for
that platform: the per-platform PATCH loop finds it on the next
`list_platforms` read and PATCHes its computed `region_filters`. The
whitebox entry survives — the reconcile preview's "will be established"
becomes a true statement.

- **Apply never deletes whitebox intent.** With creation handling every
  missing-on-resin platform, no whitebox platform is "stale" during apply
  anymore, so the `clean_stale` call and its persist-cleaned path
  (`persist_cleaned_if_changed`) are removed from `apply`. Platform removal
  happens by editing the whitebox — the one-way model's only sanctioned way
  to change desired state. `clean_stale` itself stays exported and pure
  (its disposition belongs to the Round 4 dead-code sweep, tickets 24/26;
  recorded in the ticket report).
- **Failure is honest, not silent.** If the create attempt fails (e.g. a
  name Resin rejects, a transient 5xx), the entry's `AppliedPlatform`
  reports `patched: false` with the create failure in `reason` (the
  per-platform non-fatal reporting contract is unchanged) and the whitebox
  entry KEEPS its place: the next apply retries the create. The user sees
  in the report why the platform is not on Resin and can fix the desired
  state in the editor — a failed create never turns into the reverse
  "said establish, actually deleted" action.
- **One-way direction preserved.** Creation is L2->L3 only. The creation
  path is exactly the same `ResinClient` REST seam every other L2->L3
  assert uses (`strategy_apply` PATCH, `restore_ports_from_whitebox` POST);
  no L3->L2 write is introduced.
- **Preview/apply lockstep.** The preview's `create_platform` action and
  apply's create path read the same live-platform list through the same
  Resin seam, so the same platform cannot be promised creation and receive
  a cleanup: for a sidecar-down or POST-failing Resin, apply reports the
  failed create in its per-platform report while the whitebox entry
  survives — the visible difference is a surfaced per-platform error, never
  a reverse action on the desired state.

## Alternatives considered

- **Downgrade route (change the 18-locale copy to cleanup semantics):**
  rejected — the spec's user story 1/2 explicitly prefers fulfillment, the
  upstream constraint that would have forced the downgrade (platform
  creation requiring node/policy payloads) does not exist (Resin v1.2.0
  POST /platforms accepts name-only), and rewriting the copy to "will be
  cleaned" would legitimize apply silently deleting user-authored whitebox
  intent for a merely-temporarily-down sidecar — strictly worse for the
  one-way authority story.
- **Create with full payload (regions/allocation_policy inline):** rejected
  — duplicates the PATCH loop's job; the name-only create + follow-up PATCH
  reuses the existing seam pair and keeps one code path that computes
  region_filters.

## Consequences

- `StrategyService::apply` performs one extra POST per newly-created
  platform; the per-platform PATCH loop already re-reads `list_platforms`
  before each platform, so the created row (and its id) is found there —
  no extra read is added.
- `clean_stale`'s disposition: the function and its three unit tests stay
  in place unchanged (still exported via lib.rs) — deleting them belongs
  to the dead-code sweep tickets (24/26), not this ticket; apply simply no
  longer calls it.
- The Effective Config view and its locale copy are unchanged — the promise
  becomes true without a wording change.
- IPC surface unchanged (still 71 commands); no new config layer, no new
  abstraction (ponytail-full: the create seam already existed).

## Verification

- Behavior-locking mockito tests in `crates/resin-core/src/strategy_service.rs`
  (existing test mod, no new seam): apply on a missing-on-resin platform
  POSTs `/api/v1/platforms` with `{"name": ...}` and then PATCHes its
  region_filters; the whitebox entry survives untouched; a failed create is
  reported per-platform with the entry still present in the whitebox.
- `crates/resin-core/tests/reconcile.rs` continues to pass: the twice-pass
  idempotency scenario is unaffected (its platform is live in Resin, so no
  create path is involved).
- Full gates: cargo test (resin-core + shell), vitest, tsc, i18n 435×18,
  ipc-manifest 71, vitest-isolation-guard, verify-build, P23 build
  closure (chunk hash in staged exe + smoke TITLE=EgressAPIKEY).
