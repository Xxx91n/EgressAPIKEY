# Changelog

All notable changes to EgressAPIKEY are documented in this file.

The format is based on [Keep a Changelog 2.0.0](https://keepachangelog.com/en/2.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- docs/ governance hierarchy: `docs/history/` (phases, handoffs, prompts), `docs/architecture/`, `docs/how-to/`, `docs/reference/`, `docs/research/`
- `docs/README.md` directory map, `docs/history/README.md` archive marker
- `docs/DOCUMENT_GOVERNANCE_PLAN.md` governance plan (32-source industry research: Diataxis + ADR + AGENTS.md + docs-as-code)

### Changed
- README.md / README_CN.md doc links updated to new `docs/` subdirectory paths
- AGENTS.md L47 Docs pointer line updated to new paths

### Removed
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
