<!--
  Source-of-truth path: .scratch/r11-wave-c-grill/handoffs/next-round.md (session artifact, gitignored by .gitignore:100)
  This docs/agents/ copy is the persistent, git-tracked reference.
  Generated: r11-wave-c grill closeout (D-001..D-005 settled; 2 atomcode research rounds).
  Regenerate this file from the session artifact after any ledger update.
-->
# Round 11 Wave C — Handoff / Task Book（r11-wave-c grill 定稿版）

> 来源：r11-wave-c-grill（2026-09-20 定稿）。决策账本（唯一数据源）：`.scratch/r11-wave-c-grill/decision-ledger.md`（D-001..D-005，全 current）。
> 本文件取代返工窗口起草的 `.scratch/r11-wave-c/handoffs/next-round.md`（修正其两处偏差：观察项未入任务书 → 已立 R11-15/16/17；引用不存在的 `.scratch/r11-wave-b-grill/decisions/` → 实际为 `decision-ledger.md`）。
> 基线：`origin/main @ 4b19d8fe`（wave-B 14 提交线性史；main verify 35494598357 绿，五门实测执行）。
> git-tracked 镜像：`docs/agents/round11-wave-c-next-round.md`。

## 执行纪律（wave-B 审计教训，D-001 约束列）

- 逐 lane `but land`；land 后 `git fetch` 复核 `origin/main` tip + `git cat-file -e` 树内容，再写报告。
- CI-only（ADR-0072）：本机零构建；每源码提交必有 CI verify 证据。
- i18n 改动 ×18 同 commit；权威写入口不可旁路；Resin vendored 不动。
- 叙事 token（R11-xx / wave-x / grill）禁止回灌生产代码（wave-B F4 教训）。
- 新 IPC 命令一律走 `ipcInvoke` 调度口 + capabilities registry + manifest check（R11-03 纪律，wave-B F2 教训）。
- ADR-0062 D5 lockstep 仅适用于新增端点。事实修正（账本过程注记）：`probe_node_egress` / `probe_node_latency` / metrics history probes 均已接线（resin_client.rs:381/393/647）——R11-06 直接复用，无需新增 ResinClient 方法。
- 报告：`.scratch/r11-wave-c/reports/<date>-report.md`；波末节写真实落态（逐 lane sha + 分支清理状态）。
- 轮末归档节奏（D-005③，随 R11-17 执行）：已完结目录（r11-wave-a/b、round11-grill、r11-wave-b-grill、r11-wave-c-grill）统一挪入 `closed-2026-09-XX/`，顶层 20 → ≤12。

## 票据（依赖序）

### R11-16 — 删 auto-clean：期望态删除权威落地【覆盖 D-002；ADR-0073 已立法】

- 移除 `PlatformsView.tsx:120` `cleanedPlatforms` 过滤（含 `setStrategyConfig(cleanedConfig)` 分支）；`config_put` 原样提交完整期望态，绝不以 L3 读取结果裁剪条目。
- 回归测试锁两不变式：① 策略字段同步不得缩减平台条目集合；② 任何以实况列表为输入的写回在实况为空时硬性拒绝（allowEmpty 语义，防 argo-cd#23645 类全量误删）。
- 「白盒有、live 无」交给既有 Drifted 相位 + `reconcile_now` 自愈——复用基础设施，零新配置键。
- 未来钩子（本轮不做）：reconcile 预览式 Prune=confirm 清理入口（显式确认 + audit op + 备份环）。
- 建议最先落地：小而独立，先锁安全不变式再动后续特性。

### R11-06 — 平台级策略编排 controller（P0 主票）【覆盖 D-001、D-003】

- 范围：拆 `strategy_service`（2900+ 行）编排边界（非 7 职责全分解，round11 D-005 ⑤a）；状态机＝风控信号→probe 候选→换区/换策略→观察期→固化/回滚，ejection 冷却防抖。
- 分级混合自动化（D-003）：同一状态机 + per-action 执行闸。动作面＝可逆 PATCH（region_filters / allocation_policy）；大动作（删除/备份回滚/跨平台联动/账号绑定变更）永不在控制器曲目。auto/suggest＝两档 autonomy（suggest 下 transition 停在待确认态并出 diff，人批准才 apply）——不是两套代码路径。默认档：headless＝auto（保守参数包），桌面＝suggest。
- 六不变式参数包（全部可配、出厂保守）：连续失败阈值 / 滑动窗口最小样本量 / 失败率＋慢调用双阈值（慢调用必须算数——风控常表现为挑战/超时非显式 5xx）/ 容忍带相对阈值（新 region 须好 ≥20% 或错误率差 ≥2×）/ 观察期连续 M 成功周期才固化且任一变差重新计时 / 冷却指数退避封顶（Envoy 式）/ 单轮最大切换比例（防全平台雪崩）/ min_switch_interval + 全局 max_switches_per_hour。
- 信号源：端到端 `probe_exit_ip` + `port_health_check` 为主；节点级复用既有 ResinClient probe 方法（见事实修正行）；IP 信誉仅加权（配 key 才启用）。
- 执行面：切换必走权威写入口 config_put→apply（generation + audit）；全部 transition 落 audit.jsonl 供调参；升档纪律：先 suggest 运行收集 audit 数据再放宽默认档。
- 配置归属（票据内定，约束已立）：L2（三层立法排除 L1/L3）——推荐 `egressapikey-strategy.json` 新增可选 `orchestration` 节（serde 缺省＝off，同备份环同审计），不加第三权威写入口。
- 新 IPC 命令（orchestration 状态/配置）走调度口 + registry + manifest check。
- spec 以 vendored resin 源码为准（调研中 per-node circuit-breaker/EWMA 表述未核实，不作为依据）。
- 验收：状态机单测（含两档 autonomy 与 suggest 停确认态）；防抖参数可配；切换全程权威写入口 + 审计；不引入新数据面行为。
- 立法：实现 lane 内写本票 ADR（分级自动化 + 六不变式，引用 D-003 调研来源）。

### R11-12 — 部署文档 + 双层基线【覆盖 D-001、D-004】

- HEADLESS_DEPLOYMENT.md 增补：TLS 反代指引、零信任组网推荐（Tailscale/WireGuard 非依赖）、FD ≥8192 + 内核 socket 缓冲（~174KB/conn）预算、1C1G 最低规格。
- 双层基线（D-004，执行面统一 GitHub Actions）：
  - ① github-hosted Linux runner 一次性记录型基线（无门）：SSE 首 token p99 / 500 并发 RSS / 延迟 → 新建 `docs/PERF-BENCH.md`「shared-runner 参考列」。
  - ② self-hosted runner 代表性绝对基线：用户目标硬件（1C1G Linux VPS 优先，本机 Windows 兜底）注册 runner；baseline workflow 仅 `workflow_dispatch` 手动触发；跑 CI 构建产物；数字上传 artifact + 回填 PERF-BENCH.md「代表性硬件列」——该列是 RSS ≤150MB / ≤80MB 钉数与绝对门收紧的唯一依据。
  - 安全轨（仓库 PUBLIC）：baseline workflow 仅 workflow_dispatch；任何 pull_request/push 触发的 job 禁止引用 self-hosted label。
- wave-B 校准项：SSE 首 token p99（票线 1ms，Windows 实测 1.14ms——CI Linux 数值需基线）；500 并发 RSS 绝对门；shared-runner 延迟基线。现断言形态 median≤1ms + p99≤3ms / 512KiB/conn——基线后收紧。
- round11 D-004 立法不变：共享 runner 不加硬绝对门。
- 外部取证：atomcode-research（TLS 反代 / 零信任组网，串行单在飞）。

### R11-07 — key→账号反查面【覆盖 D-001；round11 D-002 D】

- 按 API key 反查所属账号 + 绑定出口 IP/端口；不解析请求体；只读 UI 呈现。
- headless parity：新 IPC 命令走调度口 + capabilities registry + manifest check。
- 1 天，低风险。

### R11-15 — 卫生小票【覆盖 D-001】

- patchAndSync React updater + 微任务竞态；HeadlessStore last-writer-wins；`proxy_to_resin` 每请求新建 reqwest::Client；`query_param` 同名两义（headless_main vs headless_security——改命名或注释锁定语义，先读测试）；远端旧分支 `fix/zcode-GUI-Topology` 删除。
- 每项独立小 commit；触碰即清。

### R11-17 — 治理减法 + guard 补落地【覆盖 D-005】

- ① AGENTS.md 回落 ≤32KiB：ipc-manifest 全量块（~8KB 机器生成清单）外移 docs/agents/（正文留指针 + 计数，生成源 `scripts/ipc-manifest-check.cjs`）+ Storage locations 细节外移（渐进披露惯例）。不得弱化 context-mode 路由块与 CodeGraph 块（保持置顶）；外移内容 + 指针链接同 commit（dead-link 门）。
- ② verify-build.sh 补 AGENTS.md ≤32KiB size guard（Phase 6 声称但未落地；实测磁盘 ~51KB，较 Phase 5 目标 21.5KiB 回弹 2.4×）。
- ③ .scratch 归档：见执行纪律末条。
- 约束：18 语言不裁；ADR/docs 知识不删；不加新治理工具/流程。

### R11-13 — 轮末评估与立票【覆盖 D-001】

- (a) React Query 迁移触发评估（清点手写轮询点位，≥3 且均为 server-state 则立专项）。
- (b) 真 WebView 冒烟 E2E 立票（CDP tauri-driver，Windows/Linux 矩阵 post-merge/nightly；macOS 缺口文档化）。
- (c) tray i18n 构建期生成挂起确认（下次 i18n 需求触发）。

## 范围外（显式，账本依据）

- fork line：触发条件维持唯一条（round11 D-002「同一账号内部、单请求按 model 字段动态换出口」），本轮不触碰；壳层不重返数据面内核（ADR-0050）。
- Mode A 独立 crate（锐评1 建议）：deferred 无主，不立项。
- 18 语言裁剪 / docs 大合并 / ADR 删减：D-005 显式不做。

## suggested skills（下一 Agent）

| 票 | skills |
| --- | --- |
| R11-16 / R11-07 | implement（tdd：先锁两不变式/只读契约）+ code-review |
| R11-06 | implement（tdd：状态机单测先行）+ code-review；lane 内写 ADR |
| R11-12 | atomcode-research（TLS/零信任取证，串行）+ implement |
| R11-15 / R11-17 | implement + code-review（触碰即清） |
| R11-13 | research / triage（评估产出立票建议） |
| 全程 | gitbutler（but 唯一写面；lane 配方先读技能） |
| 轮末 | neat-freak + handoff + code-review（审计双轴） |
