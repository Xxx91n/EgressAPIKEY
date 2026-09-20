<!--
  Source-of-truth path: .scratch/r11-wave-b-grill/handoffs/next-round.md (session artifact, gitignored by .gitignore:100)
  This docs/agents/ copy is the persistent, git-tracked reference.
  Generated: r11-wave-b grill closeout (D-001/D-002 settled; Wave A completion audit in ledger appendix A).
  Regenerate this file from the session artifact after any ledger update.
-->
# Round 11 Wave B — Resident Task Book（r11-wave-b grill 收口）

<!--
  Source-of-truth ledger: .scratch/r11-wave-b-grill/decision-ledger.md (D-001..D-002 current, 2 entries)
  Git-tracked mirror: docs/agents/round11-wave-b-next-round.md
  Parent ticket registry: docs/agents/round11-next-round.md (R11-01..13; R11-14 defined HERE)
  Generated: 2026-09-20 wave B grill closeout (post-Wave-A execution round)
  Regenerate this file from the session artifact after any ledger update.
-->

## 对账闸（2026-09-20，通过）

| 记录 | 去向 |
|---|---|
| D-001 | 本任务书执行令（wave B = R11-03 P0 + R11-05 + R11-08 + R11-09 + R11-14 新票）；wave C/D 顺延维持 |
| D-002① | R11-14 票内执行（WHY-NOT-EFFECTIVE troubleshooting 行 + 观察哨，不改代码） |
| D-002② | ADR-0071「已知取舍」句——已于整理阶段落入 docs/adr/0071-headless-parity-policy.md |
| D-002③ | round11 任务书范围外表 ⑤c 注记——已于整理阶段落入（.scratch 与 docs/agents 两份拷贝同步） |

无去向记录清单：**空**。

## 执行令（D-001）

Wave B 照单全收，R11-03 为 P0 主票。票据规格一律按引用读 docs/agents/round11-next-round.md 对应章节，此处不复制：

| 票 | 规格引用 | 覆盖决策 |
|---|---|---|
| R11-03（P0） | round11 任务书 §R11-03：/api/v1/ports/{port} 字面路径修复、路由补齐 SPA 可达对等、GET /api/v1/capabilities、DISABLED_COMMANDS 重裁、X-Forwarded-Proto + CSP frame-ancestors、HTTP 冒烟入 CI verify | round11-grill D-003 + 本轮 D-001 |
| R11-05 | round11 任务书 §R11-05：策略诚实化 + 模板（权威写入口预置快照）+ 区域级近似 + i18n ×18 | round11-grill D-002(A/C) + D-001 |
| R11-08 | round11 任务书 §R11-08：WAL + BEGIN IMMEDIATE + busy_timeout ≥5s；验收 p99 ≤50ms | round11-grill D-004/D-005② + D-001 |
| R11-09 | round11 任务书 §R11-09：SSE 双指标断言 + 延迟 p99 ≤10ms + 500 并发内存有界 | round11-grill D-004 + D-001 |
| R11-14（新） | 见下节 | 本轮 D-001 + D-002① |

依赖：R11-04 已 landed（src/lib/headless-routes.ts 129 / headless-availability.ts 168 / headless-dispatch.ts 104 就位），R11-03 可直接开工。

## R11-14（新票）：CI 门 + 卫生

- **覆盖决策**: 本轮 D-001（审计移交项）+ D-002①
- **硬交付（CI 门，verify job / verify-build.sh 内）**:
  1. lockfile-diff 门：Cargo.lock 与 Cargo.toml 失同步即红（F1 复发防护；`cargo metadata --locked` 或等价只读 lockfile 检查，不产生构建产物）
  2. `cargo fmt --check` 门（fmt 残留持续流出问题）
- **随触碰卫生清单（本轮触碰面内顺手清，不专程扫全库）**:
  - vacuous check_port_available 测试（wave-a audit §5）
  - 死 ALLOCATION_POLICIES 导出（同上）
  - `(ADR-0016 )` 空括号注释（同上）
  - 重复目录解析块（同上）
  - isTauri 诊断探针常驻生产 bundle（src/lib/headless-availability.ts:39；R11-03 触碰该文件时 dev-only 化或删除）
  - src-tauri/src/headless_main.rs:633 url_encode_segment 随 R11-03 触碰评估换 crate（resin_client.rs:789/812 两个编码器语义有意不同且各带测试——先读测试再动，语义不合并）
- **文档顺手项（D-002①，不改代码）**: docs/how-to/WHY-NOT-EFFECTIVE.md troubleshooting 表补一行——watch_apply 回滚窗口：文件已落盘而 apply 失败时内存回滚、磁盘保留新内容；authoritative_snapshot 显示 Drifted、reconcile_now 可自愈；**勿「加固」成磁盘回滚——会吞用户手改的 L2 权威内容**
- **不触碰**: TopologyView.tsx（D-002③ 归 ⑤c 触发器）；Resin API 边界（D-001 约束）
- **工作量**: 半天-1 天；**风险**: 低

## 本轮不做（顺延/范围外，均维持既有决策，零推翻）

- Wave C/D：R11-06（编排 controller）、R11-07（key→账号映射）、R11-12（部署文档+基线实测）、R11-13（轮末评估）——wave B 完成后按 round11 任务书波次推进
- fork 线、应用层前置网关、桌面远程客户端、API-first、strategy_service 全分解、React Query 迁移、tray i18n、i18n 收缩、竞品对标、Mode B 预算、SSRF 统一、R9 D-006/D-008——理由表见 round11 任务书「范围外记录」

## Wave A 后现状基线

锐评完成度审计结论（细节与行号证据见账本附录 A）：已完成 11 项（gen_token 熵源 / TOCTOU / b64 / hex / civil_from_days / items_arr / Txx 注释 / 根目录清理 / AGENTS.md 仲裁 / ipc.ts 拆分 / 计数修正），驳回 1 项（PBKDF2 已 spawn_blocking 卸载），其余剩余项全部已映射到本任务书票号。上下文恢复链：.scratch/r11-wave-a/handoffs/wave-a-audit-handoff.md → 本账本附录 A → docs/agents/round11-next-round.md。

## 执行纪律

- but 唯一版本控制面；每票独立 lane 分支；push 触发 CI verify（ADR-0072：交付证据＝CI 产物，本机零构建）；land 前 CI 绿，but land --whole-stack（wave-a 先例）；轮末合并后停住汇报。
- 文件编辑走 ctx（node fs）；CodeGraph 探索先查、改后 codegraph sync .；i18n 键变更 ×18 同 commit（pnpm i18n:check）。
- 每票收尾 code-review；R11-03 路由语义开工前可先 to-tickets 细化。

## Suggested skills

| 阶段 | Skills |
|---|---|
| 开工 | but / codegraph（探索） |
| 每票 | implement(tdd) / code-review；R11-03 可先 to-tickets |
| R11-05 | domain-modeling（策略模板术语如需精化）+ i18n-check |
| 收尾 | handoff（生成 wave C 恢复上下文） |
| 通用 | ctx_*（工具路由，文件编辑走 ctx）/ ADR-0072（CI-only 禁本机构建）/ but |

## 数据源引用

- 账本：.scratch/r11-wave-b-grill/decision-ledger.md（D-001..D-002 current + 附录 A 审计快照）
- 票据注册表：docs/agents/round11-next-round.md（R11-01..13）
- Wave A 审计：.scratch/r11-wave-a/reports/2026-09-19-audit.md + .scratch/r11-wave-a/handoffs/wave-a-audit-handoff.md
- ADR：docs/adr/0071-headless-parity-policy.md（本轮补已知取舍句）、docs/adr/0072-ci-only-build-transition.md
