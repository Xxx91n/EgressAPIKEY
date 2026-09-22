# Round 12 Wave B 任务书（r12-wave-b grill 定稿产物，2026-09-22）

> 唯一数据源：`.scratch/r12-wave-b-grill/decision-ledger.md`（D-001..D-003 全 current）
> 上游输入：`.scratch/r12-wave-a/handoffs/2026-09-22-audit-handoff.md`（审计残留 C1..C10）+ `docs/agents/round12-wave-a-next-round.md`（已执行完毕）
> 取证与调研：三轮 atomcode 深度调研（锚点见账本过程注记），零 revised。

## §0 轮次形态（覆盖 D-001）

过滤轮：wave-a 审计移交的 10 项候选按四档规则逐项处置。C3 是唯一设计票（升 P0）；C5 升档并入 bench 扩面；卫生项合并成一批；操作项不立票。

## §1 票表

| 票 | 档位 | 覆盖 | 内容 |
| --- | --- | --- | --- |
| **R12-B1 signal-plane 契约** | P0 设计票 | D-002 全部 | **顺序：ADR-0080 草案 → manifest/§7.5 → 实现**。①verdict 五值枚举 ok/local_fail/remote_fail/skipped/environment_suspect + detail≤64B（§7.5 校验，只进 IPC/audit 不进 ring）；join 矩阵：loopback ok+egress fail=remote_fail、loopback fail+egress fail=local_fail、矛盾态记 detail 按 local_fail。②ring bool→ProbeVerdict；suspect tick 以枚举值入 ring 当时间屏障（评估器跳过、不进 Good-Event 分母）；WindowStats 分母=valid 样本。③返回面加字段不删字段：signals:N 保留 + platform_verdicts 数组（platform/verdict/ok_share/streak，≤平台数有界）；manifest --write 重生成。④共模抑制器：单 tick local_fail 占被探测 enabled bound-port 总数≥80%→suspect；ADR 写 n 退化表+单端口退化语义+引用 wave-a D-001 cooldown 兜底。⑤suspect-streak=tick 级 u32（独立平台 ring），连续 3 写死→audit row（tick 维度）+environment_status 字段。⑥ADR-0074 参数不动；ADR-0080 显式 supersede orchestration.rs majority-rule 段（互链 0074）。⑦同 commit：CONTEXT.md 词条（ProbeVerdict/Common-mode Suppressor）+codegraph sync+必测（join 矩阵、suspect 截断连续性）+UI 投影 skipped 渲染态。登记灰区：WAN 半残误归 remote_fail→detail 带 latency 观测值；解除条件=误切≥1 次/月→重开评估。 |
| **R12-B2 bench 扩面：VPS 全树** | P1 | D-001(C5) | headless 剖面全进程树 idle/startup 补测（Linux 侧 cgroup/进程后代划界，对齐 Windows 侧后代树快照口径）；PERF-BENCH 表补行；验收线 §1 的 headless 行边界由此闭环。 |
| **R12-B3 webview-smoke 排查** | P0 | D-001(C1)+账本事实更新 | 首跑 35730121030 FAILURE（report-only）：s2-nav-switch [data-testid=net-save-btn] 8s 未出现、s4-remove-path 点击路径失败（cardGone:true）、尾部 WebDriver execute/sync 级联。任务=分清选择器漂移 vs 真回归并修复；report-only 语义保持（失败只报不拦，D-003② wave-a 条款随迁）。 |
| **R12-B4 卫生批** | 轮末 | D-001(C4/C7/C9 登记) | 合并边界上限一批：C4 ipcLightweightSet debounced .catch 直修+测试（已文档化缺陷）；C7 markdownlint 前置 pre-merge（verify-build.sh 挂载或分支触发 workflow——PERF-BENCH MD049 合并时才暴露的实证）；C9 flake 观察已入册（实证≥2 次升修复票）。 |
| **操作项（非票）** | — | D-001(C2/C10) | self-hosted bench dispatch＝用户侧动作（fallback 计时 2026-09-22 起算 4 周→本机钉临时代表数）；本地 fix/zcode-GUI-Topology 分支 -D＝wave-a D-004 兑现（重实现已合并验证，执行时按破坏操作规程确认）。 |

**维持现状**（全在登记册，受巡检义务覆盖）：⑤c、tray i18n、Mode A crate、url_encode、RQ 迁移、i18n 四线、bench † 项、NODE_FC（D-003 新入册）、strategy_service 预占线（新入册）、AGENTS.md 32KiB（余量 12B——本轮未动）。

## §2 程序条款

继承 wave-a 任务书 §3 全部条款（runner fallback／再过滤／每轮巡检义务／i18n 触发线登记册）。新增：登记册已同步本轮裁决（C8 verdict+触发线、C6 预占线、C9 观察、webview-smoke fired 状态）。

## §3 执行顺序建议

R12-B3（证据链断点先修）→ R12-B1（ADR 先行）→ R12-B2 → R12-B4 扫尾；操作项并行。

## suggested skills

| 任务 | skills |
| --- | --- |
| R12-B1 设计票 | implement（ADR-0080 草案先行）+ code-review 收尾 |
| R12-B2 / R12-B3 | implement（tdd 于约定缝） |
| R12-B4 卫生批 | implement 小修集合 |
| 全程版本控制 | gitbutler（写面唯一接口） |
| 代码取证 | codegraph 先行 + context-mode 路由 |
| 下轮 grill | grill 系列（先读 wave-b 账本 + 本任务书 + 登记册） |
