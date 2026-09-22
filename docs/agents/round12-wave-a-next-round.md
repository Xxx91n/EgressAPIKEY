# Round 12 Wave A 任务书（r12-wave-a grill 定稿产物，2026-09-22）

> 唯一数据源：`.scratch/r12-wave-a-grill/decision-ledger.md`（D-001..D-005 全 current）
> 上游输入：`docs/agents/round12-next-round.md`（候选票书，本文件为其 adjudicated 版）+ `.scratch/r11-wave-c/handoffs/2026-09-22-audit-handoff.md` + `.codex-tmp/锐评{1,2}.md`
> 取证与调研：五轮 atomcode 深度调研（session 锚点见账本过程注记），全部结论与既往 current 决策同向，零 revised。

## §0 轮次形态（覆盖 D-001）

本轮为**目标轮**：验收线（§1）先行立法，全部候选票过线过滤重排优先级。过滤规则四档（已立法）：直通验收线→升档；承保路径依赖→保留/并入；与线无因果但有独立价值→降档留票面；纯结构债→挂起待触发线。

## §1 部署剖面验收线 spec（覆盖 D-002）

双部署剖面：**低配 VPS headless Linux**（1C1G 级，egressapikey-headless+sidecar，无人值守）/ **普通 Windows 桌面**（Tauri GUI+WebView2 进程树+sidecar）。

| 指标 | 口径 | 目标阈 | 危险阈 | 现状（shared-runner 参考列） |
| --- | --- | --- | --- | --- |
| SSE TTFB 附加 delta | paired p95 | ≤5ms | ≤30ms | 26.2ms |
| SSE event gap 附加 delta | 好事件占比 | ≤10ms @≥99% | ≤500ms @≥99% | Mode A 待测；Mode B 4619ms（登记豁免） |
| idle RSS 全进程树 | 绝对 | ≤80MB † | ≤120MB † | 95.6MB |
| soak RSS @200 SSE 流 | 绝对 | ≤128MB † | ≤250MB † | 85.9MB |
| paired 附加延迟 | delta p99 | ≤1ms | ≤3ms | 0.389ms |
| 吞吐 | 达成率+错误率+δp99 | ≥99% @500rps | ≥95% | 499.9rps |
| 冷启动→MainWindowTitle 就绪 | N 次取 p50/p95 | ≤500ms | ≤1s | 171ms |
| Windows 桌面追加 | 全进程树 WorkingSet（含 msedgewebview2 子进程组）/WebView2 子组内存单列/全应用稳态 | ≤128MB † | — | 待测 |

- †＝暂定、门未激活；待 self-hosted runner（1C1G VPS 优先）实测钉绝对数。shared-runner 数字 warn-only，禁反推目标硬件。
- Mode B SSE 缓冲豁免＝**登记型豁免**（写入 PERF-BENCH「Known measured behavior」节 + upstream manifest 注记；解除条件＝上游实现 SSE 特判 flush 即删）。
- 适用域：不做竞品对标；Mode B 数据面性能属 Resin 领域；event gap 钉附加 delta 非绝对间隔（防测上游 WAN 抖动）。
- SSE 行为面四断言（断连级联取消／有界背压／逐事件 flush／带内错误信令）并入验收。

## §2 票表（覆盖 D-003 / D-004 / D-005）

| 票 | 档位 | 覆盖 | 内容 |
| --- | --- | --- | --- |
| **R12-00 验收线落地** | 恒 P0 | D-002 全部；D-003①②；D-005 修正① | bench 扩展：Windows 全进程树 WorkingSet（启动时后代树快照/job object 划界，禁进程名匹配——WebView2 共享运行时会跨应用串数）、好事件占比+达成率口径实现、SSE 四行为断言、Mode B 豁免登记写法、PERF-BENCH 表结构更新；冷启动门 N 次取 p50/p95（Defender/冷盘方差）；**测量阶段与冒烟断言阶段失败语义分离**（测量数字永远是记录，冒烟失败才报）。**R12-02 并入本票**（三条件随迁：冒烟断言集≤5 条原样保留不稀释；失败只报不拦；macOS 缺口文档化；体量失控→同分支两 PR 系列而非两票）；e2e remove 路径断言弱化（F-11）随冒烟集补回；ar/hi/th 字形不溢出截图断言顺路携带。产出物含「D-004 预算框架升格为验收线」的 ADR 草案。 |
| **R12-03 编排信号面补全** | P0 | D-003③ | 端到端出口探针接入 collect_verdict（bound port→经该 port 的 exit probe，慢调用/超时算失败）+ list_nodes failure_count 口径复核（窗口化/EWMA）。**硬约束**：SLI 探针为独立有界路径（每 tick 按 bound port 数封顶），不寄生 orchestration enabled 开关——编排控制器只是其消费者，关编排不得失明验收线。 |
| **R12-05 canvas write/read fault 救援** | 独立小票（0.5-1 天） | D-004 | 以 fix/zcode-GUI-Topology 的 4d9fa39e+255eb7b8 diff 作**规格说明书**（非补丁）在 main 重实现四不变量：patchSubscriptionsViaStrategyConfig 写 subscription intent+a_class:'subscription'；onEdgesDelete 改 subscriptions 减法；边 id 加 viewMode 前缀强 remount；无条件边不带标签。+210 行测试整体移植并适配当前 API；commit body 注 provenance（recovered from fix/zcode-GUI-Topology @ 4d9fa39e, reimplemented for CanvasV4）；**分支保留至合并验证后才删**。ADR 重编号：分支 0052→0075 直接入列（加「重编号自分支、2026-09-22 对 main 复核仍有效」注记）；0050/0051 先 triage 对照 CanvasV4 现状，仍成立编 0076/0077、已吸收记 not-ported；CONTEXT.md 词条（Subscription Intent/无标签无条件边）随 0075 同 commit。禁机械 cherry-pick。 |
| **R12-06 Resin 定位评估** | 0.5-1 天 | D-003④ | 「产品真实形态是否=Resin 本身、壳层价值边界在哪」评估：resin/.git vendored fork 自带 Dockerfile/webui 的价值归属、mere-aggregation REST 缝纯洁性的结构性风险、README/验收线 §⑧ 适用理由复核。**输出为 ADR 草案，非重构授权**。 |
| **R12-01 React Query** | 劈半降档 | D-003⑤ | 0.5 天评估照做（Tauri invoke 双通道 queryFn 形态、SettingsView debounce 写入是否 mutation）；迁移挂起，预写解封条件：(a) 评估发现具体正确性/性能收益；(b) 下一特性反正触碰三条缝。 |
| **R12-04 杂项扫尾** | 降档轮末 | D-003⑥⑦；D-005 触发线登记 | F-7..F-13 观察项打包（除 F-11 已移 R12-00）+ 残余叙事 token 清零裁决 + strategy_service 4000 行天花板 CI guard（仿 AGENTS.md size guard；触碰即拆口径维持）+ i18n 四触发线登记入册。 |

**维持现状**（挂起/触发线制，全受 §3 巡检义务覆盖）：TopologyView ⑤c churn 触发、tray i18n 手工表、Mode A 独立 crate deferred、url_encode_segment 触碰即换、converge 迁移挂起（见 R12-01）。

## §3 程序条款（覆盖 D-003⑨ / D-005）

1. **runner 逾期 fallback**：self-hosted runner 注册逾期 4 周 → 用本机 Windows 剖面钉临时代表数，显式标注「非目标硬件」。
2. **再过滤条款**：self-hosted 首跑回填后，§2 全部降档/挂起项重新过一遍验收线。
3. **巡检义务**：每轮 grill 开工前必须产出锐评/债项完成度快照（本账本「锐评完成度快照」节为模板），触发线制挂起项全覆盖。
4. **i18n 触发线登记册**（D-005）：需求信号（≥3 独立非机器人 issue 指向同一非 en/zh locale）／采用证据（某非英语 locale 占比≥15%）／维护摩擦（tray.rs 年内≥2 次 locale 返工 或 i18n-check 失败≥1/月）／质量证伪（RTL/字形/溢出目验缺陷→干净退役：删目录+门控条目+tray 收缩）。AGENTS.md §3 定位声明已随本轮整理落地。

## §4 操作项（非票）

- release-matrix dispatch 复跑 e2e 修复 6d035fdc（执行细节，随 R12-00 节奏）。
- self-hosted runner 注册＝用户侧动作；逾期自动走 §3-1 fallback。

## 执行顺序建议

R12-00 先行（SLO 铁律：先 SLI 管道后门）→ R12-03 / R12-05 / R12-06 可并行 → R12-01 评估 → R12-04 轮末扫尾。

## suggested skills

| 任务 | skills |
| --- | --- |
| R12-00 / R12-03 / R12-05 实现 | implement（tdd 于约定缝）+ code-review 收尾 |
| R12-06 定位评估 / R12-01 评估 | research / atomcode-research（串行单发） |
| 全程版本控制 | gitbutler（写面唯一接口） |
| 代码取证 | codegraph 先行 + context-mode 路由（ctx_* 优先） |
| 下轮 grill | grill 系列（先读本账本 + 本任务书） |
