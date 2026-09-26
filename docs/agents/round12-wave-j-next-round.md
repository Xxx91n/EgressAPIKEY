# Round 12 Wave-J — Next-Round Taskbook

Slug: `r12-wave-j-grill` | Baseline: `origin/main` 12062c87 (wave-i landed)
Ledger: `.scratch/r12-wave-j-grill/decision-ledger.md` — D-001..D-008, all current.
Reports: `.codex-tmp/atomcode-wavej-q{1..7}-{prompt,report}.md` (gitignored, session-local).

## How to use this taskbook

- Each task declares the D-xxx IDs it covers. The ledger is the source of record.
- Register write-backs landed in the wave-j closeout commit (4 armed rows + R12-J section + Procedure bullet); tickets below are the next duties round.
- Suggested skills: `$atomcode-research` (new adjudications only, serialized), `$gitbutler` (all VC writes), `code-review` for the audit loop, `grill-with-docs / domain-modeling / neat-freak / handoff` for the next cycle.

## R12-J1 — audit.rs single critical section (D-002)

- `crates/resin-core/src/audit.rs`: `AuditLog::append` — hold the `last_hash` lock across prev_hash read -> serialize -> rotate -> write_row -> commit (one critical section; the type itself becomes thread-safe, not the callers).
- Concurrency regression test (CI domain): shared private AuditLog instance, N threads append concurrently, chain continuity verified (no two rows share prev_hash).
- Comment rewrite at the last_hash site: per-instance thread-safety now enforced by the lock; the same-file single-instance invariant stays documented (cross-instance concurrent writers to one file have no intra-process solution).
- argus rule unchanged: append failure still degrades to tracing::error + Ok(()).
- Note for the implementer: I/O inside the lock adds zero contention — the global AUDIT mutex already serializes the whole append including fsync.

## R12-J2 — macOS test leg: tier-2 signal + release gate (D-004)

- Non-gating signal: add `macos-latest` to bench.yml `matrix.os` (cargo test already runs in that leg; monthly cron + dispatch = periodic cost, not per-merge).
- Release gate: macOS cargo test as a required check inside the workflow_dispatch release path in ci.yml — gate-at-release, signal-at-main; never on push.
- Register armed row (escalation on first macOS-unique failure) landed in closeout.
- Do NOT add macOS to the push-gate verify job — ~10x runner cost + flake surface on the only merge gate (ADR-0072 noise discipline).

## R12-J3 — snapshot.rs prod-lines gate (D-005)

- `scripts/verify-build.sh`: new gate after the strategy_service guard — count prod lines of `crates/resin-core/src/snapshot.rs` excluding the `mod tests` block (anchor: the `mod tests` marker at ~:919; count lines before it); FAIL at >2000 (2.2x headroom ratchet, ~919 today).
- The armed register row landed in closeout; this ticket lands the enforcement. Keep the measured figure consistent with the register row wording.

## R12-J4 — hygiene batch: TopologyView `?s:` (D-007)

- `src/views/TopologyView.tsx:97-101` — replace `? {} :` with `? s :` in the five shallow-guard setters. zustand bails on Object.is(nextState, state) checked on the returned partial before merge; `{}` always produces a new root and notifies (v5.0.14 source-verified).
- The parked (⑤c) file-churn register row fires on this edit and discharges with it.
- Hygiene-batch admission only: do not touch the useShallow subscriber convention; no data-flow refactor; no standalone ticket.

## R12-J5 — observation roll-call (bookkeeping)

- s1 second scheduled green (~2026-09-26 09:00Z): on landing, close the holding note citing both scheduled runs (r12-wave-i D-007.2; mechanism-fix exception clause not needed - this wave was pre-scheduled).
- convergeDevMark Read#2 on the next real GUI session (overlap-only scope).
- faultinject first scheduled run 2026-10-01 03:00Z.
- ~2026-10-10: strategy_service split wave (R12-I6, unchanged).
- 2026-10-20 hard dates: DbPool arm(iv) verdict, convergeDevMark dead-fallback (postmortem if dead), W8 fallback, i18n-codegen eval.
- User-side: upstream probe issue (dormant-fork clock starts at submission); packaging acceptance (tag + push + workflow_dispatch).

## Binding negatives (D-001..D-008)

- No file locks / cross-process audit protection (out of scope); argus degrade semantics frozen.
- No port-probe rework: no held-socket transfer (architecturally impossible across the whitebox boundary), no self-healing port swap (config contract); `suggest_` naming stays.
- No macOS in push-gate verify; no unconditional platform-test exemption (exemption without triggers = silent drift).
- No snapshot split (dilutes the ADR-0051 single-merge-point audit surface); snapshot is NOT eligible for the ~10-10 wave (scope fence: read-model != write-orchestration debt).
- No ipc.ts domain split without usage evidence; specta runtime-gen stays rejected (ADR-0053; sole reopen = middleware hooks carrying trace_id+dual-mode).
- No LOC gate on ipc.ts (interface-role drift only); no TopologyView data-flow refactor.
- Composition-check pre-filter gates agenda entry only — never substitutes adjudication.
- Armed/parked lines without new machine evidence are not re-grilled.
