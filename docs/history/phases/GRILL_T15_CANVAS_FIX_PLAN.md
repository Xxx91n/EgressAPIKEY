# GRILL T15 — Canvas Bug Fix + Strategy Pipeline Unification Plan

> grill-with-docs 流程规整结果。Q1-Q5b 用户决策已固化。
> Q5 ADR: [ADR-0036](../docs/adr/0036-strategy-single-source-truth-config.md)

## Bug 审计（代码级确认）

### Bug 1: C 列冗余订阅卡片（Q1=A）
- **根因**: TopologyView.tsx L559 `subGroups.forEach` 无条件遍历所有订阅组，即使没有平台选中该订阅的任何 region，卡片仍然渲染
- **修复**: 在 forEach 内增加过滤条件 —— 如果该订阅的所有节点 region 都不在 `selectedRegions` 中，跳过该订阅组不渲染
- **验收**: vitest 验证无平台选中 region 时 C 列卡片不渲染
- ~5 行改动

### Bug 2: CanvasControls 按钮宽度（Q2=B）
- **根因**: L436 所有按钮在 flex-col 容器里默认 stretch 同宽
- **修复**: segmented toggle 保持全宽；icon 按钮（ZoomIn/ZoomOut/fitView/Home/Lock）加 `w-fit self-center`
- **验收**: vitest 验证 icon 按钮有 w-fit class
- ~5 行 className 改动

### Bug 3: viewMode/locked 状态不持久化（Q3=A）
- **根因**: L490 `useState("subscription")` / L434 `useState(false)` 纯内存，切换页面/重启后丢失
- **修复**:
  - settings.ts: `TopologyViewport` 接口扩展为 `TopologyState { x,y,zoom,viewMode,locked }`
  - `loadTopologyViewport` → `loadTopologyState` 返回完整对象
  - `saveTopologyViewport` → `saveTopologyState` 接收完整对象
  - TopologyView: useState 初始值从 loadTopologyState 读取
  - onMoveEnd 回调保存完整状态（含 viewMode/locked）
  - 保留 P20 opacity gate（ready state）防止闪现回归
- **验收**: vitest 验证 loadTopologyState 返回 viewMode + locked + viewport
- ~40 行改动（settings.ts + TopologyView）

### Bug 4: dagre 布局不从中心生成（Q4=A）
- **根因**: `layoutNodesViaDagre` 只做了中心点→左上角转换，没做图 bounding box 居中偏移
- **修复**: dagre.layout 后读取 `g.graph()` 获取 {width,height}，每节点 position 再减 (width/2, height/2)，使图中心对齐到 (0,0)
- **验收**: vitest 验证布局后节点中心点在坐标系原点附近
- ~5 行改动

### Bug 5: 画布策略绕过白盒配置文件管道（Q5=A, Q5b=A）
- **根因**: TopologyView onConnect/onEdgesDelete 直接 `ipcPlatformUpdate` PATCH Resin，绕过了 PlatformsView 的 strategyConfig JSON 管道
- **修复**:
  - 画布拖线 onConnect: 改为先读取 strategyConfig -> 更新对应平台的 regionFilters 字段 -> `ipcStrategyConfigPut` + `ipcStrategyApply`
  - 画布删除连接 onEdgesDelete: 同上，从 regionFilters 移除对应 region
  - 画布 sync() 时也从 strategyConfig 读取 region_filters（而非仅从 Resin platform 对象）
  - 引入 `ipcStrategyConfigGet` / `ipcStrategyConfigPut` / `ipcStrategyApply` 到 TopologyView
- **验收**: vitest 验证画布拖线调用 strategyConfigPut 而非直接 platformUpdate
- ~60 行改动

## 执行计划（7 Phases）

### Phase T15-1: Bug 1 — 隐藏完全未绑定的订阅卡片
- subGroups.forEach 增加 selectedRegions 过滤
- Test: vitest 验证无平台选中时 C 列无卡片
- ~5 行

### Phase T15-2: Bug 2 — CanvasControls icon 按钮 w-fit
- icon 按钮加 `w-fit self-center` className
- Test: vitest 验证 icon 按钮有 w-fit
- ~5 行

### Phase T15-3: Bug 3 — topologyState 持久化
- settings.ts: TopologyViewport → TopologyState { x,y,zoom,viewMode,locked }
- load/saveTopologyState helper
- TopologyView: useState 初始从 loadTopologyState 读取
- onMoveEnd 保存完整状态
- 保留 P20 opacity gate
- Test: vitest 验证 loadTopologyState + 持久化往返
- ~40 行

### Phase T15-4: Bug 4 — dagre 图中心对齐到 (0,0)
- layoutNodesViaDagre 读 g.graph() {width,height}，偏移每节点
- Test: vitest 验证布局后图中心在原点附近
- ~5 行

### Phase T15-5: Bug 5 — 画布拖线走 strategyConfig 管道
- TopologyView 引入 ipcStrategyConfigGet/Put/Apply
- onConnect/onEdgesDelete 改写: 读 strategyConfig -> 更新 regionFilters -> put + apply
- sync() 也读 strategyConfig 的 region_filters
- Test: vitest 验证拖线调 strategyConfigPut 而非 platformUpdate
- ~60 行

### Phase T15-6: ADR-0036 + AGENTS.md 更新
- ADR-0036 已写入
- AGENTS.md 增加新 section 记录策略统一管道

### Phase T15-7: 全量门禁 + exe stage + push
- pnpm test / tsc --noEmit / i18n:check / pnpm build / cargo build --release / exe stage / chunk hash / codegraph sync / git push

## 验收标准

1. 无平台选中 region 时 C 列不渲染订阅卡片 — vitest
2. CanvasControls icon 按钮 w-fit self-center — vitest
3. viewMode/locked 切换页面后恢复 + 重启后恢复 — vitest
4. 不闪现空态消息（P20 gate 保留）— vitest
5. dagre 布局后图中心在坐标系原点附近 — vitest
6. 画布拖线调 strategyConfigPut 而非直接 platformUpdate — vitest
7. 白盒配置文件能改策略且 GUI 同步 — vitest
8. 214+ vitest + tsc + i18n green

## 依赖

- 无新依赖（复用已有 strategyConfig 管道 + dagre + zustand）

## 联网调研结论

- ReactFlow 官方文档确认 dagre 布局后用 fitView 或手动计算 bounding box 居中
- dagre wiki 确认节点 x,y 是中心坐标，graph.width/height 是整个图尺寸
- 工业级 canvas toolbar 模式：segmented toggle 比 icon 按钮宽（Figma/VSCode 模式）
- Kiali Service Graph: 未绑定的节点不显示在画布上
