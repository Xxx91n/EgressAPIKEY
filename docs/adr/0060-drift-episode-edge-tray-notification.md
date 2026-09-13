# ADR-0060: Drift-episode edge tray notification (rising-edge predicate)

Status: ACCEPTED
- **Date**: 2026-09-04 (round5 T12 / 票 37 四环 (b) 上半)
- **Revises**: ADR-0054 §E — the notify contract changes from "once per process, re-arm on zero drift" to "once per drift episode (rising edge)". The hook point, the acknowledged-exemption rule, the sidecar-down silence rule, and the delivery path are unchanged.
- **Research basis**: R-B §3 Q3 (`C:\Users\Administrator\AppData\Local\Temp\round5-atomcode-B-config-authority.txt`) — ArgoCD notifications `when` + `oncePer` and AWS Config "SNS fires on compliance-state TRANSITION, not on steady state" are the industrial isomorphs; historical ticket 36 ADR-0057 diff-then-skip is the same transition-only reporting shape applied to writes.

## Context

ADR-0054 §E landed the drift tray notification as a one-shot per process:
`DriftNotifyState{armed: bool}` starts armed, a fire disarms it, and only a
snapshot reporting ZERO unacknowledged drift re-arms it ("归零后再武装").
Three structural defects (round5 issue 12, R-B Q3 verbatim):

1. It detects "drift EXISTS", not "drift TRANSITIONED" — several entities
   drifting across consecutive polls of the same episode cannot be
   distinguished, and a platform and a port drifting in the same pass count
   as one notification by construction (no per-entity dedup key).
2. The re-arm-on-zero rule conflicts with the acknowledged-exemption
   contract: acknowledging an entity clears the counter, which re-arms the
   state — a bookkeeping side effect ("豁免即重武装") that reads as the
   exempted entity being able to nag again.
3. Coupled to the 5s snapshot poll with no independent state history, a
   boolean cannot express the "absent → present → absent" edge, so no
   per-episode identity exists for future dedup (oncePer, F3 second phase).

## Decision

**D1 — The state is the previous predicate, not an arm/disarm flag.**
`DriftNotifyState{armed: bool}` is replaced by
`DriftNotifyState{prev_has_drift: bool}` (same field type, renamed semantics;
`Default` = no-drift baseline). No user-facing configuration exists for it
and none may be added without revisiting this ADR.

**D2 — Fire on the rising edge only.** The pure predicate is
`should_fire_drift_notice(has_drift, prev_has_drift) -> bool` =
`has_drift && !prev_has_drift`:

- absent → present (rising edge): FIRE, once per episode;
- present → present (plateau): silent — sustained drift never re-notifies;
- present → absent (falling edge, whether the user reconciled or the
  exemptions absorbed the entries): baseline update ONLY, never emits;
- absent → absent: silent.

The call site (`fire_drift_notification`) updates the baseline UNCONDITIONALLY
after every evaluation — both edges move `prev_has_drift`; the falling edge
never produces a notification.

**D3 — Acknowledge semantics (F5).** Acknowledged entities are exempt from
the counter (ADR-0054 §D unchanged). When exemptions drive the count to zero
the predicate falls, which updates the baseline only; the NEXT genuinely new
unacknowledged drift is a new episode and fires again. An exemption is
therefore never a re-arm that can re-notice the same drift.

**D4 — Process restart resets to the no-drift baseline.** The static
`DRIFT_NOTIFY_STATE` stays process-local (mirroring DRIFT_MEMORY /
RECONCILE_MEMORY): a restart recomputes from "no drift", so drift already
present at boot notifies once — ArgoCD-style current-state recomputation, no
cross-process memory.

**D5 — Scope guards.** The 5s snapshot poll rhythm is untouched; the IPC
contract is untouched (`fire_drift_notification` remains internal to the
`authoritative_snapshot` tail); per-entity `oncePer` dedup keys (ArgoCD
notifications isomorph, issue F3) are explicitly deferred to a second phase
and are NOT legislated here.

## Consequences

- A drift episode that persists for hours produces exactly one notification
  (same as before); an episode that clears and reappears now reliably
  produces one notification per appearance (the "drift never notifies again
  until a zero snapshot happens" false silence is gone).
- The acknowledge flow can no longer produce the "exempt → re-arm" state
  transition; acknowledging is presentation-side only with respect to
  notifications.
- The state machine is a pure two-bool function — the unit tests lock the
  full truth table (rising edge fires; plateau/falling/absent are silent)
  plus the acknowledge baseline sequence.
- Per-entity dedup (one notification per distinct drifting entity per
  debounce window, ArgoCD `oncePer` + Nacos 5min magnitude) remains an open
  enhancement tracked for a second phase; `DriftMemory::divergent_since`
  already provides the per-entity first-drift timestamp it would need.
