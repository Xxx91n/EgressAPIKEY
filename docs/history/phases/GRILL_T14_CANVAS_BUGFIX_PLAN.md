# GRILL T14 — Canvas Bug Fix + Node Compression Plan

> grill-with-docs 流程规整结果。Q1-Q5 用户决策已固化。

## Bug 审计（代码级确认）

### Bug 1: A/B 策略顺序翻转
- **根因**: TopologyView.tsx 第 212-224 行，PlatformNode 组件先渲染 bClassLabel 再渲染 aClassLabel
- **修复**: 翻转渲染顺序，aClassLabel 在上，bClassLabel 在下
- **验收**: vitest 验证 A 策略 badge 在 B 策略 badge 之前渲染

### Bug 2: A 策略 i18n 未同步
- **根因**: 第 494 行 aClassLabel 硬编码 "region:" + region_filters.join(",").toUpperCase() 或 "manual"，未走 t()
- **修复**: manual → t("strategy.manual")（已有 key）；region → t("topology.aClassRegion", { regions }) 新增 key
- **新增 i18n**: topology.aClassRegion，en: "Region: {{regions}}"，× 18 locales
- **验收**: vitest 验证 aClassLabel 走 t() + i18n:check green

### Bug 3: C 列冗余节点卡片
- **根因**: 第 509-550 行对每个 subGroup 逐个渲染 filteredNodes 节点行
- **修复**: C 列混合视图（Q1=C），默认按订阅折叠（Q5=A），可切换按地区视图（Q4=A）
- **折叠状态**: 订阅名 + 健康率 + 地区列表（如 "JP(53) US(12) SG(8)"）
- **展开**: 点击展开前 10 个节点行，超出显示 "+N more"
- **切换控件**: CanvasControls 顶部 segmented toggle [订阅] | [地区]
- **验收**: vitest 验证折叠/展开 + 切换 + 节点数 > 10 时只渲染 10 行

### 新增: "回到世界中心"按钮
- **行为 (Q2=A)**: 视口重置到 {x:0, y:0, zoom:1}（坐标系原点 + 100% 缩放）
- **位置**: CanvasControls 按钮组，新增第 5 个按钮（Home 图标）
- **i18n**: 新增 topology.resetCenter × 18 locales
- **验收**: vitest 验证 setViewport({x:0,y:0,zoom:1}) 被调用

## 执行计划（6 Phases）

### Phase T14-1: Bug 1 — A/B 策略顺序翻转
- 翻转 PlatformNode 渲染顺序：aClassLabel 在上，bClassLabel 在下
- Test: vitest 验证 A badge 在 B badge 之前
- ~5 行改动

### Phase T14-2: Bug 2 — A 策略 i18n 同步
- 第 494 行替换硬编码为 t() 调用
- 新增 i18n key topology.aClassRegion 带 {{regions}} 插值 × 18 locales
- manual 复用 t("strategy.manual")
- Test: vitest 验证 aClassLabel 走 t() + i18n:check
- ~10 行 + 18 locale 文件

### Phase T14-3: Bug 3 — C 列按订阅折叠（默认视图）
- 改写 SubscriptionGroupNode：折叠状态聚合信息，展开前 10 个节点 + "+N more"
- useState 管理展开/折叠
- 改写 rawNodes 构建：聚合地区统计而非逐个渲染
- Test: vitest 验证折叠/展开 + 节点数 > 10 时只渲染 10 行
- ~80 行重构

### Phase T14-4: C 列按地区视图（切换视图）
- 新增 viewMode state: "subscription" | "region"
- region 视图：按地区分组，每地区卡片显示节点总数 + 健康率
- 边连到地区而非订阅
- Test: vitest 验证切换 viewMode 后 C 列按地区渲染
- ~60 行新增

### Phase T14-5: 视图切换控件 + "回到世界中心"按钮
- CanvasControls 顶部新增 segmented toggle [订阅] | [地区]
- CanvasControls 新增第 5 个按钮（Home 图标），点击 setViewport({x:0,y:0,zoom:1})
- 新增 i18n: topology.resetCenter + topology.viewSubscription + topology.viewRegion × 18 locales
- Test: vitest 验证切换 toggle + 回到中心按钮
- ~40 行新增

### Phase T14-6: 全量门禁 + exe stage + push
- pnpm test / tsc --noEmit / i18n:check / pnpm build / cargo build --release / exe stage / chunk hash / codegraph sync / git push

## 验收标准

1. PlatformNode 上 A 策略 badge 在 B 策略 badge 之上 — vitest
2. A 策略标签走 t()，"manual" 和 "region:XX" 都有 i18n — vitest + i18n:check
3. C 列默认按订阅折叠，展开显示前 10 个节点 — vitest
4. 可切换为按地区视图 — vitest
5. CanvasControls 顶部有 [订阅] | [地区] segmented toggle — vitest
6. CanvasControls 新增"回到世界中心"按钮，重置视口到 {x:0,y:0,zoom:1} — vitest
7. 191+ vitest + tsc + i18n green

## 联网调研结论

- dagre 支持 compound graph（subgraph/cluster 布局）
- ReactFlow 节点超 100 个明显卡顿，折叠为聚合卡片是行业共识
- Kiali Service Graph 默认聚合视图，点击展开子节点
- 心智模型: 画布 = 路由流向可视化（入口→策略→出口），不是节点全览
