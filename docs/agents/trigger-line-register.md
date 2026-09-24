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
| 2 | Adoption evidence: any non-English locale >=15% of usage (requires telemetry/download-channel data if it ever exists) | telemetry if introduced — dead arm while no telemetry exists (permanently unevaluable; noted r12-wave-d D-005) |
| 3 | Maintenance friction: tray.rs reworked for locale add/remove >=2x in a year, OR i18n-check CI fails >=1x/month | `git log -- src-tauri/src/tray.rs`, CI failure rate |
| 4 | Quality falsification: manual inspection finds RTL/glyph/overflow-class defects -> clean retirement discipline (delete dir + gate entry + tray shrink; no orphaned translations) | e2e ar/hi/th render assertions + manual pass |

## Other registered trigger lines

| Item | Trigger line | Status |
|---|---|---|
| TopologyView shallow-{} selector refactor (⑤c) | file churn: next substantive edit to the selector lines re-opens it | parked |
| tray i18n hand table | covered by i18n line 3 above | parked |
| Mode A standalone crate extraction | deferred, no line — revisit only if the forwarder outgrows `bench`/app coupling | parked |
| `url_encode_segment` triplication (resin_client.rs x2 + headless_main.rs) | touch-the-code rule: next edit to any of the three sites consolidates into a shared helper | **resolved 2026-09-23** (r12 wave-d R12-D2): consolidation positively discharged (touch-rule fired on the three named sites) - `resin_core::encoding` owns `encode_path_segment` (strict) + `encode_uri_component` (JS parity); the ip_reputation form-vs-path bug was fixed in commit 1 |
| React Query migration (R12-01) | (a) concrete correctness/perf win found — measured duplicate-fetch or retry/ordering need; (b) a feature must touch polling + dispatch + write seams anyway | measuring (r12-wave-d D-005): R12-D4 lands a dev-only in-flight-overlap counter on both `ipcAuthoritativeSnapshot` trigger paths; >=1 measured overlap fires arm (a); two observation cycles at zero -> back to suspended with verdict recorded (docs/research/react-query-dual-channel-eval.md) |
| bench-selfhosted `†` provisional thresholds | representative-hardware self-hosted run lands -> values become real gates | parked pending dispatch |
| webview-smoke workflow | first post-merge run proves the stack; promote failure handling after evidence | **resolved 2026-09-23**: merged-state first run 35755345313 both legs green after R12-B3 selector fix (s1..s5 all PASS) - trigger fired and positively discharged |
| AGENTS.md 32 KiB ceiling | 32690/32768 B (78 B headroom, 2026-09-24 remeasure; supersedes stale 32756 figure): ANY net-addition trips `scripts/verify-build.sh`'s hard FAIL — the next substantive edit MUST externalize detail into `docs/agents/` (the §"Architecture & Phase History (externalized)" pattern) and leave a pointer. Do not raise the guard: 32 KiB is a consumer-side `project_doc_max_bytes` truncation limit, not an editorial choice | armed — trips on the next net-addition |
| pending.diff/reason English strings in UI (F-10) | orchestration pending payloads render Rust English text raw — i18n-ify when a second consumer or a locale-quality complaint lands | recorded observation |
| NODE_FC counter-window semantics (C8, r12-wave-b D-003) | Degraded platform finds ZERO candidates because every node flagged err >=1 occurrence -> rework to timestamped fixed-duration sliding window; verdict: tolerable-with-hole (conservative over-exclusion, no wrong switches). NOTE: no node-level self-heal path exists (cooldown is platform-level; unlike Envoy max_ejection_percent there is no evacuation floor) - the trigger IS the safety net | registered-exemption, armed |
| strategy_service split preemption (r12-wave-b D-001, extends wave-a D-003-7) | line-count headroom < 200 (currently 3636/4000 = 364 headroom) OR a feature ticket must touch the file -> split ticket preempts the feature ticket | parked, armed — critique `.codex-tmp/锐.md` logged as independent external evidence (r12-wave-d D-001); thresholds unchanged |
| paired_request_added_latency_p95 flake (C9) | empirical count: >=2 observed flakes in CI -> promote to fix ticket (isolated passes; timing-sensitive under parallel load) | observing (0 confirmed hits this round as of 2026-09-22 - the only observed CI failure was webview-smoke, a distinct harness defect fixed under R12-B3) |
| forwarder port-scan O(n) suggestion path (critique #2) | measured suggestion-latency regression OR >=1 allocation-conflict event in a real deployment -> ticket | armed (r12-wave-d D-005) |
| DbPool single-lock (bb8) (critique #6) | arm (ii) FIRED 2026-09-24 (probe p99=9615us/200 samples, run 35959139983) -> adjudicated r12-wave-f D-002: **keep-Mutex verdict** (synthetic saturation contention != production impact; Mutex-writer topology externally confirmed: emschwartz 95ms vs split-pool 83ms; abseil microbench discipline). Remaining arms: (i) dev lock-wait counter = PRIMARY instrument - real-load wait >=1ms -> reopen pool-impl (successor deadpool-sqlite, arm(i)-gated, B-read-conn first then A; r2d2 404-dead); (iii) dev counter zero for two observation cycles -> closed verdict; (iv) hard date 2026-10-20: verdict MUST land - no arm(i) corroboration -> closed-verdict archive | armed - arms i/iii/iv live, arm (ii) consumed; probe demoted to regression/A-B instrument (r12-wave-f D-002; supersedes r12-wave-e D-004/r12-wave-d D-005 wording) |
| backup envelope versioning (critique #7) | any envelope format change need (Argon2 params, wrapped_key shape) -> versioned-envelope ticket | parked, armed (r12-wave-d D-005) |
| ubuntu s1 waitForTitle flake | baseline: merged-state run 35755345313 both legs green; recurrence = the same s1 step fails again AFTER merged-state green (first failure never counts as recurrence) -> add retry | **fired 2026-09-24** (r12-wave-e D-005): s1-window-up FAIL identical signature (documentTitle:"", nativeTitle:null) on 35839072568 (9-23, first - not counted) then 35948350717 (9-24, recurrence). Remedy adjudicated: in-probe polling (waitUntil 30s/500ms); workflow/step retry REJECTED (masking analysis: Google 2016 + Mergify) -> implementation R12-E4 |
| monthly faultinject schedule heartbeat | monthly faultinject run absent >=2 consecutive occurrences (GitHub disables schedules after 60 days repo inactivity) -> review line fires | armed (r12-wave-d D-005) |
| Mode A shell forwarder boundary law (r12-wave-e D-002) | fires on ANY of: (a) VPS profile gains a credential-free entry-port requirement; (b) Mode B develops a checkpoint defect outside Registered Exemptions; (c) forwarder net-new dataplane behavior - new handshake dialect OR new request-rewrite rule OR new StreamSensor classification dimension (line count = warn-only reference, not the trigger); (d) Resin ships a StreamSensor-usable event/tap endpoint -> Mode A decommission procedure auto-starts; (e) upstream ships native port-identity (ADR-0079 Consequences - collected orphan line) -> re-open upstreaming adjudication | armed - registered-legislation record: Mode A is a legislated dataplane (ADR-0068 D1/D3), not drift; boundary law = protocol-family freeze {socks5-handshake, http-connect, absolute-form->CONNECT} + no TLS fronting (ADR-0068 D4 verbatim) + no new L7 features; CI assertions land with R12-E1; census extractor broadened to full qualifier space + declarative-macro fail-closed (r12-wave-f D-003, impl R12-F1) |
| macos-x86_64 delivery demotion (r12-wave-e D-003) | hard date 2027-08-01 (macos-15-intel retirement) -> A2 input-gate auto-escalates to A1 full-removal adjudication; demand line >=3 non-bot issues requesting Intel dmg -> re-entry adjudication; universal2 dmg (arm64 runner + x86_64 target + lipo) recorded as designated successor when the hard date fires (not implemented) | armed (implementation: R12-E2 input-gate + macos-15-intel migration) |
| DbPool read-path unlock (P3, r12-wave-f D-002) | measured headless/VPS read latency attributable to the shared lock (NOT synthetic-probe data) -> land option B: a long-lived LazyLock read-only Connection serving list_ports/get_port (hynek -shm warning: MUST be long-lived - short-lived readers hit SQLITE_BUSY on open/close; snapshot_db_readonly precedent is file-level snapshot, semantics not transferable) | parked - deferred from wave-f adjudication, not prepaid |
| census extractor AST upgrade (r12-wave-f D-003) | ANY of: (a) first proc-macro dependency lands in resin-core; (b) census expands to 3+ files; (c) >=2 regex qualifier-shape fixes within a year -> re-adjudicate AST extraction (syn/rustdoc JSON). Rejected now: nightly toolchain coupling + format drift (cargo-public-api #858, rust #135600) violates ADR-0072 minimal CI surface | armed |

## R12-04 closed observations (verdicts, no trigger line needed)

- **F-7 key-in-URL**: FIXED — `/api/v1/shell/key-lookup` moved GET `?key=` -> POST JSON body (credential material stays out of reverse-proxy access logs).
- **F-8 SIGNALS map leak**: FIXED — tick now retains rings to configured platforms only.
- **F-9 approve_impl swallow**: FIXED — `svc.apply` failure now propagates as IpcError; pending proposal stays parked instead of silently advancing phase on a divergent write.
- **F-11 e2e remove assertion**: CLOSED in R12-00 (webview-smoke assertion set).
- **F-12 ADR wildcard link**: CLOSED in r11 rework.
- **F-13 dual 60s drivers**: ACCEPTED — the headless vs desktop driver shapes intentionally diverge (independent cadence loops); duplication is 6 lines of spawn boilerplate, not shared logic.
- **Narrative tokens**: CLEARED — `T*-*/round*/ticket *` markers in production `src/` comments went 26 -> 0 (test names keep ids as regression anchors).
- **resolve_id double-RTT (critique #2)**: VERDICT r12-wave-d D-005 — already adjudicated (ADR-0071 D-002-2, r11-wave-b 2026-09-20: the BFF name-to-id resolution extra control-plane RTT was Accepted). Not reopened.
- **dead-semantics manifest commands (critique #9)**: VERDICT r12-wave-d D-005 — already adjudicated (AGENTS §7.6 + CONTEXT.md `Echo Command`: echoes are alive contract surface; deletion condition = Resin account REST surface change). Rejected.
- **i18n-check Rust-side blind spot**: OBSERVATION r12-wave-d D-005 — i18n-check scans the frontend only; Rust-side strings (tray, headless) uncovered. Extend the probe cheaply if it ever bites.

## R12-E observations and verdicts (r12-wave-e D-005 write-back)

- **read-path spawn_blocking asymmetry (D-004)**: VERDICT closed 2026-09-24 (r12-wave-f D-002) - harmless exposure confirmed by adjudication: tokio blesses short critical sections, write path already exempt via spawn_blocking (ADR-0011). No alignment work; not a trigger line.
- **edge-adapter as separate process (D-002)**: VERDICT - evaluated, not adopted. Mode A as a standalone process adds a second supervised lifecycle on desktop (Ghost already guards the sidecar) and StreamSensor needs in-process access. Recorded to prevent re-proposal.
- **D3 back-verify warn**: OBSERVATION (carried from wave-d audit) - fires only on a real hash-shape discontinuity (e.g. pre-fix logs); first sighting = data, not alarm.
- **convergeDevMark overlap counter**: OBSERVATION - dev-only counter live on all three trigger paths (R12-D4); >=1 measured in-flight overlap fires React-Query arm (a); two observation cycles at zero -> record "negative, stays SUSPENDED".
- **monthly faultinject first run**: OBSERVATION - first scheduled run 2026-10-01 03:00 UTC; verify PHASES_ARG/HEADLESS_ARG resolution in the dry-run echo step job log; two consecutive absences fire the heartbeat line above.
- **W8 bench-selfhosted runner**: OBSERVATION - registration is a user-side action; the DbPool line is decoupled from it (r12-wave-e D-004); the dagger provisional-thresholds line still depends on it (fallback 2026-10-20).

## R12-F observations and verdicts (r12-wave-f write-back)

- **s1 post-land verification pending (D-001)**: OBSERVATION - webview-smoke runs nightly (cron 03:30 UTC, main only); first post-land run must show `s1-window-up` green via the new poll with `titleEvolution` in detail output. A repeat failure re-adjudicates the remedy itself.
- **probe hang-pathology bound (audit)**: VERDICT - permanent `p99 < 250ms` assert on the lock-wait probe is beyond the literal E3 ticket text; auditor-accepted as defensible instrument sanity. Recorded, no action.
- **impl-session local rustc cfg-harness (audit)**: OBSERVATION - disclosed minor ADR-0072 deviation (standalone rustc cfg sanity probe, not a deliverable build); all delivery evidence stayed CI.
- **wave-e lanes awaiting user land (D-001)**: OBSERVATION - workspace at `ae9ecb29` = tag `r12-wavee-ci.1`; lanes `r12-e1..e4` + `ca-branch-1` applied; integration lane disposable post-land per section-4.2 (no agent merge). Landing is a user-side action.

## Procedure

- Per-round grill: re-check each line's probe column; mark hits.
- A hit line re-enters the normal ticket filter (D-003 four-tier rule).
- Adding a suspended item without a named trigger line is a ledger error.
