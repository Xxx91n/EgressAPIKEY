# ADR-0057: strategy apply idempotency — diff-then-skip (apply PATCHes only real drift)

> Status: ACCEPTED
> Date: 2026-09-03
> Ticket: architecture-recovery 36 (apply-idempotency-align) — checkpoint B.
> Extends (reopens none): ADR-0054 §A (one-way reconcile), ADR-0056
> (apply fulfills the preview's establish promise), ADR-0051
> (authoritative snapshot read model), ADR-0036 (strategy whitebox single
> write entry).

## Context

`StrategyService::apply` re-PATCHed `region_filters` for EVERY whitebox
platform found on Resin on every invocation, with no comparison against the
live row. Consequence: the reconcile pass's "zero changes on pass 2" hard
gate (architecture-recovery 14, ADR-0054 §A) held only for the ports half
(ReconcileMemory TTL window); the strategy half re-asserted every time —
effect-idempotent (the final state converges) but not write-idempotent
(redundant PATCH requests each pass). `crates/resin-core/tests/reconcile.rs`
documented this asymmetry in its comments while the file header still
promised zero changes for both halves.

Checkpoint B (WORKFLOW §5 ADR 拍板) offered: (A) document the re-assert
semantics and fix the "zero changes" wording, or (B) implement change
detection. The user directed the decision through an atomcode-research
dispatch (2026-09-03, ticket 36 checkpoint B; 23+ searches across Exa /
Tavily / AnySearch, 13 full-text reads, all five evidence angles; the same
mental-model lineage as ticket 21/37). Research verdict, with sources:

- Mature declarative systems are **layered no-op**: a diff-then-skip gate at
  the trigger/execution layer over a write layer that may also absorb
  no-ops. ArgoCD syncs only OutOfSync applications (auto-sync "will only be
  performed if the application is OutOfSync"; ApplyOutOfSyncOnly exists
  because per-object re-apply "puts undue pressure on the Kubernetes API
  server"). kubectl apply computes a 3-way merge diff and sends only
  differing fields. Terraform's empty plan means apply does nothing.
  Ansible/Puppet/Chef modules check current state first and report
  changed=0 when converged (Chef's REST resource docs state it verbatim:
  "If it exists and nothing changed: takes no action").
- Re-asserting identical content through a backend WITHOUT a no-op
  absorption layer has real, demonstrated cost: Kubernetes
  issues #131175 / #124050 / #124605 (fixed v1.31 via #125317, etcd
  short-circuit PR #67562) show every no-op SSA became a true write —
  resourceVersion bump, watch MODIFIED event, downstream infinite
  reconcile loops.
- The Resin sidecar REST has no no-op absorption (no resourceVersion, no
  CAS, no field ownership). With such a backend, the diff gate must live
  at the caller — which is exactly what this ADR does.
- TOCTOU exposure of a fresh-read diff is structurally bounded here:
  reconcile is an explicit low-frequency user action (no background loop —
  ADR-0054 rejected auto-heal), the comparison uses the SAME fresh
  list_platforms read the PATCH path already performs (no stale/TTL-skip on
  the strategy half), and the automatic post-reconcile snapshot re-check
  catches any read-to-write window drift as a new divergent badge.

## Decision

**Diff-then-skip (option B).** In the apply PATCH loop, after resolving the
platform id from the fresh per-platform `list_platforms` read, the computed
region set is compared against the live row's `region_filters` with the
SAME order-insensitive, case-insensitive, duplicate-collapsing set rule the
three-state merge and the reconcile preview use (`same_region_set`,
shared with snapshot.rs's merge semantics):

- Equal → the live row IS the desired state; the PATCH is skipped and the
  platform is reported **converged**: `patched: true` with reason
  `"in sync"`. The success/failure buckets the GUI counts (SettingsView
  "applied N platforms, M errors") are unchanged — a skip is never an
  error. The serde wire shape of `AppliedPlatform`/`ApplyReport` is
  untouched (TS contract unchanged, §7.6 manifest untouched).
- Not equal / live `region_filters` absent or unparseable → PATCH
  (conservative re-assert). Missing-on-resin create path (ADR-0056) is
  unchanged; a freshly created row still receives its region PATCH.
- Failed PATCH still reports per-platform and the whitebox entry survives
  (ADR-0056 contract); the next reconcile retries it — re-assert remains
  the failure-path fallback, now only for REAL drift.
- The strategy half gains NO TTL memory: the skip is always computed from
  the current pass's fresh read, never from remembered state (this is the
  deliberate difference from the ports half's ReconcileMemory, whose TTL
  semantics stay as ADR-0054 §A shipped them).

With this, the reconcile pass is wire-idempotent end to end: a second pass
over a converged Resin emits zero write requests on both halves, by two
different mechanisms that must not be conflated in wording (strategy =
diff-then-skip; ports = TTL window).

## Consequences

- Redundant strategy PATCHes disappear: reconcile/apply traffic drops from
  O(platforms) writes per pass to O(drifted platforms).
- The integration gate in `tests/reconcile.rs` becomes real: the fixture
  now serves the drifted row in pass 1 and the synced row in pass 2
  (two phase mocks, mockito registration order), `m_patch` is
  `expect(1)` and ASSERTED — a second-pass re-PATCH fails the test. The
  apply unit tests in `strategy_service.rs` lock the skip itself
  (`apply_in_sync_platform_skips_patch`: in-sync platform + zero-write
  mock at `expect(0)`; `apply_twice_second_pass_zero_patches`: two-pass
  run with PATCH capped at exactly the pass-1 write), including the
  case-insensitive comparison ([`"hk"`] live vs ["HK"] computed) and the
  converged-report contract.
- Stale wording fixed at every site that over-promised or under-described:
  the module/apply/reconcile doc comments, `ReconcileMemory`'s "naturally
  idempotent" claim (strategy half is now wire-idempotent via diff-then-
  skip; the claim "PATCHing the computed plan twice is a no-op" is gone),
  reconcile.rs comments (the former "rebuild the mocks for a synced world"
  narrative that was never implemented), ARCHITECTURE.md (strategy
  pipeline + reconcile sections), CONTEXT.md Reconcile term, AGENTS.md
  strategy_apply clause, CHANGELOG entry.
- IPC surface, config layers, whitebox write-entry discipline, ports-half
  logic: all unchanged (ponytail — no new abstraction; the diff reuses the
  existing `parse_resin_platforms` parser and `same_region_set` rule).
- Cost asymmetry (research Q4): one set comparison per platform on an
  already-fetched read vs a full PATCH pipeline per platform per pass.

## Verification

- `cargo test -p resin-core`: 197 lib (2 new ADR-0057 behavior locks) +
  3 integration + 3 proxy_e2e (+1 ignored live probe) + 2 reconcile, all
  green; the reconcile.rs `reconcile_twice_second_pass_zero_changes` now
  ENFORCES the once-per-scenario PATCH at the wire level.
- `cargo test -p egressapikey-app --lib`: 94 passed (shell report
  shape/serialization untouched).
- Gates: markdownlint clean on all touched docs; `git diff --check`
  clean; AGENTS §5 build closure (fresh exe staged with the new
  resin-core, Vite chunk hash verified in the binary, smoke launch).
