# R12 Wave-C — 下一轮任务书（r12-wave-c-grill 定稿 2026-09-23）

> 数据源（唯一）：`.scratch/r12-wave-c-grill/decision-ledger.md`（D-001/D-002 全 current）。
> 正本：`.scratch/r12-wave-c-grill/handoffs/next-round.md`；本文件为 git 镜像。
> 前置已结：wave-b 五 lane 全 land 上 origin/main（tip 9d3b5ce7），合并态 ci/webview-smoke/Docs Governance 三绿；远端仅 main；无遗留票。

## 0. 轮次形态（D-001）

**证据轮**。W3「environment_suspect 抑制器真实运行证据」升格主票 R12-C1；卫生批 R12-C2 随车；W4 发版 0.2.0 在证据落地后切（R12-C3，2 工作日解耦条款）。W2 self-hosted runner 继续既有 fallback 计时（2026-09-22+4wk→2026-10-20 逾期→本机钉临时代表数并标注非目标硬件）；W8 产品方向裁决推迟至 wave-a D-003⑨ 再过滤条款执行时（C2 † 回填或 fallback 钉数后重排优先级）。

## 1. 票表

### R12-C1 — fault-inject 证据票（P0；覆盖 D-001+D-002）

**目标**：给 ADR-0080 共模抑制器拿到「可信非单测」运行证据（工业惯例：防护机制上线≠可信，须有真实故障注入证据）。

规格（D-002 八格 + audit 选项 A）：

1. **载体**：`run-bench.mjs` 新增 `faultinject` 相位；`--phases faultinject` 按需启；不进默认相位集、不进 verify 门；bench.yml 经既有 phases dispatch 输入可选携带。
2. **SUT**：headless exe（POST /api/v1/shell/orchestration/tick 手动驱动 tick，不等 60s cadence；抑制器在 resin-core 共享层双传输同码）。fixture 显式 `autonomy=suggest`——把 Auto 迁移隔离出断言面。
3. **fixture**：spawn 前预置全新 stateRoot——`egressapikey.db` 端口行（5 平台 × 1 mixed enabled 端口；探针枚举源是 db `list_ports()` 非 whitebox json）+ `egressapikey-strategy.json`（orchestration enabled + platforms a_class=Region）；每 run 全新 stateDir；直写文件绕 cascade 合法；REST PUT 仅运行时调整。
4. **场景序列**：baseline ok（锚，失败重试一次）→ kill forwarder + 3 手动 tick（注入窗 <60s，避开 60s driver 插入）→ suspect 断言 → poll-wait 恢复（Resin 节点重启须过 cloudflare 健康探针才 routable）→ kill mock node + tick → remote_fail 断言 → 恢复 → clean tick → streak 归 0。
5. **断言面**：tick 返回 `platform_verdicts` + `environment_status.suspect_streak`；streak 断言=单调不减 + crossing==3 恰一行环境事件 + clean 归 0（不写死 1→2→3——60s driver 可能插同判 tick）。suspect 段锁：平台不迁移、verdict 盖 environment_suspect；remote_fail 段锁：全平台 remote_fail、loopback ok、streak 不动、无环境事件。
6. **audit（选项 A）**：同票补 `headless_main` 一行 `resin_core::audit::init(state_root.join(AUDIT_LOG_FILE))`——headless 未 init audit sink 系真实缺口补齐（与 headless 同 L2 存储叙事一致）。文件断言=streak==3 时 `audit.jsonl` 恰一行 `signal-plane/environment_suspect/orchestration:tick`（streak 4/5 不再发）。
7. **注入载体重述（票内记 provenance 行）**：D-001 ①停 mock upstream→kill mock node（egress 探针硬编码 `1.1.1.1/cdn-cgi/trace` 经 entry port→sidecar→mock node→真网，mock upstream 不在探针路径）；②bind-refuse→kill forwarder（Mode A 壳侧 entry 失效同一被测类，探针行为等价）。kill sidecar 原案码上不成立（loopback+egress 同死→join 产 local_fail 非 remote_fail）。同一 fail 类映射，非 revised。
8. **降级与边界**：forwarder 缺失→相位记 skipped（复用 modeA skipped 语义）不 fail；faultinject 需真网出口（1.1.1.1）——mock.mjs「zero internet」头注释对本相位失效，票面+PERF-BENCH 写明；CI 网络抖动=固有 flake 源（缓解：slow_call_ms 调大+baseline 重试一次）；Toxiproxy 留注释不进依赖；合成 local_fail 只证 loopback 全拒形态，WAN 半残灰区归 ADR-0080 豁免+soak 兜底不重复立法。
9. **纪律**：无新 IPC 命令；选项 A 动 src-tauri 一行→CI verify 覆盖（ADR-0072）；n=5 平台恰卡最低非平凡 n（⌈0.8·5⌉=4）；断言不锁迁移终态。

验收：bench.yml dispatch `--phases faultinject` 双腿（windows+ubuntu）绿；注入证据行写入 release notes 素材。

### R12-C2 — 卫生批（覆盖 D-001）

- W5 审计容忍级微疵：run-bench.mjs phaseHeadless/phaseApp 相位重复 ~90 行参数化、waitTcpReady/killTree 与既有内联实现去重、PlatformsView 两处 disjunct/命名拷贝。**排序条款：C1 先落、C2 后扫**——同触 run-bench.mjs 避免 rebase 撞车。
- W7 cosmetic：`git fetch --prune` 清本地 stale remote-tracking refs（b1/b2/b4/docs-consolidation 残留）。
- W1 登记册 webview-smoke→resolved：已在 wave-c 整理 commit 完成（合并态首跑 35755345313 双腿绿正面兑现）；本票只复核无遗漏。

### R12-C3 — 发版 0.2.0（覆盖 D-001）

时机：R12-C1 证据落地后切（release notes 带「抑制器已获注入证据」行）；若 C1 超 2 工作日未落则先切、notes 如实标注 †暂定项与抑制器证据状态——不互相绑架。
内容：版本位 bump（按既有发版流程对齐 Cargo.toml workspace / package.json / tauri.conf.json）、CHANGELOG [Unreleased]→[0.2.0] 收编、RELEASE_NOTES.md 更新（如实标注 † provisional + 抑制器证据状态）、tag + CI release matrix dispatch（workflow_dispatch）、产物 chunk-hash 核验。

## 2. 程序条款

- W2：self-hosted runner 注册仍用户侧动作；fallback 计时 2026-09-22+4wk→2026-10-20 逾期本机钉临时代表数并标注非目标硬件。
- W8：产品方向裁决推迟至 C2 † 回填或 fallback 钉数后的再过滤（wave-a D-003⑨）。
- W6：strategy_service 3636/4000 预占线 armed，不主动拆。
- 若实现中产生新领域词：随实现 commit 入 CONTEXT.md（wave-b D-002⑦ 先例）。
- AGENTS.md 12B 余量 armed：本轮任务书入 docs/agents/，不触 AGENTS.md。

## 3. 操作项（非票）

- 登记册巡检义务照旧（wave-a D-003⑨）：每轮 grill 复查各行 probe 列。

## suggested skills

| 任务 | skills |
|---|---|
| R12-C1 实现 | implement + tdd（断言面先行）；codegraph 取证已入票（探针枚举源/join/suppressor 行号） |
| R12-C2 | 卫生批直接 implement；同触 run-bench.mjs 注意 C1 先落 |
| R12-C3 | CI-only 纪律（ADR-0072）+ build-all 管道顺序参考；release matrix dispatch |
| 版本控制 | gitbutler（每票一 lane，C1 先落避免 C2 rebase） |
| 下轮 grill | grill-with-docs + 本账本 + trigger-line-register |
