# R12 Wave-D Next Round — Task Book

> Consolidated 2026-09-23 from `.scratch/r12-wave-d-grill/decision-ledger.md` (D-001..D-005, all `current`).
> Canonical copy: `docs/agents/round12-wave-d-next-round.md`; session artifact copy: `.scratch/r12-wave-d-grill/handoffs/next-round.md`.
> Ledger is the sole decision source. Do not re-derive dispositions from memory.

## §0 Round shape (D-001)

Wave-d = critique-adjudication round, complete. Output = four implementation tickets (R12-D1..D4) + register write-backs (applied at consolidation, see §5).

Carried negatives (binding):
- No armed/parked register line fires without new machine-checkable evidence; an external critique mentioning line counts is evidence logged, not a trigger.
- W8 product increment stays deferred (r12-wave-a D-003⑨ defer clause; bench-selfhosted fallback 2026-10-20 not reached).
- No unmeasured migrations: r2d2/deadpool, React Query, converge rewrite — measurement lines exist, tickets do not.
- resolve_id double-RTT / first-byte protocol detection / echo stubs are adjudicated (closed-observation verdicts, §5).
- If ANY ticket must touch `strategy_service.rs`, the feature-touch arm fires naturally — split ticket preempts.
- Pre-implementation gate: read ADR-0080 in full before R12-D3/R12-D4 land (legislated D-004/D-005).

## §1 R12-D1 — X-Forwarded-Proto trust boundary (P1, security fast lane) [covers D-002]

Surface: `src-tauri/src/headless_main.rs` `security_guard` unconditionally trusts `X-Forwarded-Proto` -> forgeable `; Secure` cookie suffix on plain-HTTP VPS profile.

Spec:
- Default: XFP fully ignored; `forwarded_https` true only when peer IP ∈ trusted set AND XFP first scheme == https.
- `--trusted-proxy <ip-or-cidr>` repeatable clap flag -> `Vec<IpNet>`; **`ipnet` declared as direct dependency** (lockfile-transitive does not excuse declaration). Exact IP covers loopback proxies; CIDR covers docker/k8s dynamic peer ranges.
- `axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())`; `security_guard` gains `ConnectInfo<SocketAddr>` extractor.
- Pure fn `trusted_forwarded_https(peer, trusted, xfp) -> bool`: IPv4-mapped-IPv6 normalization via `to_ipv4_mapped()` (NOT `to_ipv4` — maps `::1`->`0.0.0.1`); XFP list values take FIRST scheme (client-nearest, ASP.NET semantics), fail-closed.
- Forged XFP silently dropped (per-request warn = flood vector; optional `tracing::debug!`); startup info line prints trusted-proxy count.
- `0.0.0.0/0` = operator footgun: docs warning only, no hard block.
- ADR-0071: appended errata (amend-not-rewrite) recording why socket-peer model was chosen over XFF chain (guard only needs "did a trusted proxy relay this", not real client IP). Deployment doc gains the flag; CHANGELOG/RELEASE_NOTES behavior-change line in the SAME commit.
- Tests: 8 pure-fn cases — no flag ignores / untrusted peer ignores / trusted+https -> Secure / trusted+no-header -> none / XFP=http -> none / trusted+`https,http` list / v6-mapped `::ffff:127.0.0.1` hits v4 entry / CIDR hit+miss.

Negatives: no deps beyond ipnet; desktop IPC/manifest/`session_cookie()` untouched; AGENTS.md zero delta (12 B headroom, armed line — flag docs go to `docs/`); XFP read moves near `plant_cookie` block so both reads stay colocated.

## §2 R12-D2 — path-segment encoding consolidation (P2 correctness) [covers D-003]

Surface: `ip_reputation.rs:137` builds IPQS URL `.../api/json/ip/{key}/{ip}` with `form_urlencoded::byte_serialize` — space->`+` is literal `+` inside a path segment -> auth fails on space-bearing keys.

Spec (one ticket, TWO commits):
- New module `crates/resin-core/src/encoding.rs` exports `encode_path_segment()` (strict set) + `encode_uri_component()` (JS-parity set, name retained — account-header-rule contract tests already prove that set). Name `urlencoding` is BANNED (its form-vs-path ambiguity caused this bug).
- commit-1 (minimal fix): ip_reputation -> `encode_path_segment` + IPQS URL-construction regression assertions.
- commit-2 (byte-equivalent mechanical migration): the three named path-segment sites (resin_client `urlencoding`, headless_main `url_encode_segment`) -> `encode_path_segment` + `encode_uri_component` moved into the module. Query-value call sites KEEP the strict set (over-encoding always safe; byte change deferred to a real caller).
- Register discharge wording: "consolidation positively discharged (touch-rule fired on the three named sites)" — discharge hangs on commit-2.
- Tests: space->%20, +->%2B, /->%2F, non-ASCII UTF-8, IPQS URL regression, `encode_path_segment`->headless `percent_decode` round-trip.

Negatives: zero new deps; AbuseIpDb `parse_with_params` / raw `Key` header / `percent_decode` untouched; zero CONTEXT terms; zero AGENTS.md delta; do not assert IPQS server `%20` tolerance without credentials (worst case not worse than today).

## §3 R12-D3 — audit-chain robustness (narrowed) [covers D-004]

Surface: `crates/resin-core/src/audit.rs`. Two critique claims disproven at code read (field-order fragility — chain hashes STORED bytes not reserialized events; `.tmp` residue — none exists; `written_bytes` already seeds from `metadata.len()` at open). Real residue below.

Spec:
- Tail adoption: `last_line_hash` candidate check `from_str::<Value>` -> `from_str::<AuditEvent>` (valid-JSON-but-not-an-event no longer becomes chain tail).
- Optional single-link back-verify warn: adopted tail's `prev_hash` recomputed against the preceding complete event row; mismatch -> still adopt (forward continuity) + `tracing::warn!` (ADR-0059 D7: warn never blocks).
- `written_bytes`: re-seed from `metadata.len()` after EVERY append; on stat failure keep old value (no zero, no error). Rotation sets 0 -> next append re-seeds (self-consistent).
- ADR-0059 errata CORRECTS D3 wording (it currently teaches verifiers wrong): (1) hash = sha256(stored line bytes + `\n`), verifiers forbidden from reserializing; (2) torn tail bytes preserved, never truncated; (3) mid-stream garbage tolerance contract — verifier MUST accept "garbage line mid-stream, chain links over it"; (4) backward-walk skip semantics. Plus one-line disclosure: directory-entry fsync absent after create (best-effort per D7, file could vanish not torn).
- Module doc: single-writer invariant — `append` reads `last_hash` and writes back across the I/O in two critical sections; safe today under the process-global `AUDIT` Mutex; before any future multi-writer wiring the prev_hash read-modify-write must move into one critical section (documented, NOT a ticket).
- Tests: tail injection x4 (valid event / valid-JSON-non-event / torn / empty) + `AuditLog::new` counter == metadata.len() + forensic-preservation assert (garbage bytes still present AND new row prev_hash == last complete event hash) + back-verify warn path.

Negatives: no marker rows; no serialization change; no streaming/incremental writes; no offset file; audit.rs + ADR-0059 errata are the only touched surfaces; zero IPC/manifest/AGENTS/CONTEXT.

## §4 R12-D4 — closing batch: hygiene + faultinject monthly + dev counter [covers D-005]

### 4a Hygiene items
- `PlatformsView.tsx` A||A dead disjuncts :554/:589/:590 — pure deletion; verified :590 `protocol_mismatch` lives in the else chain and stays reachable. Ticket keeps an else-chain reachability check with ceiling = no ternary refactor.
- `" selected"` hardcode (:784) -> i18n key added to ALL 18 locale dirs.
- `manualSubExpanded`/`setManualSubCollapsed` name-value mismatch (:120) — rename only.
- `platformSubName`/`platformGroupBySub` (:65/:68) — dedupe against the NodesView pattern.
- Audit obs 1: `run-bench.mjs:687-694` assertion area — pin `streak>=3` as named constant + comment. Zero file overlap with R12-D3; must NOT touch `:75` PHASES registry.
- Audit obs 5: `mock.mjs:15-16` stale "only loopback permitted" comment -> truthful comment citing the R12-C1 external-CONNECT exception.
- Audit obs 2: `waitAllClosed` dropped return (:680) -> trace only, never error/fail.
- Exemptions recorded IN the ticket file (prevents critique re-surfacing): obs 3 (Toxiproxy comment absence), obs 4 (BENCH_FWD_* env dup), obs 6 (third bare unwrap) — tolerated per audit recommendation.

### 4b faultinject monthly schedule (legislation)
- bench.yml gains second cron `0 3 1 * *`; schedules cannot pass inputs, so:
  - `PHASES_ARG: ${{ github.event_name == 'schedule' && '--phases <explicit full default set>,faultinject' || (inputs.phases != '' && format('--phases {0}', inputs.phases) || '') }}` — verbatim into the ticket; phase list resolved from `run-bench.mjs:75` PHASES default at implementation. EXACT semantics: explicit full list + faultinject (harness additive semantics REJECTED — would violate the phase-registry freeze).
  - Build-step `if:` at :99/:107 gain `|| github.event_name == 'schedule'`; windows leg `with_app` is empty under schedule — same treatment.
  - Ticket carries a dry-run verification step for schedule-path behavior.
- Self-referential register line (applied §5): monthly run absent >=2 consecutive occurrences -> review (GitHub 60-day auto-disable).
- bench-selfhosted.yml stays dispatch-only forever — SECURITY RAIL reaffirmation, not new law.
- faultinject numbers NEVER flow into acceptance.json pin columns (suppressor survival evidence, not perf baseline).

### 4c converge measure-first counter
- Permanent dev-only counter: monotonic-timestamp marks at both `ipcAuthoritativeSnapshot` trigger-path entries; duplicate criterion = in-flight overlap ONLY (second mark lands while first invoke unresolved; sequential triggers explicitly not counted — ADR-0069 D3 drift-visibility design). `import.meta.env.DEV` compile-time elimination, ~10 lines.
- Positive: >=1 measured in-flight overlap -> React Query arm (a) fires naturally. Negative closure: two observation cycles (two release cycles) at zero -> register records "measurement complete, negative, stays SUSPENDED".

Negatives: pure hygiene — no behavior contract changes; schedule branch never enters default phase set or verify gate; the three file-touch surfaces must not trip register touch-rules; do not touch `run-bench.mjs` phase registry (R12-C1 acceptance context).

## §5 Register write-backs (APPLIED at consolidation commit) [D-005]

New armed rows: port-scan (measured suggestion-latency or >=1 allocation-conflict -> ticket); DbPool lock-wait (tied to bench-selfhosted measurement; CANNOT FIRE while runner unprovisioned, fallback 2026-10-20); backup-envelope (any envelope change need -> open); ubuntu s1 waitForTitle (baseline 35755345313 both legs green; recurrence = same s1 step fails AFTER merged-state green); faultinject-60day heartbeat.

Annotations: strategy_service row notes critique as independent external evidence (headroom refreshed 3636/4000=364, thresholds unchanged); i18n line 2 dead-arm note (no telemetry -> permanently unevaluable); React Query SUSPENDED -> measuring (armed by 4c counter).

Closed-observation verdicts: resolve_id double-RTT (ADR-0071 D-002-2 Accepted); dead-semantics manifest commands account_add/account_bind_ip (AGENTS §7.6 + CONTEXT `Echo Command`); i18n-check Rust-side blind spot (observation, no line).

## §6 Ordering & ops

- D1/D2/D3/D4 file sets are mutually disjoint (headless_main / encoding+ip_reputation / audit.rs / frontend+bench) — parallel-safe lanes; one commit each (D2 = two commits by spec).
- Ops: first monthly faultinject scheduled run needs a watched first execution (dry-run step in ticket). bench-selfhosted runner registration remains a user-side action (fallback 2026-10-20).

## Suggested skills

- `grill-with-docs` — next adjudication round.
- `atomcode-research` — any research request (serial, one run in flight).
- `gitbutler` (`but`) — all commits/pushes/landing.
- `domain-modeling` — only if a genuinely new domain term appears during implementation (none legislated this round).
- `neat-freak` — post-landing closeout.
- `sqlite-utils` — if touching `egressapikey.db` fixture rows in tests.