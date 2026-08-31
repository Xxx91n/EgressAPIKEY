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

### Changed
- README.md / README_CN.md doc links updated to new `docs/` subdirectory paths
- AGENTS.md L47 Docs pointer line updated to new paths
- (architecture-recovery 02) settings.json dead keys gatewayBind/mihomoApi: load/save helpers removed (comment lie fixed), one-time startup purge migration
- (architecture-recovery 05) config authority legislated as L1/L2/L3 (ARCHITECTURE.md §Config Authority; CONTEXT.md terms White-box Config / Authoritative Write Entry / Authoritative Snapshot; AGENTS.md storage section reorganized)
- (architecture-recovery 06) dual `map_resin_error` merged into resin-core single implementation (typed i18n_key, BindConflict port extraction; ADR-0045 semantics preserved)
- (architecture-recovery 08) src-tauri/src/commands/mod.rs (3.4k lines) split into platform/strategy/ports/backup/settings/diagnostics + common + tests modules; generate_handler registry and IPC surface unchanged
- (architecture-recovery 11) request_log_tail retargeted to Resin GET /api/v1/request-logs; direct request_logs*.db read deleted; RESIN_UPSTREAM_MANIFEST compat note added

### Removed
- resin-core dead kernel face (ADR-0050): mihomo/gateway/lane/lease/tdewma modules, CoreConfig/sanitize_lanes/lane constants, and the resin-core stub bin; headless product surface remains the egressapikey-headless bin (ADR-0043)
- Unused resin-core dependencies: axum, hyper, bytes, http, thiserror, clap, tracing-subscriber, fxhash, tower (dev), http-body-util (dev)
- `docs/CONTEXT.md` (root `CONTEXT.md` is the canonical copy)

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
