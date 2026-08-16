# GRILL_T17 Canvas V4 — Node Pool + Toolbar Merge + MiniMap ariaLabel + Cross-subscription Dedup

> ADR-0041: [0041-canvas-v4-node-pool-toolbars-merge.md](./adr/0041-canvas-v4-node-pool-toolbars-merge.md)
> Fixed point: 74c90ca (T16-audit)
> Grilling: Q1=C (node unified pool), Q2=C (Kiali-style one-node-one-group), Q3=A (merge button into toolbar), Q4=B (8 tests full coverage)
> Research sources: D:/Aworker/EgressAPIKEY/.codex-tmp/ReactFlow dagre layout multi-column topo.md + D:/Aworker/EgressAPIKEY/.codex-tmp/Tauri ReactFlow Minimap Localized Toolti.md

## Root causes (from code diff)

| Bug | Root cause | File:Line |
| --- | --- | --- |
| 1. Entry-port column stray nodes in region viewMode | subGroups.forEach loop L697-731 is unconditional; in region viewMode those nodes have no edges and dagre places them in free space near entry-port column | TopologyView.tsx:697-731 |
| 2. openStrategyConfig button overlaps MiniMap | Standalone absolute button at bottom-20 left-4 sandwiched between CanvasControls (bottom-2) and MiniMap (bottom-0) | TopologyView.tsx:942-954 |
| 3. MiniMap hover tooltip not i18n-realized on hover | div[title] wrapper L955-972 has zero height because MiniMap renders into an absolutely positioned child; native title hit-test fails | TopologyView.tsx:955-972 |
| 4. Duplicate node rows across subscriptions | region viewMode loops subGroups*g.nodes without dedup by node_hash; subscription viewMode nodeRows map also lacks explicit dedup | TopologyView.tsx:743-751, 718-726 |

## Phase T17-1: viewMode guard on subGroup loop (S1)

Root cause: subGroups.forEach L697 unconditionally pushes subscriptionGroup nodes even in region viewMode; they become edgeless and dagre scatters them.

Fix: add `if (viewMode !== "subscription") return;` early-return as the first line inside the forEach callback body (before computing healthLabel / sub / filteredNodes). This forces exactly one C-column aggregation layer per viewMode.

File: src/views/TopologyView.tsx L697 (inside subGroups.forEach((g) => { ... }) body)
Lines: ~1
Test: vitest render TopologyView in region viewMode with mocked subGroups; assert node with id="subgroup-X" is NOT present in the node list, only "regiongroup-*" nodes.

## Phase T17-2: Cross-subscription node dedup by node_hash (S2)

Root cause: L743-751 `for (const g of subGroups) for (const n of g.nodes) entry.nodeRows.push(...)` copies nodes without checking `seen.has(n.node_hash)`. L718-726 subscription nodeRows map has the same structural issue inside one subscription group when Resin echoes proxies.

Fix: 
- Region viewMode: maintain `const seen = new Set<string>()` outside the subGroups loop; inside `for (const n of g.nodes)` guard with `if (n.node_hash && seen.has(n.node_hash)) continue; seen.add(n.node_hash);` BEFORE incrementing entry.total / entry.healthy / pushing to entry.nodeRows.
- Subscription viewMode: maintain the same `const seen = new Set<string>()` for each `g.nodes` iteration; skip duplicate node_hash within one subscription group.

File: src/views/TopologyView.tsx L718-726 (subscription path) + L743-751 (region path)
Lines: ~8
Test 1: region viewMode with two subscriptionGroups both containing node_hash="abc" -> regionRows has exactly 1 row with that display_tag (not 2).
Test 2: subscription viewMode with one subscriptionGroup containing duplicate node_hash "abc" twice -> subgroup nodeRows has 1 row (not 2).

## Phase T17-3: MiniMap ariaLabel replaces div[title] (S3)

Root cause: L955-972 wraps MiniMap in static div[title] which has height 0 due to absolute-positioned child MiniMap panel. The native title tooltip never fires hit-test.

Fix: delete the wrapper div. Pass `ariaLabel={t("topology.minimapHint")}` prop directly to `<MiniMap>` — xyflow 12.11.2 source renders it into an SVG `<title>` which is both the accessibility name and the native hover tooltip (verified in .codex-tmp research file).

File: src/views/TopologyView.tsx L955-972
Lines: -18/+2 (net -16)
Test 1: render TopologyView; assert MiniMap has aria-label prop equal to translated "topology.minimapHint" text.
Test 2: assert no <div title=...> wrapper around MiniMap in the rendered output (anti-regression).

## Phase T17-4: openStrategyConfig merged into CanvasControls toolbar (S4)

Root cause: L942-954 standalone absolute button at bottom-20 left-4 occludes CanvasControls hover region and MiniMap.

Fix: delete the standalone button block. Add a new button row inside CanvasControls (L478-509) with title={t("topology.openStrategyConfig")} and the same onClick body. Place it as the last item in the second row (below zoom/fit group) or to the right of the viewMode segmented control — whichever fits the existing flex layout without reflow.

File: src/views/TopologyView.tsx L942-954 (delete) + L478-509 (insert into CanvasControls)
Lines: -13/+15 (net +2)
Test 1: render TopologyView; query the canvas-controls container; assert there IS a button with title="Open strategy config" inside it.
Test 2: assert NO standalone button with absolute bottom-20 left-4 position remains (anti-regression).

## Phase T17-5: Full gate + exe closed-loop

Run order:

1. `pnpm test` => expect 233 pass (225 baseline + 8 new). No regressions.
2. `npx tsc --noEmit` green.
3. `node scripts/i18n-check.cjs` => expect "343 keys / 18 locales" (no new keys).
4. `pnpm build` green. Note the Vite chunk hash.
5. `cargo build --release -p egressapikey-app --features custom-protocol` green.
6. Copy exe to `release/windows-gui/EgressAPIKEY.exe`.
7. Verify the new Vite chunk hash appears in the staged exe bytes (ascii grep).
8. Smoke: Start-Process exe, assert MainWindowTitle="EgressAPIKEY", WS ~35MB, resin.exe child alive at ~50MB.
9. `codegraph sync .` re-index changed files.
10. git add + commit + push.

## Phase T17-6: Documentation + ADR

- Write ADR-0041 (done above).
- Update docs/CONTEXT.md: add `nodePool` + `aggregationColumn` terms (the C-column concept under Kiali-style invariant).
- Add AGENTS section §59 documenting T17 fix.
- Verify docs/ADR-0040 references updated to note partial supersession by ADR-0041 (S3 + S4).

## Test plan (8 tests, Q4=B confirmed)

| # | Name | Phase | Coverage |
| --- | --- | --- | --- |
| 1 | T17-1: region viewMode excludes subGroup nodes (happy) | T17-1 | S1 guard happy path |
| 2 | T17-1: subscription viewMode still renders subGroup nodes (boundary/regression) | T17-1 | viewMode does not break the other branch |
| 3 | T17-2: region viewMode dedups by node_hash across subscriptions (happy) | T17-2 | S2 dedup happy |
| 4 | T17-2: subscription viewMode dedups duplicate node_hash within one subscription (boundary) | T17-2 | dedup on the other path |
| 5 | T17-3: MiniMap renders ariaLabel translated text (happy) | T17-3 | S3 ariaLabel path |
| 6 | T17-3: no div[title] wrapper around MiniMap (anti-regression) | T17-3 | ensures old antipattern gone |
| 7 | T17-4: CanvasControls contains openStrategyConfig button (happy) | T17-4 | S4 toolbar merge |
| 8 | T17-4: no standalone absolute bottom-20 left-4 button remains (anti-regression) | T17-4 | ensures standalone removed |

Total: 225 + 8 = 233 pass (was 225 after T16).

## Verification criteria (acceptance)

- Vitest 233 pass / 16 files.
- tsc --noEmit green.
- i18n:check 343 keys / 18 locales (unchanged).
- pnpm build green; new chunk hash embedded in exe.
- cargo build --release green.
- Exe smoke: MainWindowTitle=EgressAPIKEY, resin.exe child alive, no stderr.
- codegraph sync green.
- git push to codex/rust-port with the commit.
- End-to-end manual check: switch region <-> subscription viewMode; confirm no stray subGroup cards under entry-port column in region viewMode; confirm MiniMap hover shows localized tooltip; confirm openStrategyConfig button sits inside the left toolbar group, not a standalone floating pill.
