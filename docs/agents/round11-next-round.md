<!--
  Source-of-truth path: .scratch/round11-grill/handoffs/next-round.md (session artifact, gitignored by .gitignore:100)
  This docs/agents/ copy is the persistent, git-tracked reference.
  Generated: Round 11 grill closeout (D-001..D-007 settled; two fresh critiques adjudicated; 5 atomcode research rounds).
  Regenerate this file from the session artifact after any ledger update.
-->
# Round 11 Next-Round Task Book

> **数据源纪律**：本任务书唯一数据源是 `.scratch/round11-grill/decision-ledger.md`（D-001..D-007 current，7 条，0 revised/stale）。不从对话记忆补充结论。
> **对账闸**：6+1 条全部有去向（计划表条目号或显式范围外+理由），无去向记录清单为空（2026-09-19 复验）。
> **总纲（D-001）**：双轨分工——本轮主战场＝壳层（headless 控制面 + Windows GUI）；Resin vendored 原样不动；壳层不重返数据面内核（ADR-0050）；fork 触发条件未命中前禁止启动 fork。
> **构建纪律（ADR-0072）**：CI-only——本机零构建；交付证据＝CI 产物；verify job 每 push 必跑，release dispatch 按需。
> **调研存档**（ctx_search 按 source 检索）：atomcode-q2-multistrategy / atomcode-q3-vps-mgmt / atomcode-q4-perf-budget / atomcode-q5-debt / atomcode-q6-governance。

## 任务清单

### R11-01: AGENTS.md 双铁律仲裁 + ADR-0072（治理，小票）✅ 已于整理阶段完成
- **覆盖决策**: D-006#1
- **已落地**: P23 硬闭环三处删除并重写为 CI-only close-loop；ADR-0072 记录转型决策（与本任务书同一提交系列）
- **余量（残留项）**: 若 CI release job 未承载 chunk-hash 校验（现仅在 build-all 脚本内），把 T10 hash guard 移植/挂接到 CI 产物校验（半天）
- **风险**: 低

### R11-02: 根目录盘点 + 归档修正（治理，小票）
- **覆盖决策**: D-006#2、#5；吸收 Round10 backlog P1-②（归档报告 2 处 "43" 残留清理）
- **范围**: ~~.scratch 移出追踪~~（2026-09-19 事实修正：`.scratch/` 自始被 `.gitignore:100` 忽略、`git ls-files .scratch`=0，D-006#2 的 untrack 机制无对象，意图由现状满足）；`repro/`、`bench-results/`、`npm/` 逐个处置（归位 tests/fixtures 或 scripts/，或移出追踪）；`llms.txt` 保留（AI agent 事实标准）；修正归档报告中 2 处 "43" 计数残留
- **工作量**: 半天；**风险**: 低（纯 git 索引/文档，不删文件）

### R11-03: headless 管理面对等补齐（P0 主票）
- **覆盖决策**: D-003（全部）+ D-006#4a
- **范围**: ①修复 `/api/v1/ports/{port}` axum 0.7 字面路径 bug（Round10 backlog P1-① 吸收）；②CMD_TO_HTTP 40 → SPA 可达功能全覆盖（shell 侧命令由 BFF 原生实现，非透传）；③新增 `GET /api/v1/capabilities`（BFF 原生端点，不增 IPC manifest）；④DISABLED_COMMANDS 44 条按判据重裁（仅桌面环境假设类可禁，纯控制面命令转正）；⑤`X-Forwarded-Proto` scheme 检测 + CSP `frame-ancestors none`；⑥HTTP 冒烟脚本（非浏览器，断言新路由）挂入 CI verify job
- **工作量**: 3-5 天；**风险**: 中（BFF 面广）；**依赖**: 建议 R11-04 先行
- **验收**: capabilities 端点返回机器可读清单；禁用清单缩小到桌面专属；冒烟脚本在 CI 绿

### R11-04: ipc.ts 边界 preparatory 拆分（R11-03 前置）
- **覆盖决策**: D-005⑤b
- **范围**: 从 ipc.ts（2326 行）抽出路由表（CMD_TO_HTTP）/capabilities/guard 三块为独立模块——只拆 R11-03 需要的边界，不做全量大扫除（rabbit-hole 警告）
- **工作量**: 半天-1 天；**风险**: 低-中（行为不变，测试锁）

### R11-05: 策略诚实化 + 模板 + 区域级近似
- **覆盖决策**: D-002（A + C）
- **范围**: UI 如实呈现三策略（BALANCED / PREFER_LOW_LATENCY / PREFER_IDLE_IP）；策略模板/预设——经 ADR-0036/0052 权威写入口的预置快照，一键应用到平台（generation bump + 审计同行）；A-class top-N 落地为区域集的语义明示化；i18n ×18 同步
- **工作量**: 2-3 天；**风险**: 低-中

### R11-06: 平台级策略编排 controller
- **覆盖决策**: D-002（B）+ D-005⑤a
- **范围**: 先拆 strategy_service（2927 行）的编排边界（不做 7 职责全分解）；条件自动化状态机：风控信号 → probe 候选 → 换区/换策略 → 观察期 → 固化/回滚，含 ejection 冷却防抖；复用 Resin probe（probe-egress/probe-latency）与 platform PATCH API；衔接 R10 票 08 的 Inline-Polling driver 模式
- **工作量**: 3-5 天；**风险**: 中；**依赖**: R11-05（语义先行）
- **验收**: 状态机有单测；防抖参数可配；切换全程走权威写入口 + 审计

### R11-07: key→账号→绑定映射查询面
- **覆盖决策**: D-002（D 退化路径）
- **范围**: 按 API key 反查所属账号与其绑定出口 IP/端口的查询能力与 UI 呈现（不解析请求体）
- **工作量**: 1 天；**风险**: 低

### R11-08: SQLite 写路径修复
- **覆盖决策**: D-004 + D-005②
- **范围**: replace_ports 全局锁修复——WAL + BEGIN IMMEDIATE + busy_timeout ≥5s，写事务保持极短（在 ADR-0011 Mutex+spawn_blocking 模式上演进）
- **工作量**: 1 天；**风险**: 中低（db 层，测试齐全）
- **验收**: 控制面写操作 p99 ≤50ms（D-004 验收线）

### R11-09: 数据面测试扩展
- **覆盖决策**: D-004
- **范围**: forwarder 真 socket 测试补 SSE 双指标断言（首 token 增量 ≤1ms、帧不合并/frames-per-arrival）+ 新增延迟 p99 ≤10ms；500 并发 SSE 流内存有界；现有 p95 ≤5ms 断言维持
- **工作量**: 1 天；**风险**: 低（真 socket 测试模式已存在）
- **注意**: 共享 runner 不加硬绝对门；内存/并发绝对值归 R11-12 基线实测

### R11-10: gen_token 统一熵源 + TOCTOU 修复
- **覆盖决策**: D-005①③
- **范围**: sidecar.rs gen_token()（SystemTime+pid）→ getrandom CSPRNG（对齐 headless_security；安全债 High/CWE-330/340，SLA 无协商）；pick_free_loopback_port bind→drop→spawn 竞态修复
- **工作量**: 半天；**风险**: 低

### R11-11: 卫生·触碰即换
- **覆盖决策**: D-005④⑦ + D-007
- **范围**: 本轮触碰模块的手写编解码换 crate（b64/hex/urlencoding；civil_from_days 优先——off-by-one 高发区）；items_arr 三处去重；PBKDF2 主线程存疑核验（backup.rs KDF 常量，若在则 spawn_blocking 化）；本轮触碰文件中的 Txx/roundN/handoff 历史注释随触碰清理（改为 phase-history 指针或删除）
- **工作量**: 半天-1 天；**风险**: 低

### R11-12: 部署文档 + 基线实测
- **覆盖决策**: D-003（部署文档部分）+ D-004
- **范围**: HEADLESS_DEPLOYMENT.md 增补——TLS 反代指引、零信任组网推荐项（Tailscale/WireGuard，非依赖）、FD ≥8192 与内核 socket 缓冲（~174KB/连接）预算表、1C1G 最低规格；headless 全栈 RSS 基线实测（目标总 ≤150MB / 空闲 ≤80MB）——用 CI 产物在代表性硬件测，结果入 PERF-BENCH.md
- **工作量**: 1 天；**风险**: 低（实测需用户硬件或发版窗口）

### R11-13: 轮末评估与立票
- **覆盖决策**: D-005⑥⑧ + D-006#4b
- **范围**: (a) React Query 迁移触发评估——R11-03 落地后清点手写轮询点位，≥3 且均为 server state 快照则启动迁移专项；(b) 真 WebView 冒烟 E2E 立票（5-15 条关键路径，CDP 模式 tauri-plugin-playwright / tauri-driver，Windows/Linux CI 矩阵，post-merge/nightly；macOS 无桌面 WebDriver 缺口文档化）；(c) tray i18n 构建期生成挂起确认（下次 i18n 需求触发）
- **工作量**: 半天；**风险**: 低

## 波次建议（由依赖推导，非强制）
- **波次 A（可并行）**: R11-02、R11-04、R11-10、R11-11
- **波次 B**: R11-03（R11-04 后）、R11-05、R11-08、R11-09
- **波次 C**: R11-06（R11-05 后）、R11-07、R11-12
- **波次 D（轮末）**: R11-13

## 范围外记录（显式，无需立票）

| 项 | 理由（来源） |
|---|---|
| Resin fork 线 / 节点级 pin | D-001/D-002：触发条件「同一账号内部、单请求按 model 字段动态换出口」未命中 |
| 应用层前置网关（LiteLLM 模式） | D-002：远期可选组件；Mode B 双跳延迟与低延时目标冲突 |
| 桌面客户端远程管理 | D-003：三触发条件均未命中 |
| API-first（弃完整 UI） | D-003：不采纳 |
| strategy_service 7 职责全分解 / TopologyView 拆分 | D-005⑤：只拆特性边界；⑤c churn 触发 |
| React Query 全迁移 | D-005⑥：R11-13 触发评估 |
| tray i18n 构建期生成 | D-005⑧：下次 i18n 需求触发 |
| i18n 收缩 | D-006#3：维持 18 语言；locale 分布触发 |
| 竞品对标基准 | D-004：fork 线立项时的证据手段 |
| Mode B 数据面性能预算 | D-004：Resin 领域，壳层不设 |
| SSRF guard 统一 | 账本对账闸 #4：threat model 不同，暂不动 |
| R9 D-006 观察哨 / R9 D-008 导入原子性 | 前轮 deferred，触发未命中，维持 |

## Suggested skills

| Ticket | Skills |
|---|---|
| R11-02 / R11-10 / R11-11 | implement(tdd) / code-review |
| R11-03 / R11-04 | to-tickets / implement(tdd) / code-review |
| R11-05 / R11-06 | to-spec / to-tickets / domain-modeling / implement(tdd) / code-review |
| R11-07 | to-tickets / implement(tdd) |
| R11-08 / R11-09 | implement(tdd) / code-review |
| R11-12 | handoff / atomcode-research（先 ctx_search 存档再开新调研） |
| 通用 | but（版本控制唯一面）/ ctx_*（工具路由，文件编辑走 ctx）/ codegraph（探索先查）/ i18n-check（涉 UI 文案必跑）/ ADR-0072（CI-only，禁本机构建） |

## 数据源引用

- 账本：`.scratch/round11-grill/decision-ledger.md`（D-001..D-007 current）
- 本轮 ADR：`docs/adr/0071-headless-parity-policy.md`、`docs/adr/0072-ci-only-build-transition.md`
- 前轮归档：`.scratch/architecture-recovery-closed-2026-09-18/`（Round 10，A-001..A-012 已结算）、`.scratch/round9-grill/decision-ledger.md`（R9 D-001..D-010）
- 上游研究：`docs/research/RESIN_ROUTING_ARCHITECTURE_RESEARCH.md`（Resin 无 per-node API 证据）

---

**生成时间**: 2026-09-19 Round 11 grill 整理阶段
**对账闸**: 通过（无去向记录清单为空；D-007 为账本外结论经用户拍板后补记）
