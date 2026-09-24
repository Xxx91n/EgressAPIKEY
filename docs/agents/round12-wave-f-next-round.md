# R12 wave-f — next-round task book

Canonical: `docs/agents/round12-wave-f-next-round.md`
Handoff copy: `.scratch/r12-wave-f-grill/handoffs/next-round.md`
Sole spec source: `.scratch/r12-wave-f-grill/decision-ledger.md` (D-001..D-003, all current)
Produced: 2026-09-24 wave-f grill consolidation. Every task declares its D-xxx coverage.

## 0. Round shape (D-001)

Agenda A = fired-line adjudication round. Main item: DbPool pool-impl adjudication (register arm (ii) fired on machine evidence). Small ticket: Mode A census extractor hardening + register stale measure. Observation items land as write-back notes only — never tickets. Armed items stay parked absent new evidence (no re-grilling).

Binding negatives carried by the round:
- No source edits during grilling; implementation is future work.
- Method: per-question code evidence + atomcode terminal adjudication; zero revised records.
- Do not reopen parked items (#1 monolith / #4 i18n / #7 envelope) without new evidence — trigger-line discipline.

## Fixed points

- Integration tip `ae9ecb29` = tag `r12-wavee-ci.1`; release-matrix run 35960121325 produced all 13 artifacts incl. both opt-in Intel legs; audit verdict PASS (`.scratch/r12-wave-e-grill/reports/2026-09-24-audit.md`).
- DbPool arm (ii) FIRED verbatim: `p99=9615us over 200 samples (max=9879us over_threshold=76)` on run 35959139983; fixup `fd2f1ad1` restored green (probe now reports + 250ms hang bound).
- `crates/resin-core/src/db.rs` 821 lines: `acquire()` single seam at :119 (debug_assertions-gated `LockWaitStats` instrument); probe test `concurrent_list_ports_lock_wait_probe_reports_p99` at :759.
- `scripts/mode-a-contract-check.cjs` 498 lines; fn extractor regex at :275 covers `pub?`+`async?`+`fn` only — misses `unsafe`/`const`/`extern`/macro-generated fns; current 39-fns census has no misses today (audit independent re-extraction).
- `tests/fixtures/mode-a-boundary/census.json` = frozen inventory; README documents the deliberate-change path (update census in the same commit + cite D-id/ADR).
- AGENTS.md actual size 32690/32768 B (78 B headroom) — register now carries the corrected figure.
- Research artifacts: `.codex-tmp/atomcode-wavef-q{2,3}-{prompt,report}.md`.

## R12-F1 — census extractor hardening + instrument annotation alignment (D-003 + D-002 tail)

Files: `scripts/mode-a-contract-check.cjs`, `tests/fixtures/mode-a-boundary/census.json`, `crates/resin-core/src/db.rs` (comments only), `docs/agents/trigger-line-register.md` (closeout already landed).

Deliverables:
1. Widen the fn-extraction regex (script :275) to the full qualifier space — `pub(…)?` + any combination/order of `const` / `async` / `unsafe` / `extern "…"` before `fn`; `extern "C"` quoted-string handled; invalid combos may over-report (fail-closed is intended).
2. Add `extractor_version` field to census.json — a regex upgrade must bump it and update census in the same commit (assertion self-documents).
3. Add the declarative-macro fail-closed assertion: any `macro_rules!`/`macro` definition in port_forwarder.rs -> gate FAIL with the offending line number and two disposal options in the message (inline the expansion into census / rewrite as a plain fn) — a ten-second decision, not a research problem.
4. Unify the `frozen_at` line-count convention to `content.split('\n').length` and add `line_count_convention` note to census.json.
5. D-002 tail — annotation-only alignment in `db.rs`: probe/instrument comments updated to record the adjudicated outcome (arm (ii) discharged with a keep-Mutex verdict; probe is now a regression/A-B instrument, not a production criterion; arm (i) is the primary instrument). NO code behavior change — debug_assertions gating, counters, and the 250ms hang bound all stay exactly as-is.
6. Verify the gate still passes 19/19 locally-as-script (the check runs in verify-build.sh — CI is the acceptance surface per ADR-0072).

Rejected by adjudication (do not implement):
- Trait-impl / proc-macro-call-site coverage expansion — impl-block fns are internal detail, not the dataplane contract (golden-file principle: freeze consumer-visible contract only); proc-macro deps are caught by Cargo.toml diff anyway.
- syn / rustdoc-JSON AST extraction now — nightly coupling + format drift (cargo-public-api #858, rust #135600) violates ADR-0072 minimal CI surface; the AST upgrade trigger line is armed in the register instead.

## Register write-backs — LANDED in the wave-f closeout commit

- DbPool row rewritten: arm (ii) consumed (fired -> keep-Mutex verdict, emschwartz/abseil cited); arm (i) = primary instrument; arm (iii) probe leg dropped (two zero dev cycles alone discharge); arm (iv) hard date 2026-10-20 unchanged; successor deadpool-sqlite arm(i)-gated with B-first ordering.
- New parked row: DbPool read-path unlock (P3) — fires only on measured headless/VPS read latency (never synthetic); remedy = long-lived LazyLock read-only Connection (hynek -shm warning recorded; snapshot_db_readonly precedent not transferable).
- New armed row: census extractor AST upgrade trigger (proc-macro dep / 3+ files / >=2 regex fixes per year).
- AGENTS ceiling measure corrected 32756 -> 32690 (78 B headroom); .scratch historical files intentionally untouched (point-in-time records).
- Mode A row status: extractor-hardening note appended (traceability chain D-002 -> wave-f D-003).
- spawn_blocking observation closed as verdict (harmless exposure).
- New R12-F observations section: s1 post-land pending, probe sanity assert accepted, rustc cfg-harness disclosed deviation, wave-e lanes awaiting user land.

## Carried observations (no ticket)

- s1 post-land: first nightly webview-smoke on main (cron 03:30 UTC) after the lanes land must show `s1-window-up` green + `titleEvolution` in detail; a repeat failure re-adjudicates the remedy.
- faultinject first scheduled run 2026-10-01 03:00 UTC — check the dry-run echo step for PHASES_ARG/HEADLESS_ARG.
- convergeDevMark overlap counter: two zero cycles -> negative verdict; >=1 overlap fires React-Query arm (a).
- W8 bench-selfhosted runner: user-side action; `dagger` provisional-thresholds line still depends on it (fallback 2026-10-20).
- If arm (i) ever fires on real load: remedy order is B (long-lived read conn, ~0.5-1 day) before A (deadpool interact()-migration, ~2-4 days) — recorded in the register row.

## Suggested skills

- `implement` / `tdd` — R12-F1 (script assertions + metadata; comment-only Rust diff).
- `code-review` — pre-merge two-axis review.
- `atomcode-research` — external adjudication only, serial.
- `gitbutler` (`but`) — all commits/pushes.
- `neat-freak` / `handoff` — post-land closeout.
