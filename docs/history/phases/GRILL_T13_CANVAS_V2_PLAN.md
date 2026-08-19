# GRILL T13 CANVAS V2 PLAN

> Fix branch: TopologyView canvas — strategy-driven dagre layout + flash fix + zustand cache
> Predecessors: T9 (canvas fix commit c45334c), T10 (platform GUI), T11 (platform UX), T12 (platform drag fix)
> ADR-0034

## Grill Decisions (Q1-Q4)

| Q | Topic | Decision | Industry Template |
|---|-------|----------|-------------------|
| Q1 | C 列节点摆布模式 | A — 策略驱动自动摆布 + Kiali Service Graph 风格 | Kiali Service Graph (Istio), ReactFlow data-driven edges |
| Q2 | 画布布局算法 | 甲 — dagre + reactflow 官方推荐 | @xyflow/react dagre example, dagre D3 hierarchical layout |
| Q3 | 空态闪现修复 | A — gate 前移，三条消息包入 opacity gate div | standard opacity gate (existing T9 onInit pattern) |
| Q4 | 缓存机制 | A — zustand 局部 store + shallow compare，零新依赖 | zustand/shallow (already installed), env-manager Svelte pattern |

## Bug-to-Fix Mapping

| User Bug | Root Cause | Fix Phase |
|----------|-----------|-----------|
| 1. C 列节点区域分类平铺，没按 B 策略自动展示 | buildEdges 只按 region_filters 匹配；C 列全量显示不过滤 | T13-1 + T13-2 |
| 2. 画布卡顿 + 连线锚点莫名其妙 | 手工硬编码 position (x:0/300/640); Handle full-area 互相挡 | T13-2 (dagre) + T13-3 (Handle fix) |
| 3. 左上角闪现空态文字 | 三条空状态消息在 opacity-0 gate 外面 | T13-4 |
| 4. 缺缓存机制，频繁 sync 卡顿 | 每次进入画布 sync() 4 IPC 并行 + 全量 setState | T13-5 |

## Execution Plan (6 Phases)

### Phase T13-1: C 列策略驱动过滤 (Q1=A)

- parseSubscriptionGroups 改为两层过滤:
  - L1 = subscription group (折叠 chip: sub name + 被选中节点数 / 总节点数)
  - L2 = 只显示被至少一个平台 region_filters 命中 OR 未被选中但策略为 manual 的节点
  - 未被任何平台选中的节点折叠为 "+N 未绑定" 行 (不默认展开)
- 新增 helper: getSelectedRegions(platforms) — 遍历所有平台 region_filters 返回选中 region 集合
- SubscriptionGroupNode 过滤: node rows 只渲染 selectedRegions 包含其 region 的节点
- buildEdges 只画到被选中的 sub group, 未选中的不生成 B->C edge
- i18n: 1 new key topology.unbound x18 locales
- Test: vitest verifying getSelectedRegions + filtered nodeRows + unbound count
- ~120 lines refactor

### Phase T13-2: dagre 自动布局 (Q2=甲)

- Install dagre: pnpm add dagre @types/dagre (~60KB, mature)
- New helper layoutNodes(nodes, edges): 用 dagre.graphlib 构建有向图
  - rankdir=LR (left-to-right 三列), ranksep=80, nodesep=40
  - dagre 自动分配 {x,y}, 注入 node.position
- 删除手工 position: { x: 0/300/640, y: 60+i*130/120 } 硬编码
- fitView 保留: onInit 里 dagre 布局完成后 fitView
- Test: vitest verifying layoutNodes output has valid positions
- ~80 lines new (layout helper) + ~20 lines position change
- Ponytail: dagre 是 @xyflow/react 官方自动布局推荐库

### Phase T13-3: Handle 锚点逻辑修复 (Q2 衍生)

- Handle 定位修复: source/target Handle 改为固定右/左中点半圆, 不再 100% full-area 覆盖
  - T9-4 的 full-area Handle 是 "碰到盒子就连" 的正确意图, 但 opacity:0 + 100% 覆盖导致拖 card 时 Handle 吃掉 mousedown
  - 改为: Handle 固定在 card 右/左边缘中点 radius 20px, 可见但小巧; connectionRadius=40 保持旁路 snap
- Test: vitest verifying Handle style has fixed position (not full-area)
- ~30 lines CSS + Handle prop changes

### Phase T13-4: 空态闪现修复 (Q3=A)

- Gate 前移: 三条空状态消息移入 opacity-0/100 gate div 内
- sync() 完成后 setReady(true): 当前 ready 只在 onInit 触发, 改为 sync resolve 后也 setReady
- Test: vitest verifying empty-state messages only render inside opacity gate
- ~15 lines (CSS class + div nesting)

### Phase T13-5: zustand 局部 store + shallow 缓存 (Q4=A)

- New file src/store/topologyStore.ts: create<TopologyState> with slices: platforms, subGroups, leases, ports
- sync() 改写: fetch 4 IPC -> topologyStore.setState() with shallow skip
- TopologyView 改用 selector: useTopologyStore(s => s.platforms, shallow) 等
- 恢复 5s 轮询: shallow compare 跳过相同值 setState, 不卡
- visibilitychange 恢复: 窗口重新可见时立即 sync()
- Test: vitest verifying sync() with same data does not trigger re-render (shallow skip)
- ~60 lines new store + ~40 lines TopologyView refactor

### Phase T13-6: 全量门禁 + exe stage + push

- pnpm test (all vitest green)
- cargo test -p resin-core --lib (all cargo green)
- pnpm exec tsc --noEmit
- pnpm i18n:check (new keys x18 locales)
- pnpm build (Vite new chunk hash)
- cargo build --release -p egressapikey-app --features custom-protocol
- exe staged + chunk hash grep verify (AGENTS section 5 hard close-loop)
- codegraph sync .
- git push

## Acceptance Criteria

1. C 列只显示被至少一个平台选中的节点, 未选中的折叠为 "+N 未绑定" — visual + vitest
2. 画布布局由 dagre 自动计算, 无硬编码 position — vitest verifying layoutNodes
3. Handle 不再吃掉 card 拖拽 — vitest verifying Handle style
4. 进入画布不再闪现空态文字 — visual confirm
5. 5s 轮询恢复不卡 — vitest verifying shallow skip re-render
6. 182+ vitest + 120 cargo + tsc + i18n green

## Dependencies

- New dep: dagre + @types/dagre (npm, ~60KB, mature)
- Existing: @xyflow/react ^12.11.0, zustand ^5.0.0 (with shallow module already shipped)

## Industry References (pplx + exa research)

- Kiali Service Graph: Istio service mesh topology, strategy-driven edges, auto-hide untargeted nodes
- @xyflow/react dagre example: https://reactflow.dev/learn/layouting/layouting — official recommended auto-layout
- zustand/shallow: https://docs.pmnd.rs/zustand/guides/prevent-rerenders-with-use-shallow — official perf pattern
- env-manager ProfilePage.svelte: shallow compare + local fetch reuse (Svelte pattern reference)

## Master Goal Prompt

始终遵循 AGENTS.md，加载并使用 ctx_*插件，必要时用 1mcp 的 exa/perplexity 联网搜索，不要产生幻觉推理。Ponytail full 模式。
以下功能都我希望你能联网调研出一个类似功能的模板轮子出来，而不是自己开发
细分好先后顺序和验收标准和test闭环，避免只引入但是却没有做到的情况。
