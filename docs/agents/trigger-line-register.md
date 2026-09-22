# Trigger Line Register (触发线登记册)

Living register of every suspended/trigger-line-governed item
(CONTEXT.md `Trigger Line`): parked until a machine-checkable condition
fires, then re-enters adjudication. Covered by the per-round grill
inspection obligation (r12-wave-a D-003⑨) — no entry may become an
unowned orphan.

## i18n locale revisit lines (D-005)

Scope: the 18-locale surface is machine-translated, no human owner.
Re-open the trim adjudication when ANY line fires:

| # | Line | Machine-checkable probe |
|---|---|---|
| 1 | Demand signal: >=3 independent non-bot issues requesting a locale other than en/zh (or reporting quality on it) | issue tracker search per round |
| 2 | Adoption evidence: any non-English locale >=15% of usage (requires telemetry/download-channel data if it ever exists) | telemetry if introduced |
| 3 | Maintenance friction: tray.rs reworked for locale add/remove >=2x in a year, OR i18n-check CI fails >=1x/month | `git log -- src-tauri/src/tray.rs`, CI failure rate |
| 4 | Quality falsification: manual inspection finds RTL/glyph/overflow-class defects -> clean retirement discipline (delete dir + gate entry + tray shrink; no orphaned translations) | e2e ar/hi/th render assertions + manual pass |

## Other registered trigger lines

| Item | Trigger line | Status |
|---|---|---|
| TopologyView shallow-{} selector refactor (⑤c) | file churn: next substantive edit to the selector lines re-opens it | parked |
| tray i18n hand table | covered by i18n line 3 above | parked |
| Mode A standalone crate extraction | deferred, no line — revisit only if the forwarder outgrows `bench`/app coupling | parked |
| `url_encode_segment` triplication (resin_client.rs x2 + headless_main.rs) | touch-the-code rule: next edit to any of the three sites consolidates into a shared helper | parked |
| React Query migration (R12-01) | (a) concrete correctness/perf win found — measured duplicate-fetch or retry/ordering need; (b) a feature must touch polling + dispatch + write seams anyway | suspended (docs/research/react-query-dual-channel-eval.md) |
| bench-selfhosted `†` provisional thresholds | representative-hardware self-hosted run lands -> values become real gates | parked pending dispatch |
| webview-smoke workflow | first post-merge run proves the stack; promote failure handling after evidence | **fired 2026-09-22**: run 35730121030 FAILED (s2 net-save-btn 8s timeout / s4 remove click path; report-only mode) -> triage ticket R12-B3 |
| AGENTS.md 32 KiB ceiling | 32756/32768 B (12 B headroom, 2026-09-22): ANY net-addition trips `scripts/verify-build.sh`'s hard FAIL — the next substantive edit MUST externalize detail into `docs/agents/` (the §"Architecture & Phase History (externalized)" pattern) and leave a pointer. Do not raise the guard: 32 KiB is a consumer-side `project_doc_max_bytes` truncation limit, not an editorial choice | armed — trips on the next net-addition |
| pending.diff/reason English strings in UI (F-10) | orchestration pending payloads render Rust English text raw — i18n-ify when a second consumer or a locale-quality complaint lands | recorded observation |
| NODE_FC counter-window semantics (C8, r12-wave-b D-003) | Degraded platform finds ZERO candidates because every node flagged err >=1 occurrence -> rework to timestamped fixed-duration sliding window; verdict: tolerable-with-hole (conservative over-exclusion, no wrong switches). NOTE: no node-level self-heal path exists (cooldown is platform-level; unlike Envoy max_ejection_percent there is no evacuation floor) - the trigger IS the safety net | registered-exemption, armed |
| strategy_service split preemption (r12-wave-b D-001, extends wave-a D-003-7) | line-count headroom < 200 (currently 3575/4000) OR a feature ticket must touch the file -> split ticket preempts the feature ticket | parked, armed |
| paired_request_added_latency_p95 flake (C9) | empirical count: >=2 observed flakes in CI -> promote to fix ticket (isolated passes; timing-sensitive under parallel load) | observing |

## R12-04 closed observations (verdicts, no trigger line needed)

- **F-7 key-in-URL**: FIXED — `/api/v1/shell/key-lookup` moved GET `?key=` -> POST JSON body (credential material stays out of reverse-proxy access logs).
- **F-8 SIGNALS map leak**: FIXED — tick now retains rings to configured platforms only.
- **F-9 approve_impl swallow**: FIXED — `svc.apply` failure now propagates as IpcError; pending proposal stays parked instead of silently advancing phase on a divergent write.
- **F-11 e2e remove assertion**: CLOSED in R12-00 (webview-smoke assertion set).
- **F-12 ADR wildcard link**: CLOSED in r11 rework.
- **F-13 dual 60s drivers**: ACCEPTED — the headless vs desktop driver shapes intentionally diverge (independent cadence loops); duplication is 6 lines of spawn boilerplate, not shared logic.
- **Narrative tokens**: CLEARED — `T*-*/round*/ticket *` markers in production `src/` comments went 26 -> 0 (test names keep ids as regression anchors).

## Procedure

- Per-round grill: re-check each line's probe column; mark hits.
- A hit line re-enters the normal ticket filter (D-003 four-tier rule).
- Adding a suspended item without a named trigger line is a ledger error.
