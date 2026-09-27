# Round 12 Wave-K — Next-Round Taskbook

Slug: `r12-wave-k-grill` | Baseline: `origin/main` 7be89b87 (wave-j duties landed + audited; merge-red resolved by rerun)
Ledger: `.scratch/r12-wave-k-grill/decision-ledger.md` — D-001..D-002, all current.
Reports: `.codex-tmp/atomcode-wavek-q{1,2}-{prompt,report}.md` (gitignored, session-local).
Evidence: `docs/agents/evidence/r12-wave-k/` (observation class, not delivery evidence).

## How to use this taskbook

- This wave produced NO implementation tickets — it is watch-only. Each item declares the D-xxx IDs it covers; the ledger is the source of record.
- Register write-backs landed in the wave-k closeout commit (2 armed rows + R12-K section).
- Suggested skills: `$atomcode-research` (new adjudications only, serialized), `$gitbutler` (all VC writes), `code-review` dual-axis before commit, `grill-with-docs / domain-modeling / neat-freak / handoff` next cycle.

## R12-K1 — wait_bound flake watch (D-001, D-002.2)

- Armed register row: same-signature main verify failure (`bound set did not reach want=`) -> rerun once -> green=count=2 -> fix ticket (event-signal or budget extension via wave-i D-007 mechanism-fix channel); red=count=2 immediately.
- TTL bound to the 10-20 block: still poll-structured with no recurrence -> auto-promotes to hygiene ticket (wait_bound event-ification).
- Do NOT proactively fix now (count=1, >=2 threshold precedent; no-prepay).

## R12-K2 — faultinject first run (carry, ~2026-10-01 03:00Z)

- First scheduled monthly run produces machine evidence; check run + register row on/after the date (heartbeat line: absent >=2 consecutive occurrences -> fires).

## R12-K3 — macOS tier-2 signal watch (carry, wave-j D-004)

- First macOS-unique test failure on the non-gating bench leg -> promote armed row to adjudication. One green on record (36255905315).

## R12-K4 — strategy_service split wave prep (~10-10, R12-I6)

- Unchanged. Wave-j D-008 composition-check applies at agenda entry: prod/test split + duties-per-line before any LOC charge earns a slot. Snapshot.rs is scope-fenced OUT (read-model != write-orchestration).

## R12-K5 — 10-20 hard block (carry + TTL join)

- DbPool arm(iv) verdict / convergeDevMark dead-fallback (postmortem if dead) / W8 fallback / i18n-codegen eval / **NEW: wait_bound TTL check** (R12-K1).

## R12-K6 — roll-call / user-side (D-001.5)

- convergeDevMark Read#2 on next real GUI session.
- User-side: upstream probe issue submission (dormant-fork clock at submission); packaging acceptance (tag + push + workflow_dispatch — macos-test required-check now guards that path).
- snapshot prod-lines ratchet watch: 918/2000.

## Armed lines fired by next source merge

- **delta-scan** (D-001.4/D-002.3): first post-7be89b87 merge touching src/, src-tauri/, crates/, scripts/, or src/locales/ -> scan frozen range 33dc0333..7be89b87 only.

## Binding negatives (D-001..D-002)

- No critique v3 full rescan without the reopen precondition (false-positive stats or detector swap).
- No in-test retry / auto-quarantine additions (ADR-0072 noise discipline; legislated absence).
- No proactive wait_bound fix before count=2 or TTL expiry.
- Armed/parked lines without new machine evidence are not re-grilled.
- All register armed rows carry obs-env:/zero-read:/evidence: — the instrument-declaration gate enforces.
