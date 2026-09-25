# Round 12 Wave-H — Next-Round Taskbook

Slug: `r12-wave-h-grill` | Baseline: `origin/main` at closeout (was 5a146f1d at grill start)
Ledger: `.scratch/r12-wave-h-grill/decision-ledger.md` — D-001..D-005, all current.
Reports: `.codex-tmp/atomcode-waveh-q2..q5-{prompt,report}.md`

## How to use this taskbook

- Each task declares the D-xxx IDs it covers. The ledger is the data source of record; this file operationalizes it.
- Register write-backs for D-004/D-005 land in the same closeout commit as this file — code/config tickets below are next-round work.
- Suggested skills: `$atomcode-research` for any new adjudication; `$gitbutler` for all version-control writes; `grill-with-docs / domain-modeling / neat-freak / handoff` for the next re-grill cycle.

## R12-H1 — Resin upstream-first delivery package (D-002, D-003)

Produce locally, in the ignored `resin/` clone (Go 1.26.2 on host; go.mod wants 1.25.5):

1. Patch `resin/internal/proxy/forward.go` (~L294): replace the bare `io.Copy(w, resp.Body)` with an SSE-aware conditional flush writer — on `Content-Type: text/event-stream`, write then `Flush()` via `http.Flusher` assertion; non-SSE path byte-identical. Commit message: `forward: flush SSE responses per event`.
2. Tests in upstream's existing `resin/internal/proxy/forward_http_copy_test.go`: per-event arrival assertion + non-SSE invariance assertion. Verify: `go build ./...` and `go test ./internal/proxy/`.
3. Artifacts doc `docs/agents/wave-h-upstream-package.md`:
   - PR title + body: symptom (SSE buffering 2-4KB windows) -> root cause at `forward.go:294` -> fix shape -> test evidence -> AI-disclosure line ("drafted with AI assistance; verified and defensible by submitter").
   - Probe-issue draft in Chinese, ending with a question such as "如欢迎愿发 PR？".
   - User checklist: post probe issue -> await maintainer response -> rejection or stall >1 upstream release cycle (monthly cadence) -> dormant patch-fork trigger per D-002.

Negatives: do NOT create the fork repo, open the issue, or open the PR — external actions under the user's GitHub identity are user-side (D-003.4). Do not pre-build Day-1 fork infra before the trigger fires. The dormant-fork Day-1 archive (fresh repo, main rebases upstream tags, copied release.yml, manifest repoint, THIRD_PARTY.md line) lives in D-002 and activates only on trigger.

## R12-H2 — DbPool pool-impl evaluation ticket (D-004) [only code-touching item]

- step0: extend the existing dev-gated `db.rs` lock-wait WARN with holder identity (one-line increment).
- step1: B-read-conn prototype — a long-lived `LazyLock` read-only `Connection` serving `list_ports`/`get_port` (hynek caveat: MUST be long-lived; short-lived readers hit SQLITE_BUSY on open/close) + A/B measurement in a real dev session.
- step2: if contention persists -> formal `deadpool-sqlite` evaluation.
- Optional (not a prerequisite): `db-lock-metrics` feature-gate variant for release-domain corroboration of arm(iii).

Negatives: evaluation only — no migration commitment; do not retroactively rewrite arm text; arm(iii)/(iv) unchanged.

## R12-H3 — pnpm configuration repair (D-005.2)

- Resolve `pnpm-workspace.yaml` `allowBuilds` placeholders -> `{esbuild: true, edgedriver: true, geckodriver: true}` (verify with `pnpm why edgedriver geckodriver`; `false` is also legitimate if CI never runs the webdriver smoke — decide at implementation).
- Delete `package.json` `pnpm.onlyBuiltDependencies` — dead under pnpm 11 (package.json pnpm field unread).
- yaml + lockfile in one commit. NEVER `dangerouslyAllowAllBuilds`.
- First implementation step: align pnpm versions host vs CI (`pnpm --version` on both; the CI-green/host-red divergence is likely CI on pnpm10 silently skipping v11 config).
- Verify: `pnpm install` + relevant build/test on host.

## R12-H4 — observation roll-call (carried; no new decisions)

- s1 holding window: count scheduled cron greens only (cancelled excluded; run 35976721262 precedent). Watchdog was capped ~10:02Z 2026-09-25 — restart if expired.
- convergeDevMark Read#2: the next real dev-session read; unread != zero.
- faultinject first scheduled run: 2026-10-01 03:00Z (verify PHASES_ARG/HEADLESS_ARG in dry-run echo log).
- Hard dates 2026-10-20: DbPool arm(iv) verdict, convergeDevMark dead-fallback, W8 fallback.
- Upstream probe issue status check if the user has posted it (R12-H1 artifact).

## Binding negatives (D-001..D-005)

- No capability/product-grade fork work until the Fork Line pinned trigger fires (per-request model-field-driven egress switching within a single account / node-level pinning).
- No external actions under the user's identity (issue/PR/fork creation) — user executes; agent only prepares.
- ADR-0067 D3 (loopback-REST mere aggregation) and ADR-0079 (engine-is-a-seam) unchanged; no new L7 in Mode A; protocol-family freeze holds.
- Observation data is not delivery evidence; delivery evidence remains CI-only (standing ruling, register R12-H section).
- `AGENTS.md` ~78B headroom to the 32KiB cap — do not add text.
