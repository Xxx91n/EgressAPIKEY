# ADR-0058: generation/observedGeneration convergence echo (whitebox write-authority counters)

> Status: ACCEPTED
> Date: 2026-09-04
> Ticket: round5-config-authority T09 (generation-observedgeneration-echo, 波次 W3.1, 票 37 四环 a)
> Extends (reopens none): ADR-0054 §A (one-way reconcile), ADR-0051
> (authoritative snapshot read model), ADR-0036 (strategy whitebox single
> write entry), ADR-0042 S2 (ports whitebox truth source), ADR-0057
> (diff-then-skip apply idempotency).

## Context

The reconcile loop closes the gap between desired (L2 whitebox) and live
(L3 Resin) on demand, but nothing tells the user whether their last write
ever took effect. Issue 37 §4 gap (a) lists the missing fields: the
whitebox files carry no write-generation counter, no last-apply timestamp,
no last-apply error; `AuthoritativeSnapshot` carries no convergence phase.
A user who edits `egressapikey-strategy.json` (or clicks apply) cannot see
「已生效于 HH:MM (rev N)」 and cannot distinguish "failed" from "never
applied" from "wrote but not yet applied".

## Research basis (R-B §4.1-4.6)

The asymmetric two-counter model is the Kubernetes
`metadata.generation` / `status.observedGeneration` pair (KEP-1623
standardized the error surface as a status condition with reason +
message). Crossplane keeps the double counter only for types that manage
EXTERNAL resources; its LOCAL types (comparable to our ports half) are
single-generation because the writer and the applier are the same process
and the apply completes synchronously. Terraform refreshes the state
timestamp on every successful (even no-op) apply. The apply-failure
record must keep the OLD observed value — faking convergence on failure
is the exact bug class the counter exists to expose.

## Decision

**D1 — strategy half carries the full double counter.**
`egressapikey-strategy.json` gains top-level `generation` (write
authority), `applied_generation` (observed), `last_apply_at` (Unix s),
`last_apply_error` (KEP-1623-style reason+message string), `updated_at`
(all `#[serde(default)]`; a v1 file deserializes unchanged with
`generation = 0`). No schema version bump and zero migration: serde
defaults ARE the v1→v2 compat story.

**D2 — bump points.** EVERY sanctioned store-path write bumps
`generation` after validate, before the file lands: the IPC put
(`StrategyService::store`), the deep region edit
(`set_platform_regions`), rollback (`FsStrategyStore::rollback`), and
apply's own write-back. The counter lives in the write entry (ADR-0036
discipline), not in callers.

**D3 — apply write-back inside the same store entry (R-B Q2 main
answer).** When an `apply` pass returns ALL GREEN (every per-platform row
`patched = true`, including diff-then-skip "in sync" rows), the service
writes back in the SAME store entry:
`applied_generation = generation` of the document the write-back lands
(the counter pair is EQUAL again), `last_apply_at = now`, clears
`last_apply_error`. Anything not green keeps the old `applied_generation`
(no fake convergence) and records the first failing row's reason. A green
pass over an already-converged world (zero PATCH) still refreshes
`last_apply_at` — Terraform re-apply semantics.

**D4 — ports half is single-generation (D-27).**
`egressapikey-ports.json` gains ONLY `generation` + `updated_at`.
Rationale (Crossplane local-type rule): the ports writer
(`WhiteboxConfigStore::apply`) and its apply executor are the same
process and the apply completes synchronously inside the call — there is
no external resource whose observation can lag. The bump is based on the
CURRENT committed generation (`self.snapshot().generation + 1`), not the
incoming document's, because callers hand-build documents that may not
carry the latest counter.

**D5 — ConvergePhase, a top-level read-only phase (D-28).**
`AuthoritativeSnapshot` gains `strategyGeneration`,
`strategyAppliedGeneration`, `convergePhase`, `lastApplyAt`,
`lastApplyError` (camelCase on the wire). `converge_phase` is a pure
derivation (`snapshot::derive_converge_phase`, table-driven unit tests):
`NeverApplied` (generation == 0 — k8s habit: a fresh boot is not a fake
"pending apply" alarm) > `Unknown` (Resin unreachable — honesty outranks
guessing) > `ApplyFailed` (applied < generation with recorded error) >
`PendingApply` (applied < generation, no error) > `Drifted` (applied ==
generation but unacknowledged entry drift exists, per ADR-0054 §D
exemption discipline) > `Converged` (applied == generation, no
unacknowledged drift). The phase is a SECOND axis, orthogonal to the
per-entry three-state of ADR-0051 — no fourth per-entry state is created.

**D6 — UI surfaces ONE chip, zero new requests.** EffectiveConfigView
renders the phase as a header chip from the snapshot it already pulls:
green 「已生效于 HH:MM (rev N)」 for Converged, red 「ApplyFailed ·
reason」 for ApplyFailed, amber clickable 「待应用 · 点击立即收敛」 for
PendingApply (opens the existing reconcile preview), plus honest labels
for the other three phases. No automatic reconcile loop is introduced
(ADR-0054 user-explicit trigger discipline stands).

## Consequences

- The 「用户改了配置不知道有没有生效」 pain point closes: the chip answers
  wrote? (rev N) / applied? (rev N) / when? (HH:MM) / failed? (reason).
- An apply failure is now PERSISTENT STATE, not a transient toast.
- The apply write-back adds one whitebox write per green apply (mtime
  changes; the strategy half has no file watcher, and the write goes
  through the versioned backup path like every other store write).
- The ports counter is monotone under the file-watch reload race (a
  watcher reload can re-read the disk value mid-test windows); consumers
  treat it as "monotone non-decreasing per accepted apply", the same
  contract as `last_checked_at`.
- Rollback is a write: the reverted document lands at generation N+1 and
  the phase re-derives (rollback of a converged config reads
  PendingApply until the next apply converges it — honest).
- TS wrapper mirrors the six-phase union and sanitizes the counters as
  untrusted bounded integers; malformed phases degrade to `Unknown`
  (honest), never to `Converged` (fake).

## Out of scope (explicit non-goals)

- No automatic reconcile loop (still user-explicit, ADR-0054).
- No ports-half `applied_generation` (D-27; revisit only if ports ever
  gain an external-resource apply).
- No audit-jsonl integration (T11 owns the audit chain; the whitebox
  backup files already version every write).
