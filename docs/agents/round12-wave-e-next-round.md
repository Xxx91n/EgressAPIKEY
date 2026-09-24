# R12 wave-e — next-round task book

Canonical: `docs/agents/round12-wave-e-next-round.md`
Handoff copy: `.scratch/r12-wave-e-grill/handoffs/next-round.md`
Sole spec source: `.scratch/r12-wave-e-grill/decision-ledger.md` (D-001..D-005, all current)
Produced: 2026-09-24 wave-e grill consolidation. Every task declares its D-xxx coverage.

## 0. Round shape (D-001)

Agenda C = adjudication round + observation write-backs merged. Main ticket: critique #2 Mode A dual-kernel adjudication (the last unadjudicated architecture charge of the second critique). Side tickets: macos-x86_64 delivery-surface verdict + DbPool dead-arm rewrite. Small ticket: residual-minor pair inside the closing batch. Observation items land as register write-backs / ticket notes, never as tickets.

Binding negatives carried by the round:
- No source edits during grilling; all five items are adjudicated specs — implementation is future work.
- Method: per-question code evidence + atomcode terminal adjudication; zero revised records.
- The s1 armed line FIRED this round on machine evidence (runs below) — its adjudicated remedy joins the batch, it is not an open question.

## Fixed points

- Branch `ca-branch-1` tip `482657a6` (wave-d impl + audit fixup); verify `35947178042` green; release-matrix `35882970803` -> draft `r12-waved-ci.1` (6 assets, no x86_64 dmg).
- s1-window-up FAIL signature `documentTitle:"", nativeTitle:null`: `35839072568` (2026-09-23, first - not counted) + `35948350717` (2026-09-24, recurrence -> line fires).
- macos-13 runner image RETIRED 2025-12-04/08 (GitHub changelog); `macos-15-intel` is the last Intel image, retires 2027-08.
- `port_forwarder.rs` 1413 lines (Mode A legislated dataplane); `db.rs` 614 lines; `e2e-webview/smoke.mjs:131` one-shot `getTitle()` = the only non-polled assertion in the script; `ip_reputation.rs:30` `parse()` = 7-line alias fn, no direct test.
- parking_lot `Mutex` has no wait-time API; eventual-fairness forcing ~= 1 ms (0.5 ms avg) — the dev-counter threshold anchor.
- r2d2 is 404-dead on crates.io (ADR-0011 note, 2026-08-03) — pool successor = `deadpool-sqlite`.
- ADR-0068 D4: TLS fronting explicitly out of scope (cite verbatim, do not re-legislate); D2 = protocol-family freeze origin.
- AGENTS.md 32 KiB ceiling armed: any net-addition trips the verify gate; wave-e doc edits must be net-negative or externalized.
- Research artifacts: `.codex-tmp/atomcode-wavee-q{2,3,4,5}-prompt.txt` + `.codex-tmp/atomcode-wavee-q{2,3,4,5}-report.md`.

## R12-E1 — Mode A boundary-law registration + CI assertions (D-002, main ticket)

Outcome: Mode A is NOT a second-kernel violation — ADR-0068 legislated it. Register existing law + boundary law; this is NOT a new amnesty (wording discipline so future readers do not mistake Mode A for drift).

Deliverables:
1. Handler census of `port_forwarder.rs` -> frozen inventory of handlers/rewrite rules/StreamSensor dimensions = the CI assertion baseline.
2. `mode-a-contract` gate scenario-matrix assertions for the three bans: protocol-family freeze (rewrite whitelist = {socks5-handshake, http-connect, absolute-form->CONNECT}); no TLS fronting (cite ADR-0068 D4 verbatim); no new L7 features (new dataplane needs go to the fork line).
3. Register row landed in the wave-e closeout: armed line, 4+1 triggers — (a) VPS credential-free demand; (b) Mode B checkpoint defect outside Registered Exemption; (c) net-new dataplane behavior (new handshake dialect / new rewrite rule / new StreamSensor dimension; line count = warn-only reference, not the trigger); (d) Resin ships a StreamSensor-usable tap endpoint -> Mode A decommission auto-starts; (e) collected ADR-0079 orphan line (upstream ships native port-identity -> re-open).
4. Acceptance criteria MUST carry the reverse pointer to R12-E4: boundary-law CI assertions are delivered HERE; E4 register write-back only references them (mutual pointers, no orphaned assertions).
5. Closed record landed in closeout: edge-adapter-as-separate-process evaluated and rejected.

Negatives: no upstreaming now; no crate extraction; no thin-shell regression (revoking ACCEPTED ADR-0068 is out of ticket authority); StreamSensor stays shell-side; Mode B default untouched.

## R12-E2 — macos-x86_64 input-gate + macos-15-intel migration (D-003)

Deliverables:
1. `.github/workflows/ci.yml` release-track jobs (backend `:119` + gui `:214`): `os: macos-13` -> `macos-15-intel`, plus a job-level `if:` on a dispatch input `inputs.include_intel_mac` (default off; non-Intel legs always build).
2. Raise per-job `timeout-minutes` (Intel-runner builds are empirically slow).
3. Docs — RELEASE_NOTES + README download table + AGENTS.md artifact list: "macOS = arm64 (Apple Silicon) first-class; x86_64 = opt-in best-effort until 2027-08 (macOS 15+ Intel only)". AGENTS.md edit must be net-negative.
4. ADR-0010: append a Revision-section note (precedent exists) — no new ADR.
5. Verify the draft-release collect step does not hard-assert the x86_64 dmg exists.
6. Register rows landed in closeout: hard-date 2027-08-01 line + >=3 non-bot-issue demand line + universal2 succession note (arm64 runner + x86_64 target + lipo; designated successor, NOT implemented now).

Negatives: delete the impossible signal (GitHub restoring free Intel capacity); no paid larger runner; no doc-only change; not A1 today (kills a real escape hatch early).

## R12-E3 — DbPool dead-arm rewrite: dev counter + synthetic probe (D-004)

Deliverables:
1. dev-only lock-wait counter on DbPool acquisition: `Instant`-wrapped, >=1ms threshold (anchored to parking_lot eventual-fairness forcing line), DEV-gated / zero-cost in prod (convergeDevMark pattern).
2. Synthetic concurrency probe: cargo test, 8 threads hammering `list_ports` x N rounds, measure wait p99 — upper-bound proof, CI-runnable, zero runner dependency.
3. Register rewrite landed in closeout: four arms (i) dev >=1ms real load -> reopen (successor deadpool-sqlite; r2d2 404-dead recorded); (ii) probe p99 >=1ms -> same; (iii) dev counter zero for two cycles AND probe p99 <1ms -> closed verdict; (iv) hard date 2026-10-20 verdict MUST land regardless of runner.
4. Recorded observation landed in closeout: read-path spawn_blocking asymmetry (write path already exempt per ADR-0011; tokio blesses short holds — no trigger line).

Negatives: no RwLock / FairMutex / hand-rolled mini-pool; no r2d2; no lock-architecture change. The probe is an existence-of-contention instrument; the selfhosted `dagger` line still owns absolute-latency thresholds.

## R12-E4 — closing batch (D-005)

1. s1 fix — `e2e-webview/smoke.mjs` s1 block ONLY: wrap `getTitle` in `waitUntil` (or a poll matching the repo `waitPort` style): title === "EgressAPIKEY" or 30s timeout, 500ms interval. Non-masking conditions verbatim in the ticket/PR: (a) same criterion, not relaxed; (b) bounded timeout still FAILs and `detail` records the last observed value plus the title evolution across polls; (c) do NOT widen S2-S5 or waitPort/nativeTitle waits. Rejected remedies: workflow-level retry, longer timeouts.
2. `ReputationProvider::parse` table test (`ip_reputation.rs` mod tests): all aliases for the three providers, mixed case (e.g. "IPQS"), surrounding whitespace trim, unknown -> None. AGENTS section-4 compliance debt.
3. `subName` private — record only, no ticket.
4. Register write-back package — EXECUTED in the wave-e closeout commit (7 items): s1 fired + remedy wording; Mode A registered-legislation row; DbPool four-arm rewrite (old runner-gated text fully replaced); macos hard-date + demand + universal2 note; spawn_blocking observation; edge-adapter closed verdict; observation notes (convergeDevMark, faultinject first-run dry-run echo check, D3 warn = data, W8 user-side decoupled). Ticket file only references them.
5. Boundary discipline: Mode A CI assertions are E1-owned — E4 acceptance names the mutual pointer.
6. `codegraph sync .` in the same session as the parse-test commit.

## Ordering & operating notes

- E1..E4 are file-disjoint: E1 = port_forwarder / mode-a-contract gate; E2 = ci.yml + docs; E3 = db.rs + dev-counter wiring; E4 = smoke.mjs + ip_reputation.rs. Register write-backs are already committed — no register write conflicts between tickets.
- CI-only policy (ADR-0072): no local build/test; acceptance = the CI verify job.
- Grill closeout suggestion (procedural, not a ledger decision): read `docs/adr/0080` in full before landing E1/E3 — wave-d precedent.
- After source changes: `codegraph sync .` (recommended, not a gate).
- Version control: GitButler `but`; `.scratch/` is gitignored — session artifacts never commit.

## Suggested skills

- `implement` / `tdd` — E1-E4 implementation tickets (test-first where behavior changes).
- `code-review` — pre-merge two-axis review (the wave-d audit pattern).
- `atomcode-research` — any further external adjudication (serial only).
- `gitbutler` (`but`) — all commits/pushes.
- `neat-freak` — closeout after landing.
- `handoff` — next round handoff document.
