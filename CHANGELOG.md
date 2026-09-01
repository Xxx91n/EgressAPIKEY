# Changelog

All notable changes to EgressAPIKEY are documented in this file.

The format is based on [Keep a Changelog 2.0.0](https://keepachangelog.com/en/2.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- docs/ governance hierarchy: `docs/history/` (phases, handoffs, prompts), `docs/architecture/`, `docs/how-to/`, `docs/reference/`, `docs/research/`
- `docs/README.md` directory map, `docs/history/README.md` archive marker
- `docs/DOCUMENT_GOVERNANCE_PLAN.md` governance plan (32-source industry research: Diataxis + ADR + AGENTS.md + docs-as-code)
- (architecture-recovery 01) docs/agents skill-library config: issue-tracker / domain / triage-labels declarations + AGENTS.md `## Agent skills` block
- (architecture-recovery 03) `scripts/ipc-manifest-check.cjs`: three-source IPC manifest guard (#[tauri::command] vs generate_handler! vs AGENTS.md §7.6), wired into package.json (`pnpm ipc:check`) and verify-build.sh
- (architecture-recovery 07, ADR-0051) `authoritative_snapshot` deep IPC: one pre-merged three-state read-back (strategy+ports whitebox vs Resin runtime) in resin-core/snapshot.rs; TopologyView view-layer merge deleted; Settings read-only effective-config card (i18n x18)
- (architecture-recovery 09, ADR-0053 PROPOSED) tauri-specta type-layer pilot on 3 commands: generated src/bindings.ts (IpcError/LaneSnapshot/PortMapping/LogLevel), comctl32 v6 SxS manifest fix for MSVC test targets
- (architecture-recovery 10, ADR-0052) StrategyService deep module: strategyConfig get/validate/store/apply + stale-platform auto-clean converge in resin-core/strategy_service.rs; new `strategy_platform_regions_set` deep IPC; TopologyView drops whitebox JSON assembly
- (architecture-recovery 12, ADR-0054 §C/§D) snapshot metadata + acknowledged exemptions: top-level `lastCheckedAt`, per-entry `divergentSince` via process-local DriftMemory (restart clears, re-drift re-times), optional `acknowledged` arrays in both whitebox files (never enter the three-state merge)
- (architecture-recovery 13, ADR-0054 §C.4) one-level Effective Config view (nav key `effectiveConfig`): desired|live comparison, three-state badges, grey known degradation, divergentSince, lastCheckedAt, manual re-check; Settings effective card demoted to jump entry; zero write paths
- (architecture-recovery 14, ADR-0054 §A) one-way reconcile + preview: `reconcile_now` IPC (strategy apply -> ports restore, fail-fast, whitebox always wins), preview dialog derived from the in-memory snapshot, 24h ReconcileMemory idempotency window; no accept-current-state path
- (architecture-recovery 15, ADR-0054 §B) whitebox versioning + rollback: write-before-backup to sibling `backup/` (10 kept, same-second `-N` suffix), `strategy_backup_list`/`strategy_rollback` + `whitebox_backup_list`/`whitebox_rollback` IPC, rollback re-enters the same validate->apply chain (never a bypass); Effective Config history section with per-entry rollback
- (architecture-recovery 16, ADR-0054 §E) one-shot drift tray notification: fires once per process on first unacknowledged drift, re-arms after a zero-drift snapshot, acknowledged entries exempt, sidecar-down silent; 18-locale copy (`tray.driftTitle`/`tray.driftBody`); tauri-plugin-notification (best-effort); docs closeout: ADR-0054 ACCEPTED, CONTEXT.md Reconcile term, ARCHITECTURE.md reconciliation-closure section, `docs/how-to/WHY-NOT-EFFECTIVE.md` troubleshooting table
- (architecture-recovery 17, ADR-0055) processRoutes migrated L1 -> L2 whitebox: `process_routes` + `route_acknowledged` fields in `egressapikey-ports.json` reusing the versioned backup/rollback chain; `process_route_add/remove/list` are the single write entry (webview `saveProcessRoutes`/`loadProcessRoutes` pair deleted); one-time idempotent boot migration purges the legacy L1 key; `authoritative_snapshot` gains `routes` three-state (a route is as live as its target port), route drift joins the one-shot tray notify with acknowledged exemptions; one-way reconcile converges routes via the existing ports-restore half; i18n 435 keys x 18; tests: resin-core 188 (+10), shell 95, vitest 338 (+3)
- (architecture-recovery 19) reconcile preview coverage completion: the preview dialog's ports-domain will-change rows are locked beside the strategy domain by new vitest coverage (snapshot-derived, zero extra requests), and the missing-on-resin platform wording is now explicit — "exists only in the desired state — will be established" x18 locales (still 435 keys); preview interaction semantics unchanged (confirm reconciles, cancel zero-IPC, in-flight re-entry guard, auto re-verify)

### Changed
- (architecture-recovery 22, ADR-0056) reconcile platform semantics aligned: `strategy_apply` now fulfills the preview's "will be established on Resin" promise by creating missing-on-resin platforms through the existing `ResinClient::create_platform_from_name` seam (name-only POST, Resin v1.2.0) and then PATCHing their computed region_filters; the former apply-path auto-clean that DELETED the same whitebox entries (the last active say/do contradiction) is gone — apply never deletes whitebox desired state, a failed create is reported per-platform and retried on the next apply; `clean_stale` stays exported for the dead-code sweep tickets but apply no longer calls it; preview/view/locale copy unchanged (the promise became true, 435×18 keys untouched); tests: resin-core 198 (+3 apply behavior locks: create+PATCH+whitebox-survives, failed-create reports-and-keeps, live-platform no-create), shell 97, vitest 340
- README.md / README_CN.md doc links updated to new `docs/` subdirectory paths
- AGENTS.md L47 Docs pointer line updated to new paths
- (architecture-recovery 02) settings.json dead keys gatewayBind/mihomoApi: load/save helpers removed (comment lie fixed), one-time startup purge migration
- (architecture-recovery 05) config authority legislated as L1/L2/L3 (ARCHITECTURE.md §Config Authority; CONTEXT.md terms White-box Config / Authoritative Write Entry / Authoritative Snapshot; AGENTS.md storage section reorganized)
- (architecture-recovery 06) dual `map_resin_error` merged into resin-core single implementation (typed i18n_key, BindConflict port extraction; ADR-0045 semantics preserved)
- (architecture-recovery 08) src-tauri/src/commands/mod.rs (3.4k lines) split into platform/strategy/ports/backup/settings/diagnostics + common + tests modules; generate_handler registry and IPC surface unchanged
- (architecture-recovery 11) request_log_tail retargeted to Resin GET /api/v1/request-logs; direct request_logs*.db read deleted; RESIN_UPSTREAM_MANIFEST compat note added
- (architecture-recovery 20) single-owner throttle model: poll rhythm (adaptive interval + exponential backoff) and lightweight close-delay arithmetic consolidated into `crates/resin-core/src/throttle.rs` (`ThrottleParams`); parameter values unchanged at both call sites so the external rhythm is identical (legacy-equivalence tests pin every formula); `spawn_watcher_with` cadence seam + tokio virtual-time tests for multi-client coexistence and the shared pause flag


### Fixed
- (architecture-recovery 18) NodesView vitest flake eliminated by test isolation only (zero product-code changes): root cause = sub-header expand clicks landing before the default-collapse seeding effect (NodesView.tsx seeding useEffect wholesale-overwrites `collapsed` with every subscription), which inverts the toggle (expand -> collapse) under full-suite CPU contention - standalone runs always green, full-suite 2/6 rounds failed at T4-3c; all 11 sub-header clicks in NodesView.test.tsx now route through `clickSubHeaderExpand`/`clickSubHeaderCollapse` helpers that await the settled chevron state first; new scannable guard `scripts/vitest-isolation-guard.cjs` (raw sub-header clicks outside the guarded helper block fail the build) wired into verify-build.sh; build-all.sh: `TRIPLE` used-before-assignment under `set -u` fixed (backend sidecar staging aborted every run); evidence: post-fix 5/5 full-suite green 338/338 + 6/6 standalone green 26/26
### Removed
- resin-core dead kernel face (ADR-0050): mihomo/gateway/lane/lease/tdewma modules, CoreConfig/sanitize_lanes/lane constants, and the resin-core stub bin; headless product surface remains the egressapikey-headless bin (ADR-0043)
- Unused resin-core dependencies: axum, hyper, bytes, http, thiserror, clap, tracing-subscriber, fxhash, tower (dev), http-body-util (dev)
- `docs/CONTEXT.md` (root `CONTEXT.md` is the canonical copy)
- (architecture-recovery 24) strategy.rs dead catalog face: `StrategyId::parse`, `StrategyId::to_resin_allocation_policy` (Rust-side strategy↔allocation_policy mapping), `protocol_weight`, `strategy_catalog`/`StrategyInfo` deleted with their 4 self-referential tests (resin-core lib tests 199→195); `snapshot.rs` keep-import-honest placeholder line and the top-level `StrategyId` import it propped up purified; `lib.rs` re-export narrowed to `StrategyId`; the repo's only strategy↔allocation_policy mapping is now `src/lib/strategy.ts` (display/PATCH use, header comment added); `StrategyId` type + `as_str` kept (live `b_class_of` consumer)

## [0.1.0] - 2026-08-19

### Added
- Tauri 2 + React 19 desktop app: L7 proxy gateway for AI API keys
- Resin Go sidecar integration (v1.2.0): platform, subscription, node-pool, leases API
- Topology canvas: three-column key-to-egress routing (entry proxy -> platforms -> IP channels)
- 49 ADRs (MADR format) covering architecture decisions
- 18-locale i18n support (en, zh, ja, es, fr, de, ko, ru, pt, ar, hi, id, it, nl, pl, th, tr, vi)
- CI/CD: 5-artifact-group release pipeline (windows-gui, linux-gui, macos-gui, gui-portable, headless-backend)
- Ghost safety net: sidecar health poll + OS proxy clear on 3 consecutive failures
- IPC retarget to Resin sidecar (G2 phase 2): 11 commands forward to live Resin admin REST
- Ponytail debt ledger: 6 source-tagged markers tracked

### Changed
- Folder renamed from ai-api-route to EgressAPIKEY (all artifacts aligned)
- IPC commands retargeted from local SharedGateway/SharedRegistry to Resin sidecar REST

### Fixed
- Headless white-screen fix: isTauri guard + SPA fallback + ErrorBoundary + items-unwrap (T22)
- Topology connection race/resync/drag fixes (T20-T22)
- Orphan-sidecar process bug: SidecarHandle.child Mutex<Option<CommandChild>> + Exit event kill (P22)
- Subscription drag-reorder: Pointer Events replacing broken HTML5 DnD (P20-P21)
