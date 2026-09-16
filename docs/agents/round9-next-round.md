<!--
  Source-of-truth path: .scratch/round9-grill/handoffs/next-round.md (session artifact)
  This docs/agents/ copy is the persistent, git-tracked reference.
  Generated: Round 9 grill closeout (D-001..D-010 settled; 11 sharp-critique friction points adjudicated).
  Regenerate this file from the session artifact after any ledger update.
-->

# Round 9 Next-Round Task Book

> **数据源纪律**：本任务书唯一数据源是 `.scratch/round9-grill/decision-ledger.md`（D-001..D-010 current），不从对话回忆补充任何结论。
> **grill 终裁**：锐评全部 11 项摩擦点（P0×4 + P1×6 + P2×1）已决策。

---

## 任务清单（按立票优先级排序）

### Ticket R9-01: ADR-0068 D4 四场景 + t17 合同验证挂入 CI（小票）
- **覆盖决策**: D-001
- **范围**: ADR-0068 D4 四场景（HTTP 503 NO_AVAILABLE_NODES / SOCKS5 05-00 成功 / SOCKS5 05-ff 拒绝 / HTTP 403 ENDPOINT_CAPABILITY_DISABLED）+ ticket 17 合同验证（repro/t17-contract）挂入 verify-build.sh 或 CI release checklist
- **工作量**: 半天
- **风险**: 低（挂入既有门禁）
- **依赖**: 无

### Ticket R9-02: EffectiveConfigView 补显 Live policy（小票）
- **覆盖决策**: D-002
- **范围**: `src/views/EffectiveConfigView.tsx` Consistent 行渲染 `resin_allocation_policy` + 18 locale 各 1-2 key + vitest 断言
- **工作量**: 半天
- **风险**: 低（纯渲染 + i18n）
- **依赖**: 无

### Ticket R9-03: Headless guard 接线恢复 + 回归锁 + CI 补洞（P0）
- **覆盖决策**: D-003
- **范围**: 
  1. 从 `git show f18f365b:src-tauri/src/headless_main.rs` 取回 146 行（import + CLI auth_token/allowed_host + resolve_token 调用 + HeadlessGuard::new + .layer + security_guard 中间件）
  2. 新集成测试断言 build_router 对无 token /api/v1/platforms 返回 401/403、对坏 Host 返回 403
  3. verify-build.sh push 路径加 `cargo check -p egressapikey-app --features headless`
  4. P0③ 残余 runtime 复测（GET query 透传 + body 双包体）
- **工作量**: 1-2 天
- **风险**: 低（恢复已审阅落板代码；primitives + 14 单测幸存；`git log f682940e..HEAD` 为空可直接 cherry-pick）
- **依赖**: 无

### Ticket R9-04: reset_kernel 接现成 restart_resin（小票）
- **覆盖决策**: D-004
- **范围**: `src-tauri/src/commands/settings.rs` sidecar_restart 改为调用 `egressapikey_app::sidecar::restart_resin(app)` via spawn_blocking（启用当前弃用的 _app 参数）
- **工作量**: 1-2 小时（~20 行）
- **风险**: 极低（复用 sidecar.rs:532-586 已测路径 + round8-07 4 单测）
- **依赖**: 无

### Ticket R9-05: subscription_remove 补清理 + ensure_default_port 补偿（P1）
- **覆盖决策**: D-007
- **范围**: 
 1. subscription_remove 补清理逻辑（remove_default_port_if_orphaned，port_list 过滤 platform_id 匹配 → port_remove 正常路径）
 2. ensure_default_port 补偿检查（同 platform 已有 default port 跳过创建，幂等）
 3. 单测 + 集成测试断言孤儿不留存
- **工作量**: 1-2 天含测试
- **风险**: 低（Mode A 已无孤儿面 ticket 17，修复仅针对 Mode B 残留）
- **依赖**: 无

### Ticket R9-06: 工程边角 5 项批量修复（P2）
- **覆盖决策**: D-010
- **范围**:
 1. **#1+#2 必修**（1h）: verify-build.sh L9-11 非 Windows CI 分支追加 2 行 cargo check headless+custom-protocol
 2. **#3 必修**（10min）: 改 `02-headless-adapter-report.md:193` "43 条"→"44 个"
 3. **#7 必修**（30min）: verify-build.sh 加 codegraph status 或 AGENTS.md 降格 "P23 硬闭环"→"建议 codegraph sync"
 4. **#5 可选**（半天）: readme-lang-check.cjs 加第 18 check（EN/CN code fence 语言标识镜像）
 5. **#8 可选**（半天-1天）: vitest-isolation-guard.cjs 扩全部 `src/views/*.test.tsx` + 泛化 pattern
- **工作量**: 约 1 天（必修 2h + 可选 0.5-1 天）
- **风险**: 低（必修项改动小，可选项与 R9-03 #3 CI 补洞有部分重叠可合并）
- **依赖**: 无

---

## 范围外记录（无需立票）

| 决策 | 范围外理由 |
|------|-----------|
| **D-005**（三层一致性）| 维持现状，ADR-0069 D1 已立案（L2-first + level-triggered reconcile）|
| **D-006**（幂等 vs 强制）| 推迟 + 观察哨，等实证案例再立票 |
| **D-008**（配置导入原子性）| 推迟，validate-both + 自动备份 + 双 rollback 兜底已足 |
| **D-009**（WebDAV 恢复）| 已完成，ADR-0070 闭环齐全 |

---

## 总工作量预估

- **R9-01**（小票）: 半天
- **R9-02**（小票）: 半天
- **R9-03**（P0）: 1-2 天
- **R9-04**（小票）: 1-2 小时
- **R9-05**（P1）: 1-2 天
- **R9-06**（P2）: 约 1 天
- **合计**: 约 4-5 天

---

## Suggested skills（按票优先级）

| Ticket | Suggested Skills |
|--------|-----------------|
| R9-01 | to-spec / to-tickets / implement（含 tdd）/ code-review |
| R9-02 | to-tickets / implement（含 tdd）/ i18n-check guard |
| R9-03 | diagnosing-bugs / implement（含 tdd）/ code-review / but cherry-pick |
| R9-04 | implement（含 tdd）/ code-review |
| R9-05 | to-spec / to-tickets / implement（含 tdd）/ code-review |
| R9-06 | to-spec / implement（含 tdd）/ code-review |

通用 skill：
- **but**：版本控制唯一面（commit / branch / cherry-pick / verify-build 闭环）
- **codegraph**：探索前先 codegraph_explore（避免重复 Read）
- **ctx_***：文件编辑/分析/抓取全程 node.js 写文件
- **atomcode-research**：新调研先 ctx_search 既有存档再开

---

## 数据源引用

- 账本：`.scratch/round9-grill/decision-ledger.md`（D-001..D-010 current）
- 锐评：`.codex-tmp/rui.txt`（P0×4 + P1×6 + P2×1 = 11 项摩擦点）
- 上轮归档：`.scratch/architecture-recovery-closed-2026-09-16/`（Round 8 归档，含 A-001..021 决策）
- 上轮 handoff：`handoff-round8-closed-2026-09-16`（ctx 知识库）

---

**生成时间**: Round 9 grill 整理阶段（用户消息："继续"）
**数据源**: decision-ledger.md（10 条 current，无 revised）
**对账闸**: 通过（无去向清单为空）
