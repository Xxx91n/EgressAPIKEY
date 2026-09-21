# Round 12 任务书（wave C 轮末评估产物，R11-13）

> 来源：round 11 wave C 收束评估（2026-09-21）。本文件由 R11-13 票产出：
> 轮末清点 → 立票。上游任务书：docs/agents/round11-wave-c-next-round.md；
> 决策账本：.scratch/r11-wave-c-grill/decision-ledger.md（D-001..D-005）。

## Wave C 收口快照

| 票 | 落点 | CI |
| --- | --- | --- |
| R11-16 删 auto-clean（D-002/ADR-0073） | `5da19d3b` — `cleanedPlatforms` 过滤移除 + 双不变式回归测试 | verify `35609687230` 绿 |
| R11-15 卫生 ×6 + 远端分支删除 | `fc7cf134`/`1da10a18`/`4af1922b`/`0e24c4da`/`340206ea`/`e0b0ed71`/`5968c3cb`/`f92668d1` | verify 绿（fmt/tsc 两轮修复后） |
| R11-07 key→账号反查 | `1dc4ffae` + fmt `c8ca1b2d` | verify `35614840192` 绿 |
| R11-17 治理减法 + guard | `7ee61023` — AGENTS.md 51KB→32.3KB + size guard + manifest 外移 + .scratch 归档 | verify `35615897698` 绿 |
| R11-06 编排 controller（D-003/ADR-0074） | `60e1f8bb` + `242e194d`/`9a231013` 修复 | 见当轮 verify 记录 |
| R11-12 部署文档+双层基线 | `242e194d`（文档+workflow 随 R11-06 修复 commit 一起落地，票据归属混合已披露） | perf-bench `35619850050` 记录型基线 |

## (a) React Query 迁移触发评估 — 结论：立专项 R12-01

手写轮询点位清点（全部 server-state）：

| 点位 | 机制 | 语义 |
| --- | --- | --- |
| `appStore.subscribeToConverge` | `setTimeout` 链（5s/30s） | 全局快照收敛 |
| `TopologyView.sync` | usePoll 5s | 拓扑快照 |
| `NodesView.refresh` | usePoll 10s | 节点列表 |
| `DiagnosticsView.refreshDiagnostics` | usePoll 动态间隔 | 诊断 |
| `SubscriptionsView.pollEdge` | usePoll 30s | 边缘状态 |
| `PlatformsView.refreshOrch` | usePoll 15s（R11-06 新增） | 编排待批提案 |
| 各 view mount 拉取 + `HeadlessStore` | 一次性 fetch | server-state 首载 |

清点结果：**≥3 且均为 server-state → 触发条件命中，立票**。但注意收敛现状——轮询已统一在 `usePoll` 钩子（usePoll.ts:1-98）+ `convergeSubscribe` 单缝之后，迁移面 = 替换 usePoll 内部实现 + converge 订阅 + 挂载拉取，而非逐点重写。

### R12-01 — React Query 采用评估与（可选）迁移

- **范围**：把 usePoll/convergeSubscribe/mount-fetch 三类 server-state 面映射到 React Query（queryKey 规范、staleTime 替代手写 interval、refetchOnWindowFocus 替代 visibilitychange、mutation 替代手写 patchAndSync 链）。
- **判定前置**：先回答 React Query 在 Tauri WebView + headless BFF 双通道下的 invoke 包装形态（queryFn = ipcInvoke），以及 SettingsView 的 debounce 写入是否属于 mutation。
- **约束**：不引入第二个数据层；usePoll 对外签名保持兼容或一次性全量替换；headless dispatch 路径不变。
- **工作量**：评估 0.5 天；若迁移 2-3 天；**风险**：中（收敛时序敏感——`convergeSubscribe` 的 5s/30s 节奏是漂移检测语义的一部分）。

## (b) 真 WebView 冒烟 E2E — 立票 R12-02

现状：`e2e/` 5 个 Playwright spec 全部打 vite dev server（playwright.config.ts `webServer: pnpm dev`），**不覆盖真 WebView/Tauri 运行时**——IPC 桩、tray、sidecar 生命周期、窗口行为零覆盖。

### R12-02 — WebView 冒烟 E2E（tauri-driver / WebDriver）

- **范围**：tauri-driver + WebDriverIO（或 CDP 直连）对 **CI 构建产物** 跑冒烟集：窗口拉起、`MainWindowTitle=="EgressAPIKEY"` 活性断言（沿用 ADR-0072 测活协议）、nav 切页、至少一条 IPC 往返（如 `port_list`）。
- **矩阵**：windows-latest + ubuntu-latest（WebKitGTK），post-merge/nightly 触发（不进 PR 门）；macOS 缺口显式文档化（runner 许可 + tauri-driver macOS 限制）。
- **约束**：CI-only 执行（ADR-0072）；冒烟集 ≤5 分钟；失败只报不拦（首轮稳定后再升 gate）。
- **工作量**：1-2 天；**风险**：低-中（CI 环境 WebView2/WebKitGTK 依赖安装是主要摩擦）。

## (c) tray i18n 构建期生成 — 维持挂起

现状：`src-tauri/src/tray.rs` 静态表覆盖 18 locale，靠「同一 commit 同步 + i18n-check 门」手工 lockstep（tray.rs:60-62 注释约束）。构建期从 frontend catalog 生成的方案**继续挂起**——当前双目录（JSON catalog × Rust 表）靠门控守住一致性，零事故记录；触发条件维持不变（下一次 i18n 需求或 lockstep 事故）。

## 移交项（非立票，记录）

- `strategy_service.rs` ~2900 行 7 职责——R11-06 只加了 `orchestration_mutate` 状态写，未拆职责（任务书范围外）。
- `TopologyView.tsx` ~1271 行——⑤c 触发线维持（churn 触发再拆）。
- converge 模块级全局——R11-13a 原点位，已由本评估 (a) 并入 R12-01 的迁移面描述。
- self-hosted runner 注册（R11-12 ② 的用户侧动作）：`bench-selfhosted.yml` 已就位，等用户在目标硬件注册 runner 后 workflow_dispatch 触发，数字回填 PERF-BENCH.md 代表列。
