# Round 12 Wave-L - Next-Round Taskbook

Scope: watch-only wave. The single fired obligation (critique delta-scan armed line) was executed and discharged in-wave (D-001/D-002): zero actionable findings, six closed-observations. No implementation tickets issued.

## How to use this taskbook

Each item declares the D-xxx records it covers. Binding negatives at the end are part of the contract - do not reopen discharged items without new machine evidence.

## R12-L1 - faultinject first monthly run (~2026-10-01 03:00Z) - carry

- Covers: r12-wave-d schedule legislation; r12-wave-k D-001.5 roll-call.
- At/after 2026-10-01 03:00Z: confirm the bench.yml scheduled run exists; inspect the dry-run echo (PHASES_ARG/HEADLESS_ARG) and results.json for the faultinject phase. The armed heartbeat fires only after TWO consecutive missing occurrences.
- macOS leg note: explicit schedule phases (app/appstart/headless/headlessstart/faultinject) record "skipped" rows on macos-latest - no exe is built there by design; skipped is graceful, not a failure.
- Earliest checkpoint (r12-wave-n D-002): the 2026-09-28 02:00Z weekly bench run is the first observable faultinject evidence (schedule events inject faultinject into PHASES_ARG). Check run existence + faultinject rows in results.json. cancelled != absent - run 35574009682 (2026-09-21, ~1h, cancelled) does NOT count toward monthly absence; a missing or cancelled 09-28 run must be flagged within the wave. Monthly absence counting (1st-of-month cron) unaffected; this note does not feed the monthly absence tally; no trigger-line implication.

  - Outcome (r12-wave-o, checked 2026-09-28 ~07:26Z): the 09-28 scheduled run was ABSENT - flagged per duty; register R12-O holds the record. Monthly tally unaffected; no trigger-line implication.

## R12-L2 - wait_bound flake watch (r12-wave-k D-001; unchanged by wave-L)

- Covers: r12-wave-k D-001 armed row.
- count=1 on record (36259888091 attempt-1 fail / failed-only rerun green). Same-signature recurrence ("bound set did not reach want=") on main verify -> rerun once (one-shot classification, not retry-to-green): green = count=2 -> fix ticket (event-signal wait_bound or budget extension, wave-i D-007 mechanism channel); red = count=2 immediately.
- TTL: bound to the 10-20 eval block - still poll-structured with no recurrence by then auto-promotes to a hygiene ticket.

## R12-L3 - macOS tier-2 signal watch (r12-wave-j D-004)

- One green on record (36255905315). First macOS-unique failure on the non-gating leg fires the armed promotion row (tier-2 -> tier-1 re-tier adjudication).

## R12-L4 - ~10-10 strategy_service split wave prep (r12-wave-i R12-I6)

- Covers: wave-i split scheduling; wave-j D-005/D-008 composition-check prefilter.
- When due: composition check BEFORE treating LOC as an architecture issue; snapshot.rs is NOT in scope (read-model, wave-j D-005); parked row preempts on headroom <200 or a feature ticket touching the file.

## R12-L5 - 10-20 hard block (carry + wait_bound TTL joins here)

- DbPool arm(iv) verdict MUST land (no arm(i) corroboration -> closed-verdict archive); convergeDevMark dead-fallback decision (postmortem if dead); W8 fallback; i18n codegen evaluation; wait_bound TTL join (R12-L2).

## R12-L6 - roll-call / user-side

- convergeDevMark Read#2: requires a REAL GUI session (exes exist on disk - wave-k audit correction; session type is the blocker).
- User-side: upstream issue submission (dormant-fork clock starts on submit); packaging acceptance (tag/push/dispatch - macos-test required-check guards the dispatch path).

## Armed lines fired by the next source merge

- None outstanding. The delta-scan line was fired by merge 9c422868 and positively discharged (r12-wave-l D-002); reopen-when on the register row: frozen range re-opened / pass2 methodology upgrade / released hunk defect-attributed back into the range.
- Parked (NOT armed): fresh-context second-reader methodology - promotes only when the next fired line's delta carries cross-file semantics (r12-wave-l D-001/D-003; binding-negatives discipline: never armed without machine evidence).

## Binding negatives (r12-wave-l D-001..D-003)

- No full critique rescan without the wave-k precondition (critic false-positive stats or a detector swap).
- No re-grilling discharged/parked lines without new machine evidence.
- No source edits inside grill waves; delivery evidence is CI-only (ADR-0072).
- The second-reader line is parked, never armed without machine evidence.
- Sealed-evidence discipline: scratch audit/report artifacts are sealed evidence - corrections go in dated append-only errata blocks, never rewritten in place (r12-wave-n D-003).

## Suggested skills

- `$gitbutler` - all VC writes (dedicated branch per session).
- `$atomcode-research` - adjudication research only, strictly serialized.
- `codegraph` CLI - first stop for code questions.
- `$handoff` - end-of-session durable taskbook.
