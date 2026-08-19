# GRILL T22 — Canvas a_class-semantic C-column + new-platform default entry

> Canon: ADR-0048 (decisions) + this file (phases).
> Source grill rounds: Q1-Q7 (all A), code-level root cause analysis via codegraph + ctx_execute_file.

## Decisions locked

| Q | Decision | Impact |
|---|----------|--------|
| Q1 | C-column filter by a_class, not region_filters union | buildCColumnGroups rewrite |
| Q2 | New platform auto-default strategyConfig entry in sync merge | sync() merge block +15 lines |
| Q3 | Per-platform a_class branch filtering helper | isNodeSelectedByAnyPlatform ~30 lines |
| Q4 | region + subscription viewMode share filter helper | buildCColumnGroups unify |
| Q5 | buildEdges B->C edge by a_class semantics | buildEdges rewrite ~30 lines |
| Q6 | 12 new vitest closed-loop assertions | TopologyView.test.tsx |
| Q7 | Delete getSelectedRegions + selectedRegions | -15 lines dead code |

## Phase execution plan

### Phase 1 — isNodeSelectedByAnyPlatform helper (Q3+Q4)

**Files**: src/views/TopologyView.tsx

**Work**:
1. New pure helper `isNodeSelectedByAnyPlatform(node: NodeItem, platforms: PlatformFull[], groupKey: string): boolean`
2. Logic: iterate platforms, check a_class per Q3 spec
3. Export it (for direct vitest)

**Tests**: 4 vitest (subscription match, region match, quality all-healthy, manual hash match)

**Est**: ~30 lines + 4 tests

### Phase 2 — buildCColumnGroups signature + filter rewrite (Q1+Q4+Q7)

**Files**: src/views/TopologyView.tsx, src/views/TopologyView.test.tsx

**Work**:
1. Change buildCColumnGroups signature: `selectedRegions: Set<string>` -> `platforms: PlatformFull[]`
2. Replace selectedRegions.has(getNodeRegion(n)) with isNodeSelectedByAnyPlatform(n, platforms, groupKey)
3. Replace selectedRegions.has(region) in region viewMode (L271) with same helper
4. Delete getSelectedRegions (L104-110)
5. Remove selectedRegions useMemo (L859)
6. Update rawNodes useMemo call (L942): pass platforms instead of selectedRegions
7. Update test imports + fix existing buildCColumnGroups tests (L843-859)

**Tests**: update 2 existing tests to pass PlatformFull[] instead of Set<string>

**Est**: ~40 lines refactor + ~10 lines test fix

### Phase 3 — buildEdges a_class-semantic B->C edge (Q5)

**Files**: src/views/TopologyView.tsx, src/views/TopologyView.test.tsx

**Work**:
1. buildEdges subscription viewMode branch (L620-641): replace region_filters-only with a_class switch
2. a_class=subscription+empty subscriptions -> edge to ALL groups
3. a_class=subscription+specific -> edge to matching subscription groups
4. a_class=region -> edge to matching region groups
5. a_class=quality -> edge to ALL groups
6. a_class=manual -> NO group-level edge
7. region viewMode edges (L960-966): same a_class switch
8. Update existing buildEdges tests (L270-316) to pass a_class field

**Tests**: 4 new vitest (subscription+empty=all edges, subscription+specific, region, manual=no edges)

**Est**: ~30 lines refactor + 4 tests

### Phase 4 — sync() merge default strategyConfig entry (Q2)

**Files**: src/views/TopologyView.tsx

**Work**:
1. In sync() merge block (L774-792), after cfgRaw.platforms map build: iterate Resin platforms, if not in map -> auto-create default entry
2. Default: { platform_name, a_class: "subscription", b_class: "random", subscriptions: [], regions: [] }
3. This is in-memory merge only (not persisted to JSON until user configures)

**Tests**: 1 vitest (Resin platform not in strategyConfig -> merge creates default entry)

**Est**: ~15 lines + 1 test

### Phase 5 — Full gate: build + i18n + test + exe smoke

**Files**: N/A (verification only)

**Work**:
1. `pnpm test` — all vitest green (existing 83 + 12 new = 95)
2. `pnpm exec tsc --noEmit` — clean
3. `pnpm build` -> new Vite chunk hash
4. `cargo build --release -p egressapikey-app --features custom-protocol` -> exe
5. Stage exe + resin.exe sidecar to release/windows-gui/
6. Grep Vite chunk hash in exe bytes (stale-bundle guard)
7. Smoke launch: MainWindowTitle="EgressAPIKEY", WorkingSet ~36MB, resin.exe child alive
8. `codegraph sync .` + commit + push

**Acceptance**: all gates green, exe fresh, smoke alive

## Test closed-loop summary

| Test | What it asserts |
|------|----------------|
| isNodeSelectedByAnyPlatform subscription | node in selected subscription -> true |
| isNodeSelectedByAnyPlatform region | node in selected region -> true |
| isNodeSelectedByAnyPlatform quality | healthy node -> true |
| isNodeSelectedByAnyPlatform manual | node_hash in manualNodes -> true |
| isNodeSelectedByAnyPlatform empty default | new platform subscriptions=[] -> all true |
| buildEdges subscription+empty | edges to ALL groups |
| buildEdges subscription+specific | edges to matching groups only |
| buildEdges region | edges to region groups only |
| buildEdges manual | NO group-level edges |
| buildEdges quality | edges to ALL groups |
| sync merge default entry | Resin platform not in strategyConfig -> auto-default |
| buildCColumnGroups updated signature | accepts PlatformFull[] not Set<string> |
