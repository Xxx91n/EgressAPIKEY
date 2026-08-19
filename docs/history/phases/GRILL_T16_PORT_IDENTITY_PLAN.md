# GRILL T16 - Port-Based Identity 复核收尾（基于真实 tree 修订）

> 原 7 Phase 路线图经 ctx 代码级核实后修订：T16-1/4/5 已在历史 commit 落地；
> T16-2 route_id/normalize_auth 已被 commit c71bab5 P1 主动删除；
> T16-3 白盒配置闭环已被 T15 ADR-0036 strategyConfig 管道完成。本计划收尾 = 文档校正 + ADR-0037 + 门禁 push。

## 前序（已验证事实）

- T15 commit 8dac8e6: 7 Phase + 219 vitest + tsc + i18n(327 keys) + cargo(119 pass) + exe stage + push
- ADR-0036 strategyConfig 统一管道已落地 (strategy_config_get/put/apply Rust IPC + ipcStrategyConfigGet/Put TS)
- ADR-0037 决策记录：port-based identity 已全面落地，route_id/interceptor 已被 P1 commit c71bab5 主动删除并不复接回

## T16 原 7 Phase 实际状态

| Phase | 原计划 | 实际状态 | 证据 |
|---|---|---|---|
| T16-1 | TopologyView A 列从单一节点改为多端口节点 | ✅ 已落地 | TopologyView.tsx L572-579 ports.forEach + 测试 L236 "ADR-0012: A->B edge connects port to platform" |
| T16-2 | route_id helper 接入 IPC 入口 | ❌ 不复接回 | commit c71bab5 P1 主动删除（"Delete interceptor.rs, a4_3_live.rs, route_id/normalize_auth, InterceptorPort"），port-based identity 取代三个组身份模型 |
| T16-3 | 白盒配置全程闭环验证 | ✅ 已落地 | T15 ADR-0036 strategyConfigPut -> strategyApply 管道 + TopologyView onConnect/onEdgesDelete 走该管道；T15-5 测试覆盖 |
| T16-4 | PlatformsView 左 pane 重构为 Entry Ports | ✅ 已落地 | PlatformsView.tsx L390 t("platform.entryPorts") + L429 label + platform_name/unbound 显示，KeyCandidates 概念已删除 |
| T16-5 | ProcessRouteView process->port | ✅ 已落地 | ProcessRouteView.tsx L30 r.target_port + L50 processRoute.conflict { port: tgt } + L84 processRoute.targetPort；Rust process_route_add L407-425 target_port 字段 + process_route_conflict_check 基于 port |
| T16-6 | ADR-0037 + AGENTS.md | ✅ 本计划收尾 | ADR-0037 写入 + AGENTS.md §28/§30 Reverted 块 |
| T16-7 | 全量门禁 + exe stage + push | ✅ 本计划收尾 | 纯文档变更跑 pnpm test/i18n:check/tsc 不跑 cargo build --release |

## 收尾执行计划

### Phase T16-Final-1: ADR-0037 ✅ 完成
- docs/adr/0037-port-based-identity-reverts-route-id.md (2047 bytes, UTF-8 LF no BOM)
- 决策：port-based identity 已全面落地，route_id/interceptor 不复接回

### Phase T16-Final-2: AGENTS.md §28/§30 文档失步校正 ✅ 完成
- §28 (L351): Reverted by P1 (commit c71bab5) 块前置，注明 route_id/normalize_auth 已删除
- §30 (L365): Reverted by P1 (commit c71bab5) 块前置，注明 interceptor.rs 整个模块已删除
- 字符纯净：BEL=0 CR=0 LF=668，无字面 \r/\n
- §541 那个 §30 (P5+P6 route-correction final architecture) 是正确描述，未动

### Phase T16-Final-3: 全量门禁
- pnpm test (vitest) — 219 pass / 16 files green
- pnpm i18n:check — 327 keys / 18 locales green
- pnpm exec tsc --noEmit — 0 errors
- 纯文档变更不必 cargo build --release（不改任何 .rs source）

### Phase T16-Final-4: commit push
- 分阶段提交或合并提交（ADR-0037 + AGENTS.md + PLAN.md）
- codegraph sync . 重索引（.md 变更也跑避免 codegraph 漂移）
- git push 到 codex/rust-port

## 验收标准

1. ADR-0037 文件存在且 UTF-8 LF no BOM ✅
2. AGENTS.md §28/§30 Reverted 块文案干净无 BEL/backslash 字符 ✅
3. GRILL_T16_PORT_IDENTITY_PLAN.md 状态表与真实 tree 对齐 ✅
4. 219 vitest + tsc + i18n:check 仍 green
5. commit + push 完成

## 依赖

- T15 ADR-0036 strategyConfig 管道（已落地）
- T15 commit 8dac8e6（基线）
- P1 commit c71bab5 死代码删除决策（已发生）

## 联网调研方向

无新增——本计划是事实校正收尾，不引入新功能。如有未来 agent 想复活 route_id/interceptor，应以 ADR-0037 + 本计划状态表为依据否决。