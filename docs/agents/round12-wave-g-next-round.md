# R12 Wave-G Next Round - Task Book

Source of truth: `.scratch/r12-wave-g-grill/decision-ledger.md` (1 current
record: D-001). Base: origin/main `e71cb54d` - wave-e + wave-f fully landed,
CI verify run 36033301265 green.

Wave-g was a **verification/semantics round**: it produced NO code ticket.
This book is an operational duty list; every item cites its covering D-xxx.

## 0. Round shape and binding negatives (D-001)

- One discharge-hold (s1), one semantics legislation (convergeDevMark
  cycles), one metadata hygiene (residual_limits), one roll-call.
- Binding negatives (ledger verbatim):
  - NO re-grilling of parked/armed items absent new evidence (monolith,
    i18n trim, envelope versioning, AST upgrade, edge-adapter, React Query
    arm a).
  - residual_limits is metadata-only: no gate judgment-path or census
    change; zero runtime behavior change.
  - The s1 waitUntil 30s/500ms poll is the remedy itself - permanent,
    never weakened or removed.
  - The s1 holding window counts SCHEDULED (cron) runs only - a cancelled
    scheduled run does not count (35976721262, 2026-09-24).
  - .scratch historical files are point-in-time: never edited.

## R12-G1 - s1 scheduled-run watch + closure (covers D-001.1)

State at handoff: EVIDENCE-POSITIVE, HOLDING. Push-run 36033301454 (main @
e71cb54d) shows s1-window-up PASS on both legs with `titleEvolution` in
detail; the ubuntu leg captured the diagnosed race verbatim ("" at 11ms ->
"EgressAPIKEY" at 579ms) - mechanism-level proof the poll absorbs it.

Duty:

1. Watch the next scheduled nightly run(s) - cron 03:30 UTC, main only
   (first candidate: 2026-09-26 03:30 UTC).
2. Count only SCHEDULED greens toward the 1-2 run window; cancelled or
   push-triggered runs do not count.
3. On 1-2 consecutive scheduled greens -> update the register s1 row to
   discharged (cite run ids) and move the observation to closed verdicts.
4. On ANY s1-window-up failure -> re-adjudicate the REMEDY (in-probe
   polling), not the ticket; escalate back to grill with the new
   titleEvolution evidence.

## R12-G2 - convergeDevMark first real read (covers D-001.2)

Duty: in the next real dev session (GUI run), read the dev-only overlap
counter (console tag `[converge]`); record the reading + UTC timestamp as
a register data point. Semantics legislated: an observation cycle = a REAL
dev-session read; unread cycles are no-data, NOT zero - never-observed can
never count as a zero reading. Hard date 2026-10-20 (shared backstop with
DbPool arm iv): still no real read by then -> record dead/unevaluable
(i18n line-2 dead-arm precedent). >=1 measured overlap fires React-Query
arm (a).

## R12-G3 - scheduled-observation roll-call (covers D-001.4)

- 2026-10-01 03:00 UTC: first monthly faultinject run (bench.yml cron
  `0 3 1 * *` verified) - check the dry-run echo resolves PHASES_ARG /
  HEADLESS_ARG; two consecutive absences fire the heartbeat line.
- 2026-10-20: DbPool arm (iv) hard date - verdict MUST land; no arm (i)
  real-load corroboration -> closed-verdict archive.
- W8 bench-selfhosted runner: user-side registration; dagger
  provisional-thresholds fallback shares the 2026-10-20 date.
- monthly faultinject heartbeat + D3 back-verify warn: carried unchanged.

## Already landed in the wave-g consolidation commit (no action)

- census.json `residual_limits` field (D-001.3): the two extractor blind
  spots (same-line #[attr] fn; glued extern { fn }) enumerated; 0
  occurrences grep-verified; hand-verify mandate if ever introduced.
- Register write-backs: s1 row evidence-positive/holding; React-Query +
  convergeDevMark cycle semantics; AST row cross-reference to
  residual_limits; R12-G observations section; wave-e/wave-f lanes-landed
  closure.
- CONTEXT.md Trigger Line entry refined: observation-cycle semantics
  (NOT_OBSERVED != 0; hard-date dead/unevaluable exit).

## Suggested skills for the next agent

- `grill-with-docs` - next round's interview loop
- `gitbutler` (but) - any commits/pushes
- `handoff` - round closeout
- `atomcode-research` - evidence gathering per question (serial, one in
  flight)
- `code-review` - only if new diffs appear
