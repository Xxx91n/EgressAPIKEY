# ADR-0080: Signal-plane probe verdicts - typed outcomes, common-mode suppression

Status: ACCEPTED
- **Date**: 2026-09-22 (r12-wave-b grill D-002; execution ticket R12-B1)
- **Supersedes**: the majority-rule aggregation segment in
  `orchestration_tick_impl` (former `bad * 2 > total` — a same-source
  pseudo-majority: every bound-port probe shares one process, one loopback
  stack and one vantage, so port count was never independent witness count)
- **Extends**: [ADR-0074](0074-orchestration-graded-autonomy.md) (graded
  autonomy state machine — its six invariants and ALL parameter values are
  unchanged; this ADR retypes the evidence the evaluator consumes)
- **Research basis**: wave-B grill atomcode sessions (IETF
  draft-inadarei-api-health-check narrow-enums+rich-detail, Envoy
  split_external_local_origin_errors fault classification, AWS Route53
  multi-vantage argument, sonarops layered alerting, speakeasy contract
  evolution convention — cross-verified at source)

## Context

The wave-a signal plane emits one boolean per platform per tick
(`tick_failed`). Three defects motivated re-typing it:

1. **The return contract was blind**: `signals: N` reported a count,
   not verdicts — a consumer could not tell *what* failed.
2. **Single-vantage pollution**: a local network partition fails every
   loopback probe at once; the boolean recorded N platform failures for
   what was one environmental event (WAN-partition blast radius, ledger
   C3).
3. **The join was lossy**: loopback (`port_health_check`) and egress
   (`probe_exit_ip`) verdicts were OR-ed into one bit, discarding
   *where* the failure lived.

## Decision

### 1. ProbeVerdict — five-value enum (per probed bound port)

`ok | local_fail | remote_fail | skipped | environment_suspect`, serde
snake_case. Each port's verdict JOINS the two SLI probes:

| loopback | egress | verdict | note |
|---|---|---|---|
| ok | ok | `ok` | |
| ok | fail | `remote_fail` | egress path dead past a live listener |
| fail | fail | `local_fail` | shell-side entry broken |
| fail | ok | `local_fail` | contradiction — recorded in detail, classified local |

`skipped` = the port produced no evidence this tick (probe task join
failure) — an explicit no-evidence slot, never silently counted either
way. `environment_suspect` is never produced by the join; it is stamped
by the suppressor (§3).

Every verdict MAY carry `detail` — a ≤64-byte operator-facing note
(char-boundary-safe truncation, NUL/control rejected, same discipline as
`bounded_diff` / the §7.5 IPC caps). Remote-fail rows carry the observed
egress latency/status; contradictions record both probe outcomes. Detail
enters the IPC return surface and audit rows only — **never the ring**.

### 2. Platform tick verdict — dominant-class aggregation (supersedes majority rule)

Let `v` = the platform's port verdicts this tick, `valid` = v minus
`skipped` (skipped ports hold no evidence and leave the denominator).

- `valid` empty → `skipped`
- fail-class ports (local_fail + remote_fail) strictly more than half of
  `valid` → the platform verdict is the dominant fail class; ties break
  to `remote_fail` (the actionable class — local common-mode is the
  suppressor's job, §3)
- otherwise → `ok`

This keeps the former rule's one-bad-port tolerance while replacing its
typeless count with a classified verdict.

### 3. Common-mode suppressor (tick level, all platforms)

When `local_fail` ports reach **≥80% of all probed enabled bound ports
in the tick** (denominator = attempted probes, skipped included — the
attempt was made), the whole tick is environment-suspect: every probed
platform's verdict becomes `environment_suspect` and the tick-level
suspect streak increments. Integer behavior of the 80% rule, per probed
count n:

| n | suppresses at | reading |
|---|---|---|
| 1–4 | n/n (unanimous) | small fleets need full agreement |
| 5–9 | ⌈0.8n⌉ | 5→4, 6→5, 7→6, 8→7, 9→8 |
| ≥10 | ≥80% | direct share |

Single-port degenerate: one bound port failing loopback reads as
suspect — with n=1 the shell cannot distinguish its own listener fault
from platform failure, and the streak audit (§4) bounds the blindness.
The suppressor covers the *common-mode* case only; a partially degraded
WAN (mixed local/remote fails) stays unsuppressed by design — the ADR-0074
cooldown path remains the backstop for the tail (wave-a D-001 linkage).

### 4. Suspect streak → audit + environment_status

A tick-level u32 counter (process-local, independent of platform rings)
increments per suspect tick and resets on any clean tick — interruption
zeroes it. At streak **3** (hardwired, not a parameter — anti-Goodhart)
the tick emits one audit row (target `signal-plane`, op
`environment_suspect`, reason carries streak + share evidence) and the
tick response's `environment_status` field reports
`{suspect_streak: n}` on every path.

### 5. Ring & WindowStats — typed samples, valid-only denominators

The per-platform ring stores `ProbeVerdict` (was bool), cap unchanged
(64). Suspect and skipped entries act as **time barriers**: they enter the
ring (the evidence gap is real history) but the evaluator never counts
them — `WindowStats.samples` = valid verdicts (ok/local_fail/
remote_fail), `fails` = the two fail classes, `consecutive_fails` =
the trailing fail run truncated at any non-fail entry, `tick_failed` =
this tick ∈ fail classes. `evaluate_section` and every ADR-0074
threshold are untouched — suppression works by starving evidence, not by
retuning gates.

### 6. Return surface — additive only

Every tick path returns the new fields; nothing is removed:

- `signals`: platform count (kept verbatim, back-compat)
- `platform_verdicts`: rows of `{platform, verdict, ok_share, streak}`
  — one per probed platform (bounded by platform count). `ok_share` =
  ok / valid samples (null when the window holds no valid samples).
  `streak` = trailing run of this tick's verdict, suspect/skipped
  truncating.
- `verdict_details`: `{platform, detail}` rows only where detail
  exists (bounded by platform count, each ≤64B).
- `environment_status`: `{suspect_streak}` on every response.

### 7. Persistence & projection

When an orchestration section exists (enabled or explicitly disabled),
the tick persists `signal_verdicts: {platform: verdict}` +
`suspect_streak` into the section via the same bookkeeping write
(status subresource — no generation bump). An absent section is never
created just to hold signals (zero-migration invariant). The desktop UI
projects `skipped` (and every verdict) as a per-platform chip on the
Platforms view — a contract with no consumer is a dead contract.

## Registered gray zone

A half-degraded WAN can fail every egress probe while loopback stays
healthy → misclassified `remote_fail`. Detail rows carry the observed
latency/status so an operator can tell egress-dead from WAN-half-dead.
Escape condition (registered): if misclassification reaches ≥1 observed
incident/month, re-open for a third probe or a time-correlation window.

## Consequences

- Consumers can classify failure origin (shell vs egress vs environment)
  without parsing logs; the audit trail records environmental events as
  first-class rows.
- The suppressor narrows evidence during real partitions instead of
  degrading every platform — blast radius of a local outage shrinks to
  one audit row + a status field.
- Majority-rule deletion removes a pseudo-quorum: port count was never
  independent witness count, and the typed join + suppressor carry the
  information it was approximating.
- ADR-0074 parameters unchanged; evaluator code paths unchanged; the
  change is confined to evidence production and classification.
