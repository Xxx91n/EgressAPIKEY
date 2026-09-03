# Round 5 — config-authority 范围调研（不动手执行，仅立票 + 调研）

## 1. 范围定位

承接 Round 4（8 票闭环落地 origin/main = `d700be8`）。本轮用户拍板：
- **范围**：全部立票合并一轮（订阅 bug 4 票 + 心智模型 8 票 + 上游 Resin 文档化 10 票 = 22 票，按依赖图分 4 波并行）
- **深度**：atomcode-research 联网深调研 + 仓库静态调研都要

**我的本轮身份**：大脑 Agent 宏观调查者，**不动手修改任何仓库源文件**——只产出 `.scratch/round5-config-authority/` 工作目录（gitignored，仿 Round 4 模板）+ 三次 atomcode 联网调研落 OS temp。

## 2. 三次 atomcode 联网深调研（Round 4 先例：node spawn 直连 atomcode.exe，输出落 OS temp 不回灌主上下文）

| 调研 | 主题 | 触发句 | 落盘 |
|---|---|---|---|
| **R-A 订阅串联心智模型** | 工业级「订阅→平台→路由→端口」全链如何串起（GitOps / Crossplane / Cilium / 阿里云 ACK / ArgoCD ApplicationSet / Terraform Module） | "工业级 SaaS 风格平台的『数据源订阅 → 平台绑定 → 路由匹配 → 端口暴露』心智模型调研。重点看 GitOps 系（ArgoCD/Terraform）、Crossplane Composition、Cilium NetworkPolicy 等价物、K8s CRD+Controller 范式。给出本项目架构最适配的方案" | `C:\Users\Administrator\AppData\Local\Temp\round5-atomcode-A-subscription-pipeline.txt` |
| **R-B 配置权威 + observedGeneration 闭环** | "权威写 → 派生 → 回显 → 收敛"完整闭环的工业对照（K8s controllers / ArgoCD sync / Terraform state drift / Crossplane status / AWS Config / Nacos / Apollo） | "桌面级（单进程）配置权威 + observedGeneration 闭环心智模型调研。重点：K8s spec/status/observedGeneration、Terraform drift detection、ArgoCD Sync、AWS Config 历史。找出『桌面单进程 + 单文件白盒 + 单 sidecar』最简实现路径" | `C:\Users\Administrator\AppData\Local\Temp\round5-atomcode-B-config-authority.txt` |
| **R-C 写审计日志 + 回滚溯源** | 桌面级 / 单用户场景的"写审计 + before/after hash + append-only JSONL"心智模型（Kubernetes audit / AWS CloudTrail / SQLite WAL / GitButler / time-travel debug） | "桌面单用户级『写审计日志』心智模型调研。重点：append-only、before/after hash、防篡改、回滚溯源。最少开销 + 最强调试价值方案" | `C:\Users\Administrator\AppData\Local\Temp\round5-atomcode-C-write-audit.txt` |

**串行护栏**：每次只跑一个 atomcode 进程，结束后清理 OS temp runner `.cjs` + 输出 `.txt`，不杀 atomcode 进程但等其自然退出再开下一轮。

## 3. 22 票归一化（基于 3 份子代理报告）

### 3.1 订阅 bug 修复（W1 — 4 票，HIGH）

- **T01-subscription-add-platform-port-cascade**：subscription_add 后自动建 platform + 绑 port + 触发 apply（GUI 引导流 / 自动串联 / 弹窗选 port）—— 根因 1 + 2
- **T02-subscription-refresh-nodecount-closure**：refresh 后正确读最新 node_count + 修 NodesView 的 setNodes 闭包 bug —— 根因 3
- **T03-subscription-last-error-promotion**：把 `last_error` 升级到 toast 区域 / 订阅行 banner —— 根因 4
- **T04-subscription-post-retry**：POST `/subscriptions` 加重试（参照 send_read 的 5xx 规则）+ update_interval 默认 30s→5s —— 根因 4 兜底

### 3.2 心智模型统一（W2 — 8 票，HIGH/MED）

- **T05-diagpollinterval-typed-command**（HIGH）：给 `diagPollInterval` 加 typed command + `src/lib/settings.ts` 包装 —— 裂痕 #1
- **T06-backup-create-l3-private-exception**（MED）：ARCHITECTURE.md 「L3 不直接读私有文件」补 backup_create 例外 —— 裂痕 #2/#8
- **T07-config-export-import-whitebox-source**（HIGH）：config_export/import 改为读 / 写 `egressapikey-strategy.json`（白盒为真源），违反则显式标"派生导出"+ ADR —— 裂痕 #3
- **T08-account-add-bind-ip-deprecation**（MED）：account_add / account_bind_ip 标 deprecated + AGENTS.md 加 echo 命令清单 —— 裂痕 #4
- **T09-generation-observedgeneration-echo**（HIGH，票 37 四环 a）：白盒写计数 + apply 后 `applied_generation`/`last_applied_at`/`last_apply_error` + snapshot 回带 + UI 顶部常驻 chip —— 票 37 §4.a
- **T10-snapshot-conditions-axis**（MED，票 37 四环 c）：每实体 `conditions: Vec<Condition>` + port_health 与 snapshot 合并 + UI「Synced-but-Degraded」象限 —— 票 37 §4.c
- **T11-write-audit-log**（MED，票 37 四环 d）：app_data/audit.jsonl append-only + StrategyService/WhiteboxConfigStore 内嵌写一行 + UI Settings「Export audit log」按钮 —— 票 37 §4.d
- **T12-tray-drift-edge-notification**（MED，票 37 四环 b 上半）：fire_drift_notification 改跃迁边沿检测 —— 票 37 §4.b

### 3.3 上游 Resin 对接（W4 — 10 票，HIGH/MED/LOW）

- **T13-resin-api-coverage-doc**（HIGH）：`docs/architecture/RESIN_API_COVERAGE.md` 列上游 57 endpoints × 状态 + 引用 `server.go:65-157` —— 裂痕 2
- **T14-g4-webhook-misnomer-fix**（HIGH）：`HANDOFF_PATH_A*.md` / `architecture-state.md §16` / `DOCUMENT_GOVERNANCE_PLAN.md` 改"G4 = IPC retarget + sidecar-status 事件订阅" —— 裂痕 1
- **T15-system-config-decorative-command-cleanup**（HIGH）：要么 IPC body 真调 `client.system_config_*`，要么从 manifest + 注册表 + 文件删两条命令 —— 裂痕 3
- **T16-account-header-rules-integration**（HIGH）：补 4 个 client 方法 + IPC + TS wrapper + ADR，解释与 process_route_* 的关系（替代或共存）—— 裂痕 4、10
- **T17-resinclient-name-id-helpers**（MED）：新增 `resolve_platform_id_by_name` / `resolve_subscription_id_by_name`，替换 8 处重复 —— 裂痕 6
- **T18-resin-platform-schema-doc**（MED）：`docs/reference/RESIN_PLATFORM_SCHEMA.md` 字段 × IPC × 前端 form 四列对齐 —— 裂痕 8
- **T19-metrics-history-snapshots-realtime**（MED）：封装 12 metrics endpoints 至少 history/probes + realtime/throughput 两端点 + UI 画图 —— 裂痕 5
- **T20-resin-geoip-vs-thirdparty-doc**（MED）：ADR-0050 旁补 ADR 解释为何 ip_reputation_snapshot 走第三方而非 Resin GeoIP —— 裂痕 7
- **T21-request-log-detail-commands**（LOW）：expose get_request_log + get_request_log_payloads 给 UI diagnostics 抽屉 —— 裂痕 9
- **T22-platform-actions-classify**（LOW）：reset-to-default / rebuild-routable-view / preview-filter 三个 actions 归类（补 IPC vs 文档化"故意不接"）—— 裂痕 2 子项

## 4. 4 波次排期（基于依赖图）

| 波次 | 票号 | 阻塞边 | 预期落地形态 | 门禁 |
|---|---|---|---|---|
| **W1 — 订阅 bug** | T01 → T02 → T03 → T04（线性） | T01 阻塞 T04 的 retry hook；T02 独立；T03 独立 | GUI handleAdd 引导流 + 节点计数闭环 + 错误可见性 + POST 重试 | cargo / vitest / i18n / ipc-manifest / isolation-guard |
| **W2 — 心智模型裂痕** | T05 → T08 → T06 → T07（线性）| T05 独立；T06 独立（文档化）；T07 依赖 T08 拍板 account_* 去留 | L1 旁路清理 + L3 例外声明 + config_export 白盒化 + echo 命令 deprecate | 同上 + markdownlint 0 |
| **W3 — 票 37 四环落地**（独立栈，可与 W2 并行）| T09 → T12 → T10 → T11（线性）| T09 阻塞 T10 snapshot 字段扩展；T12 独立 | generation 回显 + drift 边沿 + sync×health + 写审计 | 同上 + 新增 audit.jsonl schema 校验 |
| **W4 — 上游对接**（独立栈）| T13 → T14 → T15 → T16 → T17 → T18 → T19 → T20 → T21 → T22 | T14 依赖 T13 完成总表（因为总表是误称修订依据）；T17 独立；T22 依赖 T13 总表 | Resin 全 endpoints 表 + G4 误称修订 + 装饰命令清理 + account-header-rules 集成 + name→id 工具 + schema 文档 + metrics 接入 + GeoIP ADR + request_log 详情 + actions 归类 | 同上 |

**栈关系**：W1 / W2 / W3 / W4 都是独立 GitButler 分支栈，可并行开发。落地按 W1 → W2 → W3 → W4 顺序 `but land`（无依赖边），但每个栈内严格按票号顺序合入。

## 5. 工作目录与产物

```
.scratch/round5-config-authority/
├── README.md                              # 4 波表 + 22 票状态表 + 调研摘要 + backlog
├── spec.md                                # 22 票总览 + 决策依据 + 3 次 atomcode 调研引用
├── reports/
│   ├── 01-subscription-pipeline-bug.md   # 子代理 1 报告（订阅链路）
│   ├── 02-config-authority-fractures.md  # 子代理 2 报告（三层心智模型）
│   └── 03-resin-upstream-coverage.md     # 子代理 3 报告（上游对接）
├── issues/
│   ├── 01-subscription-add-platform-port-cascade.md
│   ├── 02-subscription-refresh-nodecount-closure.md
│   ├── 03-subscription-last-error-promotion.md
│   ├── 04-subscription-post-retry.md
│   ├── 05-diagpollinterval-typed-command.md
│   ├── 06-backup-create-l3-private-exception.md
│   ├── 07-config-export-import-whitebox-source.md
│   ├── 08-account-add-bind-ip-deprecation.md
│   ├── 09-generation-observedgeneration-echo.md
│   ├── 10-snapshot-conditions-axis.md
│   ├── 11-write-audit-log.md
│   ├── 12-tray-drift-edge-notification.md
│   ├── 13-resin-api-coverage-doc.md
│   ├── 14-g4-webhook-misnomer-fix.md
│   ├── 15-system-config-decorative-command-cleanup.md
│   ├── 16-account-header-rules-integration.md
│   ├── 17-resinclient-name-id-helpers.md
│   ├── 18-resin-platform-schema-doc.md
│   ├── 19-metrics-history-snapshots-realtime.md
│   ├── 20-resin-geoip-vs-thirdparty-doc.md
│   ├── 21-request-log-detail-commands.md
│   └── 22-platform-actions-classify.md
├── handoffs/
│   └── 01..22-*.md                       # 每票完成定义（参照 Round 4 模板）
└── prompts/
    └── 01..22-*.md                       # ≤ 60 行 launcher
```

## 6. 不做的事（边界）

- ❌ 不修改任何仓库源文件（.rs / .ts / .json / .toml / .md / ADR）
- ❌ 不跑 verify-build（会改 dist/ 与 stage exe）
- ❌ 不 commit / branch / push（开窗执行阶段再做）
- ❌ 不调用 ctx_* 工具（本会话无；atomcode 用 node spawn 走 OS temp 替代）
- ❌ 不在主上下文灌入 atomcode 全文输出（每轮只摘 5-10 行关键结论写入 spec.md / README.md）

## 7. 完成定义（本 plan 自身的完成标准）

1. 3 次 atomcode 调研全部跑完，每轮 output 落 OS temp，runner `.cjs` + 输出 `.txt` 已清理
2. `.scratch/round5-config-authority/` 全目录 22 票已立齐（issue + handoff + prompt）
3. `README.md` 含 4 波表 + 22 票状态表 + 调研摘要 + 阻塞边说明
4. `spec.md` 含决策依据（明确引用 3 次 atomcode 调研路径 + 3 份子代理报告）
5. 大脑 Agent 呈报给用户：调研已完、立票已齐、波次已排、依赖已标，等用户拍板"是否开窗执行 W1"

## 8. 拍板与开窗

本 plan 通过后，我立即按顺序执行：
1. 跑 atomcode 调研 R-A → 写 spec.md 初稿
2. 跑 atomcode 调研 R-B → 更新 spec.md
3. 跑 atomcode 调研 R-C → 更新 spec.md
4. 写 `.scratch/round5-config-authority/` 全目录 22 票
5. 呈报给用户，等用户说"开窗执行 W1"或调整范围

**开窗执行 W1 不在本 plan 内**——另立 plan 或用户口头确认后再开。