# GRILL_T16 Canvas Region Filter + Port Drag-Bind + MiniMap i18n

> ADR-0040: 0040-canvas-region-filter-port-bind-minimap.md
> Fixed point: 04929a4 (T15-v3-audit)
> Grilling: Q1=A, Q2=A+port-drag, Q3=A+tooltip-i18n

## Phase T16-1: Region viewMode filter guard

Root cause: TopologyView.tsx L737-756 region viewMode iterates all subGroups nodes and generates region group cards for every region that has nodes, without checking selectedRegions.

Fix: In the if (viewMode === "region") block, add a guard: if (!selectedRegions.has(region)) continue; before pushing the regiongroup node.

File: src/views/TopologyView.tsx L737-756
Lines: ~3
Test: vitest render TopologyView with mocked platforms region_filters=["jp"], mock subGroups with nodes in jp+us+kr, assert only regiongroup-jp appears.

## Phase T16-2: Port drag-bind onConnect

Root cause: onConnect (L812) only handles conn.source.startsWith("platform-"). Dragging from entry-port to platform is silently ignored.

Fix: Add new branch at top of onConnect for entry-port- to platform- connections, calling ipcPortBindPlatform(port, platformName) then sync().

File: src/views/TopologyView.tsx L812 onConnect
Lines: ~12
Test: vitest mock ipcPortBindPlatform, simulate onConnect source="entry-port-1790" target="platform-Default", assert ipcPortBindPlatform called with (1790, Default) and sync called.

## Phase T16-3: MiniMap i18n tooltip + nodeColor

Fix: Wrap MiniMap in div with i18n title, add nodeColor function coloring nodes by type (entryPort=blue, platform=purple, subscriptionGroup=green, regionGroup=amber), add maskColor.

New i18n key: topology.minimapHint x18 locales.

File: src/views/TopologyView.tsx L925 MiniMap section
Lines: ~15
Test: vitest render TopologyView, assert MiniMap wrapper div has title attribute with translated text.

## Phase T16-4: Canvas open strategy config entry

Fix: Add button near CanvasControls that opens strategy config file in OS default editor. Reuses ipcGetConfigDir + egressapikey-strategy.json filename + openPath.

New i18n key: topology.openStrategyConfig x18 locales.

File: src/views/TopologyView.tsx CanvasControls area
Lines: ~10
Test: vitest render TopologyView, click open-config button, assert ipcGetConfigDir and openPath called.

## Phase T16-5: Full gate + exe closed-loop

pnpm test >=228 pass (was 225, +3 new), tsc green, i18n:check 343 keys, pnpm build green, cargo build release green, stage exe, verify chunk hash, smoke, codegraph sync, git commit + push.

## Phase T16-6: AGENTS.md SS58 + CONTEXT.md update

AGENTS.md: add section 58 documenting 3 fixes. CONTEXT.md: add terms selectedRegions, port_bind_platform, viewMode, MiniMap.