# GRILL T9 CANVAS FIX PLAN

> Status: PLANNED (Q1-Q5 all confirmed)
> Date: 2026-08-14
> Branch: codex/rust-port (or new fix/canvas-t9)
> Budget: 100M tokens
> Skills: grill-with-docs, domain-modeling, ponytail (full), neat-freak
> Supersedes: ADR-0025 partial (T4-5 commit 34b5f9b only did region labels)

## Grill Decisions (Q1-Q5)

| Q | Topic | Decision |
|---|-------|----------|
| Q1 | C column node pool display | C — Two-level fold: L1=subscription(collapsible), L2=node list(display_tag+region+latency color+health) |
| Q2 | Drag-to-connect hit area | A — Enlarge Handle to cover entire card (opacity:0, position:absolute, full width/height) |
| Q3 | Controls button tooltip i18n | A — Custom CanvasControls component replaces built-in Controls, lucide icons + t() i18n |
| Q4 | Platform A+B dual strategy | A — Dual-row badge: blue B-class strategy badge + green A-class strategy badge, all i18n |
| Q5 | B->C edge strategy labels | C — Edge shows only A-class strategy source (region:HK / manual / quality>75), B-class stays on platform node |

## Correction: ADR-0025 Unfulfilled Content

| ADR-0025 Planned | T4-5 commit actual | T9 fix |
|-------------------|-------------------|--------|
| C-col=subscription-folded node pool | Still region-flat | Change to subscription fold (L1=sub, L2=node) |
| B-col=B-class strategy name | Only allocation_policy text | Dual badge (A+B), via shell 6-option mapping |
| B->C=strategy-driven edge labels | Only region:HK label | Add manual/region/quality strategy labels |
| Drag B->C=add to manual | Only region_filters PATCH | Keep region PATCH + extend manual selection |
| Multi-strategy coexist multiple edges | Single edge | Support manual + region coexistence |

## Dependencies

- ADR-0022 (strategy_engine.rs) — LANDED (commit 3d95dfe), strategy_engine.rs exists
- ADR-0023 (NodesView tree) — LANDED (commit ca04d0f), NodesView collapsible tree
- ADR-0025 (dynamic pool edges) — PARTIAL (commit 34b5f9b), T9 completes it

## Execution Plan (6 Phases)

### Phase T9-1: C column two-level fold refactor (Q1=C)
- **parseSubscriptionGroups**: Replace parseNodeGroups; group by subscription source first, then by region within
- **SubscriptionGroupNode component**: L1 = subscription (collapsible chip with sub name + node count), L2 = node rows
- **Node rows**: display_tag + region badge + latency color (<200ms green / 200-500ms yellow / >500ms red / timeout gray) + health status
- **Collapsed state**: Default collapsed, expand shows node list
- **New node type**: "subscriptionGroup" replaces "nodeGroup" in nodeTypes
- ~150 lines refactor of TopologyView C column
- **Test**: vitest verifying parseSubscriptionGroups correctly extracts subscription groups + collapse toggle
- **i18n**: New keys topology.subscriptionLabel / topology.nodeCount / topology.latencyMs / topology.timeout x18 locales

### Phase T9-2: B->C edge strategy-driven + strategy labels (Q5=C)
- **buildEdges extension**: Edge label from "region:HK" to full A-class strategy source
  - region_filters match -> label = "region:HK"
  - manual match (future strategy_engine) -> label = "manual"
  - quality match (future) -> label = "quality>75"
- **Multi-strategy coexist**: Multiple A-class strategies hitting same node group -> multiple edges
- **Auto-strategy edges non-deletable**: Only manual edges can be deleted
- **Edge deletion guard**: onEdgesDelete checks edge type; auto edges return without PATCH
- ~80 lines buildEdges + EdgeWithLabel type extension
- **Test**: vitest verifying buildEdges multi-strategy labels + manual deletable / region non-deletable

### Phase T9-3: Platform node A+B dual strategy badge (Q4=A)
- **PlatformNode component**: Add two badge rows below existing sub text
  - Blue badge row: B-class strategy name (via strategyToI18nKey -> t("strategy.random") etc)
  - Green badge row: A-class strategy labels ("manual" / "region:HK,JP" / "quality>75")
- **A-class label generation**: From platform.region_filters + strategy_engine AClassStrategy
- **B-class label**: From platform.allocation_policy via strategyToI18nKey mapping
- ~60 lines PlatformNode component changes
- **Test**: vitest verifying dual badge rendering for different strategy combos + i18n correct

### Phase T9-4: Full-area Handle for drag-to-connect (Q2=A)
- **Handle CSS**: EntryPortNode/PlatformNode/SubscriptionGroupNode Handle changed to:
  - source Handle: style={{opacity:0, position:'absolute', width:'100%', height:'100%', top:0, left:0, pointerEvents:'auto'}}
  - target Handle: same
- **Keep ReactFlow connectionRadius default**: 20px extra snap zone
- **Ensure drag not blocked**: Card remains draggable, Handle overlay only responds to connections
- **preventDefault on Handle mousedown**: Stop drag from triggering when clicking on Handle area
- ~30 lines CSS + Handle prop adjustments
- **Test**: vitest verifying Handle has full-area style class

### Phase T9-5: Custom CanvasControls i18n (Q3=A)
- **CanvasControls component**: lucide icons (ZoomIn/ZoomOut/Maximize/Lock/Unlock)
  - Replaces ReactFlow <Controls /> component
  - Each button tooltip via t("topology.zoomIn") / t("topology.zoomOut") / t("topology.fitView") / t("topology.lock")
- **ReactFlow method calls**: Through useReactFlow() hook for zoomIn/zoomOut/fitView
- **Lock toggle**: Local state, sets interactionEnabled prop on ReactFlow
- ~50 lines new component
- **Test**: vitest verifying CanvasControls i18n tooltip correct
- **i18n**: New keys topology.zoomIn / topology.zoomOut / topology.fitView / topology.lock / topology.unlock x18 locales

### Phase T9-6: Full gate + build + release
- pnpm test (all vitest green)
- cargo test -p resin-core --lib (all cargo green)
- pnpm exec tsc --noEmit (type green)
- pnpm i18n:check (key count aligned x18 locales)
- pnpm build (Vite new chunk hash)
- cargo build --release -p egressapikey-app --features custom-protocol
- exe staged at release/windows-gui/EgressAPIKEY.exe
- chunk hash grep in exe bytes verify
- smoke: MainWindowTitle=EgressAPIKEY, resin.exe child alive
- codegraph sync .

## Acceptance Criteria
1. C column shows subscription-folded tree (not region flat) — visual + vitest parseSubscriptionGroups test green
2. Drag-to-connect hits card anywhere — visual + Handle full-area class test green
3. Controls button tooltips all use i18n — switch zh/en locale confirm non-English
4. Platform nodes show A+B dual strategy badges — visual confirm blue+green badges + vitest test green
5. B->C edge labels show strategy source (not anonymous) — visual + vitest buildEdges test green

## Master Goal Prompt (for future agents)

Follow all grill decisions above. Set goal, start large build, complete all T9 phases.

Always follow AGENTS.md, load and use ctx_* plugins, use 1mcp exa/perplexity for web search if needed, no hallucination. Ponytail full mode.
[$ponytail:ponytail](C:\Users\Administrator\.codex\plugins\cache\ponytail\ponytail\4.8.4\skills\ponytail\SKILL.md)
[$ultragoal](C:\Users\Administrator\.codex\skills\ultragoal\SKILL.md) Set goal to prevent loss.
For each feature, research existing template wheels instead of self-developing.

Sequence phases T9-1 through T9-6 strictly. Each phase must have test closed-loop before marking complete.
