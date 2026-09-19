# Capability Matrix — Current Verified Surface

> **What this page is.** The repo-level answer to "how mature is this
> project, really" — a list of capabilities where every entry is anchored to
> evidence that CI executes, plus an explicit list of what no CI gate proves
> (round-8 A-021). It replaces the "70 ADRs" maturity narrative: an ADR count
> says how many decisions were recorded, not which behaviors are verified.
>
> **Reading rule.** A capability appears as *verified* only when the cited
> evidence runs in an automated gate. "Code exists" or "docs say so" is not
> evidence. Rows marked **not CI-verified** are still true statements about
> the code — they just carry no automated regression proof.
>
> **Snapshot.** Tree state at round-8 execution: applied ticket branches
> 01/02/03/05/06/07/08/09/10 + grill-docs over base `911e35e1`. Uncommitted
> in-flight work (tickets 04/12/13/17/18) is excluded and listed in §5.

## 1. Evidence tiers

| Tier | Gate | Trigger | Executes |
| --- | --- | --- | --- |
| T1 | `ci.yml` job `verify` | every push / pull_request | `cargo build` + `cargo test -p resin-core` (~308 tests), `tsc -b`, `vite build`, `vitest run` (24 spec files), six guard scripts, `git diff --check` |
| T2 | `docs-lint.yml` (Docs Governance) | PR touching docs/governance files; push to main/master | markdownlint-cli2, dead-link check, layer-separation guard, AGENTS.md size warning |
| T3 | `ci.yml` release matrix | workflow_dispatch only | playwright e2e (5 specs), headless binary ×4 targets, GUI installer ×4, portable GUI ×3 — **builds only, no test run** |
| T0 | — | never in CI | `src-tauri` Rust unit tests (~156: sidecar, tray, headless_*, commands/tests — app crate is a Windows-only compile surface skipped by T1 on Linux), `#[ignore]`d live-sidecar probes, all performance budgets |

The six T1 guard scripts (all wired through `scripts/verify-build.sh`):
`i18n-check` (18-locale full-key parity), `ipc-manifest-check` (78 commands:
defined = registered = AGENTS.md manifest), `vitest-isolation-guard`,
`license-field-check` (package.json / Cargo workspace / README / LICENSE),
`readme-lang-check` (EN↔CN structural sync), `upstream-router-check`
(UPSTREAM.md links + RELEASE_NOTES↔CHANGELOG version alignment).

## 2. Capability matrix (anchor table)

### Data plane

| Capability | What CI proves | Anchor (tier) | Residual gap |
| --- | --- | --- | --- |
| Entry-port lifecycle (upsert/remove/toggle/bind, 1024–65535, ≤256 ports) | Validation, persistence, Resin endpoint CRUD order | whitebox_config + db + ports IPC vitest wrappers (T1) | Listener lifecycle is Resin-owned by design — no shell TcpListener (ADR-0012) |
| Port = (platform, account) identity record | `resin_identity()` string + PortMapping persistence | port_forwarder tests: `identity_defaults_account_to_port`, `distinct_ports_distinct_identities` (T1) | Binding is config-plane only; live exit selection still keys on the client-supplied credential (Mode B) until Mode A lands (§5) |
| Entry protocols {http, socks5} | Closed-set validation at whitebox + IPC boundaries | validator tests + `ipcPortUpsert` rejection tests (T1) | `mixed` default + socks5-tightening in flight (ticket 13, §5) |
| Exit-IP probe parsing | Cloudflare-trace body → IP parse | proxy_e2e mockito tests ×3 + port_forwarder parse tests (T1) | Real-network probe `test_probe_real_1_1_1_1` is `#[ignore]`d (T0) |
| Port health classification | alive/degraded/restarting/dead + adaptive interval + backoff cap | port_health 12 tests (T1) | Watch-stream delivery to UI tested at component level only |
| Stream classification (SSE / WebSocket / unary) | First-bytes classifier | stream_sensor 4 tests (T1) | Classification ≠ stickiness; SSE lock-to-node is Resin lease behavior (§4) |

### Control plane

| Capability | What CI proves | Anchor (tier) | Residual gap |
| --- | --- | --- | --- |
| Resin REST client surface | platform/subscription/endpoint/node/lease/header-rule/metrics/request-log CRUD + probes; loopback-only enforcement; read-retry on 503, write-retry ≤3, 4xx never retried | resin_client 53 mockito tests incl. `new_rejects_non_loopback` (T1) | Covers wire contract, not a live Resin |
| Name→UUID resolution | deterministic resolve + input rejection pre-HTTP | resolve_* tests (T1) + headless `id_for_name` tests (T0) | — |
| Strategy truth set {BALANCED, PREFER_LOW_LATENCY, PREFER_IDLE_IP} | unknown token rejected, six legacy tokens migrated once, canonical wire spelling | strategy 5 + strategy_engine migration tests (T1) | — |
| Strategy apply: diff-then-skip + allocation_policy drift | only drifting axes PATCHed; policy-only drift reports Divergent; second apply = zero writes | strategy_service 47 incl. `apply_twice_second_pass_zero_patches`, `apply_allocation_policy_drift_patches_policy`; snapshot `allocation_policy_drift_is_divergent...` (T1) | — |
| Snapshot three-state merge | Consistent/Divergent/Missing + acknowledged + divergent_since + converge phase table | snapshot 32 tests (T1) | — |
| Reconcile idempotency | second pass = zero changes; strategy error fails fast | tests/reconcile.rs 2 + subscription_pipeline_e2e 2 (T1) | TTL/stamp nuance covered by unit tests; live-restore timing not CI-tested |
| L2-first write order (port_*) | whitebox intent persists before Resin mutation; L3 rejection surfaces typed error | whitebox apply/rollback tests + ports.rs write-order contract (T1) | Ordering is a code contract; no dedicated fault-injection test |
| Subscription pipeline | 5-step cascade, compensation never deletes whitebox-declared rows, second-pass zero writes, default port stays ≥1024 | subscription_pipeline 21 + e2e ×3 (T1) | — |
| Process route honesty | rules are metadata + watchdog; UI states no automatic process takeover | ProcessRouteView vitest + ipc validation tests (T1) | Copy verified; "no takeover" is architectural, not a testable assertion |

### State integrity

| Capability | What CI proves | Anchor (tier) | Residual gap |
| --- | --- | --- | --- |
| Whitebox atomic write + backup/rollback | no partial file; backup-before-write; tampered backup rejected | whitebox_config 20 + whitebox_backup 10 (T1) | — |
| Config export/import | export embeds both whitebox docs verbatim; invalid schema/version rejected with zero writes | config_transfer 8 tests (T1) | Both docs validated up front, but sequential writes can still half-land on mid-sequence failure — two-phase atomic swap in flight (ticket 12, §5) |
| Audit log | sha256-chained JSONL, restart relinks tail, rotation keeps chain | audit 8 tests (T1) | — |
| Port map store (egressapikey.db) | idempotent migration, upsert/delete/replace | db 5 tests (T1) | — |
| IPC error contract | serde round-trips + Resin error → typed IpcError mapping (incl. bind-conflict precedence) | ipc_error 27 + ipc_error.test.ts (T1) | — |

### Frontend

| Capability | What CI proves | Anchor (tier) | Residual gap |
| --- | --- | --- | --- |
| SPA builds and typechecks | tsc -b + vite build every push | verify job (T1) | — |
| Views/store/hooks behavior | converge state machine, IPC sanitizers, settings, process-route copy | vitest 24 spec files (T1) | Playwright e2e (5 specs: mount, tab switch, platform round-trip, safety-net contract, sidecar boot) runs release-track only (T3) |
| i18n ×18 locales | full-key parity across all 18 locales | i18n-check guard (T1) | Guard checks key coverage, not translation quality; app.title rename value checked by readme-lang + e2e (T3) |

### Headless

| Capability | What CI proves | Anchor (tier) | Residual gap |
| --- | --- | --- | --- |
| Browser control surface | 35/79 IPC commands have real HTTP semantics (26 Resin-proxy translations + 9 native port routes); 44 carry typed disabled reasons — no runtime Tauri-only throws | CMD_TO_HTTP/DISABLED_COMMANDS tables + T17 dual-mode vitest cases (T1); headless_main.rs 17 tests (T0) | 44 shell-local commands intentionally absent (L1 prefs, L2 whitebox raw files, snapshot/reconcile, process routes, config transfer, backup, sidecar lifecycle, tray/OS) — "full parity" is not the design |
| Server-side token injection | browser never holds Resin admin token; BFF strips inbound Authorization, injects server-side | headless BFF code + headless_security tests (T0 — not CI-run) | Structurally enforced in code; tests run only on Windows-capable hosts |
| Non-loopback security gate | refuses to bind beyond loopback without explicit --auth-token; Host/Origin allowlist; CSPRNG token | headless_security.rs 14 tests (T0) | Same T0 caveat |
| Headless binary ships | egressapikey-headless compiles for linux/win/macos×2 | backend matrix (T3) | Release-track only; no push-gate compile of the app crate |

### Delivery & governance

| Capability | What CI proves | Anchor (tier) | Residual gap |
| --- | --- | --- | --- |
| Push→verify gate | every push/PR runs build+tests+guards | ci.yml `verify` (self-evidencing, T1) | Branch-protection "required check" is a repo setting, outside CI files |
| Release artifacts | installers (msi/nsis/deb/AppImage/dmg), portable ×3, headless ×4 | gui/gui-portable/backend matrix (T3) | Manual dispatch by design (ADR-0010 revision) |
| Docs governance | markdownlint, dead links, layer separation | docs-lint.yml (T2) | AGENTS.md size is warn-only (currently ~46 KB > 32 KiB Codex default) |
| License layering declared | package.json = Cargo workspace = README = GPL-3.0-or-later; LICENSE is verbatim GPL text | license-field-check (T1) | Legal posture documented in THIRD_PARTY.md/ADR-0067 — declared, not audited |
| Bilingual README | EN/CN heading map 1:1 + license body + drift-token absence | readme-lang-check (T1) | — |
| Upstream docs integrity | UPSTREAM.md links resolve; RELEASE_NOTES↔CHANGELOG versions aligned | upstream-router-check (T1) | — |
| IPC surface locked | 79 commands defined = registered = manifest | ipc-manifest-check (T1) | — |

## 3. Cross-check: README / docs promises vs verified state

| # | Claim (source) | Verdict | Evidence / note |
| --- | --- | --- | --- |
| 1 | "Multi-port socks5/http forwarder" (README headline) | **anchored** | {http,socks5} closed set enforced; Resin owns listeners (§2 Data plane) |
| 2 | "each port is the identity of one (platform, account) pair" (README) | **partially** | Identity record is stored/validated (T1); live exit binding requires client credential today — Mode A forwarder in flight |
| 3 | "guarantees a distinct sticky exit IP per pair" (README) | **not CI-verified** | Guarantee is Resin-side; shell anchors identity plumbing only; live probe is #[ignore]d (T0) |
| 4 | "SSE session stickiness — locks its node until completion, auto-switching on failure" (README) | **not CI-verified** | Classification anchored (T1); stickiness = Resin lease behavior, no e2e in any gate |
| 5 | "Per-request TCP freshness — pool_max_idle_per_host(0)" (README + README_CN) | **stale mechanism name** | Symbol absent from codebase; actual knob is whitebox `network.max_idle_conns*` → sidecar transport env (ADR-0028). Claim intent survives; named mechanism is wrong |
| 6 | "Strategy engine — B-class decides how a port picks its exit (random / round-robin / low-latency)" (README) | **stale vocabulary** | Post ticket-01 truth set is the three Resin allocation policies; the six-way per-port picker narrative is withdrawn (CONTEXT.md B-Class Strategy) |
| 7 | "Topology canvas — drag-to-connect hot-patches region filters on the live sidecar" (README) | **anchored (partial)** | `strategy_platform_regions_set` → apply path tested (T1) + TopologyView vitest; live-sidecar effect covered by contract, not e2e |
| 8 | "Zero adaptation — point your gateway at an entry port; client code stays unchanged" (README) | **partially true today** | No-auth entry ports accept credential-free clients (ADR-0027); per-port (platform,account) binding still needs client-supplied credential until Mode A lands (tickets 13/17/18) |
| 9 | "Headless twin — the full GUI control surface over HTTP" (README) | **overstated** | 35/79 mapped + 44 typed-disabled by design (§2 Headless); "same control surface" holds only for the Resin-backed subset |
| 10 | "admin bearer token injected server-side; browser never sees it" (README) | **anchored (code) + T0 tests** | BFF strips/injects (headless_main.rs) + security gate tests exist but are not CI-executed |
| 11 | "68 ADRs" (docs/README.md) | **stale count** | 70 files on disk (69 numbered + 0050-bis) — the narrative this page replaces was not even numerically current |
| 12 | "socks5/http/https listening port" (CONTEXT.md Entry Port) | **wording drift** | Protocol set is {http,socks5}→{mixed,http,socks5}; "https" describes CONNECT targets, not an entry protocol — harmless but loose |

## 4. What no CI gate proves (explicit)

- **Real exit-IP correctness**: no gate opens a live upstream connection; the
  only real-network probe is `#[ignore]`d and needs a running sidecar.
- **End-to-end SSE behavior**: per-event flush, disconnect cascade-cancel,
  bounded backpressure, in-band error signaling (D-004 hard assertions) have
  no automated evidence yet.
- **Performance budgets**: all D-004 targets (memory/CPU/latency/RPS) are
  measure-then-calibrate placeholders; the bench harness (`scripts/bench/`)
  is in flight, no locked values exist.
- **App-crate Rust tests**: ~156 src-tauri unit tests (sidecar lifecycle,
  tray, headless security/adapter, command layer) never execute in CI —
  Windows-only compile surface; release jobs build but do not test.
- **GUI installer correctness**: artifacts are built (T3) but never launched
  under test.
- **Windows/macOS push-gate coverage**: T1 runs on ubuntu-latest only.

## 5. In-flight work (uncommitted / parallel round-8 tickets)

Excluded from "verified" above until they land with green CI:

- Ticket 13/17/18 — `mixed` entry protocol, Mode-A shell forwarder
  (credential injection so the client needs none), perf/SSE acceptance.
- Ticket 12 — config_import two-phase atomic swap; reconcile stamp semantics.
- Ticket 04/17 — bench drivers + acceptance table (scripts/bench/*).
- Ticket 11 — ADR Status-line batch (shares A-021 with this ticket).
- Ticket 14 — backup/restore unification.
- Ticket 15 — production-code process-trace comment strip (A-019).

## 6. Maintenance rule

Every new capability claim in README/docs must name its evidence anchor
(job, test, or guard) — same closed-loop discipline as ADR-0020. When a gate
gains coverage, move the row; when a claim loses its anchor, the claim goes
to §4 or is deleted. This file is linted by docs-lint (T2) like every
docs/** page.
