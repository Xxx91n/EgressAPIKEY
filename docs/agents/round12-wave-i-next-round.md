# Round 12 Wave-I — Next-Round Taskbook

Slug: `r12-wave-i-grill` | Baseline: `origin/main` 33dc0333 (wave-h landed)
Ledger: `.scratch/r12-wave-i-grill/decision-ledger.md` — D-001..D-007, all current.
Reports: `.codex-tmp/atomcode-wavei-q1..q7-{prompt,report}.md` (gitignored, session-local).

## How to use this taskbook

- Each task declares the D-xxx IDs it covers. The ledger is the source of record.
- Register write-backs landed in the wave-i closeout commit; tickets below are the next duties round.
- Suggested skills: `$atomcode-research` (new adjudications only, serialized), `$gitbutler` (all VC writes), `code-review` for the audit loop, `grill-with-docs / domain-modeling / neat-freak / handoff` for the next cycle.

## R12-I1 — XFP trust-boundary tighten (D-002)

- `src-tauri/src/headless_security.rs`: `trusted_forwarded_https` — merge all XFP header instances, trim, take LAST (rightmost) segment; value not in {http,https} => fail-closed. Peer-CIDR gate (R12-D1) untouched.
- `src-tauri/src/headless_main.rs`: `parse_trusted_proxy` rejects `0.0.0.0/0` + `::/0` unless a second explicit flag (suggested `--trusted-proxy-unrestricted`) is present; startup info line prints a visible warning when unrestricted.
- Tests: trusted peer + injected `https,http` must NOT yield Secure; `http,https` positive does; double-header-instance cases; flip the codified first-segment test.
- Docs: fix the "fail-closed" doc-comment to describe rightmost/nearest-hop semantics; HEADLESS_DEPLOYMENT.md 215-219 sync.
- Severity: hygiene-required, not a boundary defect (self-inflicted availability quirk). Zero new deps. N>1 trusted-hop depth YAGNI until needed.

## R12-I2 — db-lock-metrics feature + dual threshold (D-003.1)

- Promote the `#[cfg(debug_assertions)]` lock-wait instrument to opt-in cargo feature `db-lock-metrics` (default off = zero-cost retained).
- Dual thresholds: keep >=1ms existence tier, add >=5ms magnitude tier (fixes the fairness-line low-discrimination defect).
- Feature-on builds are for observation cycles only — NOT shipping config; CI adds a `--features db-lock-metrics` verify pass to guard feature-unification drift.
- Holder-identity attribution must land — the D-005 reopen-when condition depends on it.

## R12-I3 — evidence convention enforcement (D-003.2 + D-007.1)

- Closeout already created `docs/agents/evidence/` + README + wave-h backfill.
- This ticket: machine gate — extend or clone the `waiver-ledger-check.mjs` pattern so new trigger-line instrument entries lacking the three declarations (observation environment / zero-read power / evidence class) fail verification.

## R12-I4 — test-quality trio (D-004)

- Move p99<=50ms and the 250ms probe bound out of the push gate into bench.yml artifacts (benchmark bodies still run = correctness regression still caught; duration assertions leave the gate — djust #2157 precedent). On landing, decrement the p95-flake line's covered-assertion count in the register.
- Demote the `contains("TransactionBehavior::Immediate")` source-read to a scripts/ source-gate with comment-stripping + whitespace normalization (keyhog-scanner pattern); add the two-connection behavioral test (A holds write txn -> B replace_ports asserts BUSY/non-upgrade path; strict ordering, no racing).
- Un-gate `file_backed_reads_use_the_dedicated_read_conn`: make it observable via public API or a SEPARATE test feature (not db-lock-metrics — keep the observation-only boundary). Instrument self-check tests stay debug-gated.

## R12-I5 — convergeDevMark comment re-scope (D-003.3)

- Update the instrument comment in `src/store/appStore.ts` (~:241) to declare overlap-only semantics: zero readings prove zero-overlap, NOT churn-free; sequential-duplicate detection requires the parent line to define redundancy semantics first. (Register row already updated in closeout.)

## R12-I6 — strategy_service split, dedicated wave (D-006.1)

- Dedicated duties wave, hard anchor ~2026-10-10. Three steps: (1) shared types first (ReconcileMemory etc. = the seam — ReconcileMemory is NOT pure read-plane; first step is `find_references ReconcileMemory` to confirm seam edges); (2) read plane (plan/get/ReconcilePlan); (3) write plane (apply/reconcile — may merge into the 10-20 batch).
- Scope fence: moves + re-exports only, zero behavior change, same tests before/after; stop rule = read plane + shared types landed. Do NOT touch the 4000-line budget to fit.

## R12-I7 — i18n codegen eval (D-006.2) — 2026-10-20 block

- Evaluate checked-in generated file + verify diff gate (gen script from `src/locales/*/common.json` -> `tray_labels.gen.rs` committed; verify-build regenerates + `git diff --exit-code`). build.rs rejected (OUT_DIR unlintable). First check whether `tauri-plugin-i18n` already solves the shared-source problem.

## R12-I8 — observation roll-call

- s1 second scheduled green (~2026-09-26 09:00Z): close with corroboration note citing both runs (D-007.2).
- convergeDevMark Read#2 on next real GUI session.
- faultinject first scheduled run 2026-10-01 03:00Z.
- 2026-10-20 hard dates: DbPool arm(iv), convergeDevMark dead-fallback (with postmortem note if dead), W8 fallback, i18n-codegen eval.
- Upstream probe issue status (user-side; dormant-fork clock starts at submission).

## Binding negatives (D-001..D-007)

- No external actions under user identity (issue/PR/fork) — user executes.
- No behavior changes inside the split wave; no build.rs for i18n; no new deps for XFP; feature-on builds never ship as delivery config.
- Observation data is not delivery evidence; delivery evidence CI-only.
- No new trigger line for reader-vs-reader (covered-by note on existing line only).
- `AGENTS.md` ~78B headroom — no additions.
