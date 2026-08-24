# Changelog

All notable changes to EgressAPIKEY are documented in this file.

The format is based on [Keep a Changelog 2.0.0](https://keepachangelog.com/en/2.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- Canvas node-below-platform bug: subscription/region selection now renders the C-column to the RIGHT of the platform card (was stacked below-left). Three root defects identified via a diagnosing-bugs live-sidecar feedback loop against the real state.db: (A) Resin `/api/v1/nodes` tags use snake_case `subscription_name` while the frontend parser only read camelCase, (B) port-less platforms dragged the C-column into rank 1 alongside the platform, (C) region viewMode skipped platforms with non-empty `subscriptions`. Fixed by a snake-case fallback in `parseSubscriptionGroups`, three invisible dagre column anchors (`__col0/__col1/__col2`, `minlen:0` same-rank binding), and a new exported pure helper `buildRegionViewEdges` with the missing subscription+specific-subs branch. See ADR-0051; commit 255eb7b.
- Manual aClass now draws visible group-level edges (`manual:N`, `deletable: true`) to every subscription/region group containing >=1 selected node_hash; deleting such an edge removes that group's node_hashes from `strategyConfig.manual_nodes` (mirrors the existing region_filters PATCH path via `ipcStrategyConfigPut` + `ipcStrategyApply`). Revises ADR-0048 S3. See ADR-0050; commit 255eb7b.

### Added
- `docs/adr/0050-canvas-manual-visible-group-edges.md` and `docs/adr/0051-canvas-node-below-platform-three-defects.md`; ADR-0051 carries a 2026-08-24 atomcode re-verification (dagre PR #271 manual ranking shipped as `@dagrejs/dagre` 1.1.5 on 2025-06-17 — the supported upgrade path; `@xyflow/react` v12 `deleteElements -> getElementsToRemove` + `onBeforeDelete` is the official cascade pipeline; muthub-ai/aac `use-graph-store.ts` YAML projection store is the reference for derived member-hash state)
- docs/ governance hierarchy: `docs/history/` (phases, handoffs, prompts), `docs/architecture/`, `docs/how-to/`, `docs/reference/`, `docs/research/`
- `docs/README.md` directory map, `docs/history/README.md` archive marker
- `docs/DOCUMENT_GOVERNANCE_PLAN.md` governance plan (32-source industry research: Diataxis + ADR + AGENTS.md + docs-as-code)

### Changed
- CONTEXT.md Strategy-Labeled Edge glossary now lists `manual:N` alongside `region:US / quality>75 / subscription:subA`, with deletion semantics (edge delete -> remove that group's manual_nodes from the platform)
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
