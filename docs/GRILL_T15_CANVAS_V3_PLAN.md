# GRILL T15 Canvas v3 Fix Plan

> Source: grill session 2026-08-16 (Q1-Q4 user decisions all A/A1).
> Authoritative refs: ADR-0039 (canvas v3 fold/center/sync/two-file boundary), ADR-0036 (strategy write pipeline, extended here on the read side), docs/CONTEXT.md (domain glossary).
> Industry research: pplx kimi_k26 on (a) proxy/topology dual-fold toggle best practice, (b) dagre center-to-top-left canonical formula. Provenance archived in ADR-0039.

## Scope (this plan only)

Four user-visible issues, each with a root cause found in code (not speculation):

1. **Region mode floods the canvas.** Toggle is useful, but RegionGroupNode is auto-expanding every per-node row. Fix: fold contract.
2. **Home button centers wrong.** `layoutNodesViaDagre` subtracts `graphLabel.width/2` on top of the correct `nodeWidth/2`; the offset grows with node count. Fix: drop the extra offset.
3. **Strategy badges mismatch the JSON.** `aClassLabel` is computed from `p.region_filters?.length` alone; only the `Region` variant renders correctly. `bClassLabel` is read from Resin `allocation_policy`, not from strategyConfig `b_class`. Fix: extend sync merge.
4. **Whitebox edits not visible from inside the GUI.** The strategy JSON path + reload button (mirroring the network-whitebox card) is missing. Fix: add it; keep two-file boundary.

Out of scope: the legacy `patchAndSyncOnce` dead code (tracked in AGENTS SS56 ponytail-debt, not this plan); a JSON text editor inside the app; merging `config_export` with `strategy_config` into one file.

## Phases

### Phase T15-v3-1: RegionGroupNode fold contract

- Mirror `SubscriptionGroupNode`: collapsed summary (region label uppercase + healthy/total count badge + region flag chip), no per-node rows by default.
- Expandable on click: same disclosure pattern as `SubscriptionGroupNode` (collapsible section with max 10 rows visible + "+N more").
- **No auto-expand** when only one region exists; fold contract is stable regardless of count.
- i18n keys (add to all 18 locales): `topology.regionGroup` (label), `topology.expandNodes` (expand affordance), `topology.collapseNodes` (collapse affordance). Reuse `topology.healthy` / `topology.unhealthy` already present from T15-2.
- Test: vitest rendering a 50-node region group asserts exactly one summary card renders; expanding via the disclosure button adds at most 10 node rows.
- ~15 lines (component body) + 3 new keys x 18 locales.

### Phase T15-v3-2: dagre center correction

- In `layoutNodesViaDagre` (TopologyView.tsx:170-176), delete the two lines that compute `offsetX` / `offsetY` from `graphLabel.width` / `graphLabel.height` and the `... - offsetX` / `... - offsetY` terms in the node-position math.
- Result position: `{ x: pos.x - nodeWidth / 2, y: pos.y - nodeHeight / 2 }` only. Verified per ADR-0039 SS4 against the kimi_k26 research and the official ReactFlow + dagre example.
- Keep the existing `marginx: 20 / marginy: 20` graph config; dagre's internal margin still provides breathing room at the origin.
- Test: vitest with 1, 3, and 20 nodes each layout; assert the average node-position delta vs 1-node baseline is bounded (< graphLabel.width drift). Confirm Home (`setViewport({x:0,y:0,zoom:1})`) shows the graph origin corner consistently.
- ~2 lines deleted.

### Phase T15-v3-3: strategyConfig read-side merge

- Extend `PlatformFull` (TopologyView.tsx:34-42) with optional fields: `aClass?`, `bClass?`, `manualNodes?`, `subscriptionNames?`, `topN?`.
- In `sync()` (already merges `regions`), also merge these from `cfgRaw.platforms`:
  - `aClass` <- `ps.a_class` (string; one of `manual` / `region` / `quality` / `subscription`)
  - `bClass` <- `ps.b_class` (shell StrategyId string)
  - `manualNodes` <- `ps.manual_nodes`
  - `subscriptionNames` <- `ps.subscriptions`
  - `topN` <- `ps.top_n`
- Replace the aClassLabel computation (L613) with a switch on `p.aClass ?? "manual"`:
  - `manual` -> `t("topology.aClassManual")`
  - `region` -> `t("topology.aClassRegion", { regions: p.region_filters!.join(",").toUpperCase() })` (unchanged)
  - `quality` -> `t("topology.aClassQuality", { n: p.topN ?? 10 })`
  - `subscription` -> `t("topology.aClassSubscription", { subs: (p.subscriptionNames ?? []).join(",") })`
- Replace bClassLabel computation (L612): use `p.bClass` (strategyConfig) if present; fall back to `mapResinToShell(p.allocation_policy)` only when `bClass` is absent (debug path).
- i18n keys (add to all 18 locales): `topology.aClassManual`, `topology.aClassQuality`, `topology.aClassSubscription`. Reuse `strategy.*` keys already present.
- Tests:
  - Extend T15-5 "whitebox override" test: strategyConfig `a_class: "subscription"` + `subscriptions: ["sub1","sub2"]` and `a_class: "quality"` + `top_n: 5` each render the correct badge text (not "Manual").
  - New test: assert Home centers the graph by calling `layoutNodesViaDagre` with 1 vs 20 nodes; assert `setViewport({x:0,y:0,zoom:1})` arguments match (this is covered by T15-v3-2 actually; here only assert the pre-Home layout positions are stable).
- ~35 lines (sync merge expansion + label switch) + 3 new keys x 18 locales.

### Phase T15-v3-4: Settings strategy-JSON reload surface

- Add a "Strategy config" card in `SettingsView.tsx`, mirroring the existing "Network whitebox" card pattern:
  - read-only path label: `egressapikey-strategy.json` absolute path (need a new IPC `strategy_config_path` or compute from `get_config_dir` + filename; simplest is to reuse `get_config_dir` and append the filename in TS).
  - "Reload from disk" button: calls `ipcStrategyConfigGet` -> `ipcStrategyApply` (forces re-read + re-PROTECT). Reuses existing IPC, no new Rust code.
  - Status line after reload: applied N platforms, errors M (parsed from `strategy_apply` result).
- No text editor widget, no Save button on this card. External-edits then reload is the whitebox loop (ADR-0039 SS5 / SS1).
- i18n keys (add to all 18 locales): `settings.strategyConfigPath`, `settings.strategyConfigReload`, `settings.strategyReloaded`.
- Test: vitest rendering SettingsView with mocked `strategy_config_get` returning a 2-platform config; click reload -> assert `strategy_apply` is invoked exactly once. Reuses the existing SettingsView test file.
- ~12 lines + 3 new keys x 18 locales.

### Phase T15-v3-5: Full gate + exe hard close-loop + push

- `pnpm test --run` (verify 225 baseline + new tests still green)
- `pnpm exec tsc --noEmit`
- `pnpm i18n:check` (verify all 18 locales match; new keys added symmetrically)
- `pnpm build` (Vite writes a new chunk hash to `dist/assets`)
- `cargo build --release -p egressapikey-app --features custom-protocol`
- Stage `release/windows-gui/EgressAPIKEY.exe` + `resin.exe` sidecar
- **Chunk-hash hard close-loop** (AGENTS SS5): grep the new Vite chunk hash inside the exe bytes via `Buffer.indexOf`; FAIL if not found.
- Smoke: launch staged exe once, assert `MainWindowTitle == "EgressAPIKEY"`, WS ~30-40MB, resin.exe child alive.
- `codegraph sync .`
- `git add` changed source + locale + docs + AGENTS.md update; `git push origin codex/rust-port`.

### Phase T15-v3-6: AGENTS.md SS57 + ADR-0036 read-side addendum

- Append AGENTS.md SS57 documenting: fold contract, dagre center correction, strategy badge sync merge (ADR-0036 read-side), two-file boundary, reload surface, new i18n keys, test totals.
- Append a short addendum to ADR-0036 noting that the *read* side of the pipeline is now specified by ADR-0039 SS2; no reversal of the write-side decision.

## Acceptance Criteria

1. Region viewMode renders folded summary cards only; 50-node region group renders 1 card + 0 node rows by default - vitest.
2. `setViewport({x:0,y:0,zoom:1})` frames the same origin corner with 1 node and with 20 nodes - vitest asserting layout positions do not drift by more than the natural dagre rank growth.
3. A `subscription` A-class strategy renders `A: Subscription: sub1,sub2` (i18n key), not `A: Manual` - vitest.
4. A `quality` A-class strategy renders `A: Quality Top-10` (i18n key), not `A: Manual` - vitest.
5. Settings shows `egressapikey-strategy.json` path + a Reload button that invokes `strategy_apply` once - vitest.
6. 225+ vitest + tsc + i18n:check green; new keys x 18 locales; cargo untouched; release exe staged with chunk-hash verified; codegraph synced; pushed.

## Dependencies

- No new npm or cargo deps. All edits use existing ReactFlow/dagre/zustand/i18next plumbing.
- ctx_* file-edit pattern continues (Node `.cjs` scripts under `.codex-tmp/`); AGENTS SS7 windows-file-integrity observed (LF, no BOM, no inline `$` PS).

## Risk and rollback

- ADR-0039 SS4 (dagre offset deletion) is the only change that touches layout math; visual regression risk is minimal (Home now reliably shows the origin corner instead of a node-count-dependent point). Roll-back: re-add the deleted two lines.
- ADR-0039 SS3 (fold) is additive (node rows are now hidden by default; expanding still works). Roll-back: change the default expanded state on RegionGroupNode.
- ADR-0039 SS2 (badge sync) is additive; fallback to Resin fields when strategyConfig has no entry for the platform. Roll-back: revert the sync merge lines.

## Industry references

- pplx kimi_k26 (2026-08-16): "proxy/topology dual-fold toggle best practice" - conclusion: mature tools keep the toggle but default to a fold-first mode; region mode should never auto-expand child rows.
- pplx kimi_k26 (2026-08-16): "dagre center to ReactFlow top-left authoritative formula" - conclusion: `pos.x - nodeWidth/2` is the canonical conversion; graph.width/2 subtraction is a known mistake.
- ADR-0039 captures both citations inline.
