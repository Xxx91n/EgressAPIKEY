# ADR-0069: L2-first write order, reconcile idempotency semantics, atomic config import

> Status: ACCEPTED (2026-09-13, round8-grill decision D-006)
> Extends (reopens none): ADR-0042 (whitebox as truth S2/S6), ADR-0054 (reconciliation loop), ADR-0057 (apply idempotency diff-then-skip), ADR-0058 (generation/applied_generation).
> Research basis: atomcode Q6 (Kubebuilder level-based reconcile; Azure Saga pattern limitations; WAL intent-first principle; ArgoCD self-heal semantics).

## Context

The port mutation commands (port_upsert / port_remove / port_toggle)
mutate Resin first and persist the whitebox second; a second-step failure
leaves the engine serving configuration the user never persisted, the
next strategy apply self-heals it back (silent revert), and a crash in
the window loses the intent entirely. Separately, the reconcile
idempotency memory stamps successful assertions as well as 409 conflicts,
so an externally-deleted endpoint is not restored for up to 24h — even by
manual reconcile (the test reconcile_memory_ttl_throttles_reassert_and_stamps_clear_it
itself documents "zero changes even though Resin still reports the port
missing").

## Decision

### D1. L2-first write order (unification)

port_upsert / port_remove / port_toggle persist the whitebox intent
FIRST (validate -> DB -> atomic swap), then perform the Resin mutation.
A Resin-side failure returns an error while the drift stays visible and
retryable; no intent is lost. This unifies the port family with the
strategy half, which already writes L2 first. Known trade-off, accepted:
a permanently-rejected port (e.g. held by the OS) persists as a drifted
entry the user removes — desired-state behaviour, not an error loop.

### D2. Reconcile memory stamps only conflicts

stamp_asserted is called only on the 409-conflict path (anti-hammer);
successful assertions rely on the liveness filter for idempotency.
Ticket 14's hard gate ("two consecutive passes, the second is a no-op")
still holds, and externally-deleted endpoints recover on the next pass.
Tests are updated to the new semantics.

### D3. Failure visibility

When the second step fails, the command's error names the state ("L3
changed, L2 not persisted") and triggers an immediate snapshot refresh
so the drift is observable without waiting for the poll cycle.

> Note (2026-09-14, ticket 12 - wording correction, no decision change):
> D3's parenthetical names the PRE-D1 failure state. Under D1 the Resin
> mutation is the second step, so its failure leaves **L2 persisted and L3
> unchanged** - and that is precisely the drift D3 must make visible. The
> shipped message therefore reads "L2 persisted, L3 unchanged (Resin
> rejected: ...) - drift is visible, retry or remove the entry". The
> ticket-12 issue file still quotes the pre-D1 wording; it is reported as
> wording drift in the ticket report and should be corrected at source.

### D4. Atomic config import

config_import stages both whitebox documents to temporary files with
validation, then renames both into place back-to-back; any validation
failure writes nothing (no half-imported state).

### D5. Explicit non-goal: general saga compensation

Automatically compensating L3 mutations is rejected (Azure documents
that compensation transactions themselves may fail). The only sanctioned
compensation remains the narrow cascade case already legislated
(compensate_failed_cascade: delete only what that pass created, claims
confirmed by the failure-owner probe; user data and L2 untouched).

## Consequences

- The 5s/30s converge retry intervals remain project-defined parameters
  (no external SLA baseline; decision D-004's measurement ticket
  provides the data).
- port_health_check may observe a briefly not-yet-asserted port after an
  accepted upsert — the mirror image of today's window, bounded by the
  same converge retry.
