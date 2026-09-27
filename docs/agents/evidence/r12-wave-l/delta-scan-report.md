# r12-wave-l delta-scan evidence

> observation data, not delivery evidence (r12-wave-h D-005); delivery evidence remains CI-only (ADR-0072).

Fired armed row: critique delta-scan (r12-wave-k D-001/D-002) - merge 9c422868 touched scripts/.
Frozen range: 33dc0333..7be89b87 (wave-i/j diff, 23 files, +1144/-168).
Executed 2026-09-27 per r12-wave-l D-001 (two-pass self-scan) / D-002 (outcome disposition).

## pass1 - traceability map (file -> adjudicated wave commits)

| File | Commits | Wave work |
|---|---|---|
| crates/resin-core/src/audit.rs | b9b68b6a | R12-J1 single critical section (wave-j D-002) |
| crates/resin-core/src/db.rs | a85bc47c 81668d29 299fde65 8dac5c07 | db-lock-metrics feature + dual tiers + read leg + BUSY test (wave-i D-003.1/D-004) |
| crates/resin-core/Cargo.toml | a85bc47c | db-lock-metrics feature decl |
| src-tauri/Cargo.toml | a85bc47c | feature forward |
| src-tauri/src/headless_security.rs | 1c2a725c 9f70a1f1 | XFP rightmost + iterator fix (wave-i D-002) |
| src-tauri/src/headless_main.rs | 1c2a725c 67e9d717 fad26c09 | wildcard proxy gate + fmt + refusal msg (wave-i D-002) |
| docs/how-to/HEADLESS_DEPLOYMENT.md | 1c2a725c | doc for the two flags (matches impl verbatim) |
| src/views/TopologyView.tsx | 10b71a2d | R12-J4 ?s: guards (wave-j D-007) |
| src/store/appStore.ts | 44b2b1d1 | convergeDevMark comment - overlap-only semantics (docs-only) |
| .github/workflows/ci.yml | f399ca80 | macos-test tier-2 release gate (wave-j D-004) |
| .github/workflows/bench.yml | f399ca80 299fde65 | macos leg + dbprobe phase + duration asserts out of push gate |
| scripts/bench/run-bench.mjs | f399ca80 12062c87 299fde65 | dbprobe phase + host-target override + failure echo |
| scripts/bench/acceptance.json | 299fde65 | two measure-only rows (dbprobe) |
| scripts/db-immediate-txn-check.cjs | 299fde65 | new source gate (BEGIN IMMEDIATE) |
| scripts/instrument-declaration-check.cjs | 79f1b7b0 1d773a5d a85bc47c | new three-field declaration gate |
| scripts/verify-build.sh | 79f1b7b0 a85bc47c 299fde65 7097f7cc | mounts: feature pass + txn gate + instrument gate + snapshot prod-lines |
| docs/agents/evidence/ (README + r12-wave-h digests) | 1d773a5d 4dd0ff07 | evidence convention + digests |
| docs/agents/*taskbook/register | 7be89b87 4c262ec7 4dd0ff07 ed13f006 | wave closeouts/errata |

Coverage: 23/23 files map to adjudicated wave-i/j commits; zero orphan hunks.

## pass2 - mechanism defect hunt (mandatory list, non-exemptable)

1. db.rs (+411): three StdMutexes (wait_micros / holder / read_holder) each in short non-nested scopes - no lock-order cycle; observe() runs under the data lock so holder reads the immediately-preceding acquirer (accurate attribution); writer/read domains separated (read waits never pollute the writer p99 reservoir); cfg feature consistent across struct/tests/impl; verify-build feature pass (cargo check --all-targets + test) prevents dead-cfg rot; read fallback->writer domain is semantically correct (the wait IS on the writer lock).
2. audit.rs (+77): single critical section verified - lock order last_hash -> written_bytes everywhere; rotate touches only written_bytes (no parking_lot reentrancy deadlock); failure paths drop the guard with the chain tail uncommitted (consistent: next append re-reads the same prev_hash); the new concurrency test asserts the real invariant (each row links to the physical previous row's own-hash).
3. headless_security.rs +89 / headless_main.rs +99: rightmost-wins across merged header instances; every boundary case fails closed (empty/missing/trailing-comma/whitespace/garbage segment); is_wildcard_proxy_net via prefix_len()==0 covers all /0 spellings; startup refusal precedes any dir/tracing/sidecar work; docs match verbatim.
4. TopologyView.tsx (+10): same-ref return makes zustand v5 Object.is bail real (bail precedes merge in vanilla.ts).

Demoted checks: instrument-declaration-check (directional pin match, stale-pin failure, covered-by/reopen-when pairing, fail-closed) OK; db-immediate-txn-check (comment-strip + whitespace-collapse + brace-match, all failure directions conservative) OK; verify-build mounts OK; ci.yml macos-test (dispatch-only, needs verify, tag checkout matches sibling release legs) OK; bench.yml macos leg + explicit phases without exe record graceful skipped rows (verified in run-bench.mjs phase functions) OK; acceptance.json measure-only rows OK.

## outcome

Zero actionable findings. Six closed-observation nits (all filtered by composition check, none earn a ticket):

1. read_conn_dedicated() ungated derefs LazyLock -> force-inits read conn if ever called on a poll path; zero production callers today (footnote hazard).
2. XFP filter_map drops non-UTF8 header instances before rightmost resolution -> binary trailing instance skipped, penultimate readable wins; worst case = Secure planted on http cookie (browser drops it) = availability noise.
3. concurrent_appends heads tally vacuous (always 1; real check in else-branch assert).
4. ci.yml macos-test comment "same test scope as the unix verify leg" omits the db-lock-metrics feature pass - touch rule: fix wording on next ci.yml edit.
5. BUSY-lock test panic path parks holder thread + leaks temp dir on Windows (test-hygiene noise).
6. dbprobe spawnSync status===null without error -> silent empty marks (measure-only marginal robustness).

Declarations (wave-i D-007): obs-env: single-reader self-scan inside the governance session (2026-09-27); zero-read: no independent second reader - detection-rate floor unknown (maker-checker residual disclosed); evidence: this directory + ledger rows.
