# HANDOFF - Polish Phase (mainline A closure)

> Polish phase handoff prompt for the next Codex session. After reading this document the next agent can start polishing; no further grill questions required.

---

## 启动提示词 (copy-paste to next Codex session as the first user prompt)

遵循以上 grill 构造的所有内容，设定目标，开始大型构建，完成 grill 路线图后续安排的所有内容。
始终遵循 AGENTS.md，加载并使用 ctx_* 插件，必要时用 1mcp 的 exa/perplexity 联网搜索，不要产生幻觉推理。Ponytail full 模式。

[$(ponytail)](C:\Users\Administrator\.codex\plugins\cache\ponytail\ponytail\4.8.4\skills\ponytail\SKILL.md)

[$(ultragoal)](C:\Users\Administrator\.codex\skills\ultragoal\SKILL.md) 设定目标以防止丢失

以下功能我都我希望你能联网调研出一个类似功能的模板轮子出来，而不是自己开发

遵循以上 grill 构造的所有内容，设定目标，开始大型构建

---

## 必读上下文 (按顺序)

1. `docs/GRILL_ISSUES_BACKLOG.md` 顶部 **PLAN SCHEDULE** 表 - 12 项 confirmed, serial 单 lane, 一项一 commit.
2. `docs/adr/0006-mainline-a-then-b.md` - mainline A (功能闭环) 先于 B (发布).
3. `docs/adr/0011-observed-key-pool-sqlite-b-b-3-path.md` - 第一个 SQLite 实例落地方案: r2d2 池 + 手写 PRAGMA user_version + 复用 P14 backup 流程 + WAL mode.
4. `docs/MEMORY_REUSE_DECISION.md` - 项目 fork Resin 的原始决策依据.
5. `docs/REFACTOR_PLAN.md` - R1/R2 后端 IPC 与 Topology canvas 三列设计.
6. `docs/KEY_ENDPOINT_FORMAT_RESEARCH.md` + `docs/RESIN_ROUTING_ARCHITECTURE_RESEARCH.md` - A3/A4 源码级研究表明 route_id three-tuple 与 X-Resin-Account 注入机制.
7. `docs/PROTOCOL_WEIGHT_RESEARCH.md` - 节点协议对 AI API SSE/WebSocket 的影响权重研究.
8. `README.md` / `README_CN.md` - 现有项目外宣文档 (后续需与代码同步, 但 mainline B 不发布).
9. `CONTEXT.md` + `docs/glossary.md` - 领域术语 (Observed Key Pool / DbPool / route_id / X-Resin-Account / mainline A/B 等).
10. AGENTS.md (Repo 根) - 项目唯一守则文件, 每完成一个 item 立刻按 §5 闭环重 build + stage release exe + §10 追写新节.

## 执行规则 (A13 = (3)(i) 小步快跑)

每完成 PLAN SCHEDULE 表中一个 item:
1. **实现代码** - 写源码 (Rust/TS), 修改 i18n locale (18 base locales 同步, `pnpm i18n:check` 必须绿), 改 Cargo.toml 如有新 dep.
2. **闭环测试** - cargo unit test + mockito (若需) + vitest (前端组件). 没有闭环测试的 item = 玩具, 用户原话不接受.
3. **Type check** - `pnpm exec tsc --noEmit` 绿.
4. **Build** - `cargo build -p ai-api-route-app --features custom-protocol` 绿; `cargo build --release` 产出新 exe.
5. **Stage release** - 把 `target/x86_64-pc-windows-gnu/release/ai-api-route.exe` 复制到 `release/windows-gui/ai-api-route.exe`, 再复制 `resin.exe` sidecar.
6. **验证嵌入的 bundle 是新版** - 抓 `dist/assets/*.js` 的最新 Vite chunk hash, 在 exe bytes 里 ASCII grep 验证 (AGENTS §5 hard close-loop).
7. **Smoke launch** - `Start-Process release/windows-gui/ai-api-route.exe`, 用 `MainWindowHandle != 0` + `MainWindowTitle == "ai-api-route"` + `WorkingSet ~20-40MB` + stdout `tracing initialized` + no panic 三验.
8. **CodeGraph sync** - `codegraph sync .` 增量更新索引 (AGENTS §1).
9. **AGENTS.md 追写** - 与本次 item 相关的 § (如 §Storage locations for C1-1), 格式见现有 §22 / §23 / §24 等.
10. **Commit + 本地 push** - commit `<Item-ID>: <brief>` 约定; 本地 push (远端 token 已失效, 按用户多次确认只本地提交即可, 保持可回溯).
11. **目标完成一个 item 后立刻进下一个, 不要批** - 小步快跑.

## Constraints (AGENTS §8 + grill user)

- **Single-threaded** - do NOT invoke Codex native subagent, OMX multi-agent runtime, or worker lanes. All work stays in the current agent.
- **ctx_* first** - file edits via `ctx_execute` with Node `fs.writeFileSync` (`cwd: "D:/Aworker/ai-api-route"`); this dodges the ctx_execute_file workspace root drift bug. read-to-analyze goes through ctx_batch_execute. shell only for short commands (git, pnpm script, cargo build, smoke launch).
- **Research before reinventing** - user explicitly asked "use pwm/exa to copy enterprise templates" multiple times. C2-9 / C2-12 / C2-14 etc that involve third-party crate selection should call pwm ask first.
- **Ponytail full** - ladder enforced: reuse existing helpers / installed deps / stdlib native coverage > new deps. Deletion over addition.
- **A8 no v0.1.0 release** - mainline B publish pipeline stays dormant (ADR-0010 Track 1 only); after all items done, do NOT tag, do NOT trigger GitHub Actions.
- **Do NOT auto-pick grill pending items (C1-4 / C2-9..13 / P25burst-1..4 / P25-Q8-extra-1)** - those are pending until user confirms in grill. This round executes only the 12 confirmed items in PLAN SCHEDULE.

## Current test / build baseline (commit 6f2ac02)

- `cargo test -p resin-core --lib` = 79 passed
- `cargo test -p resin-core --bin resin-core` = 6 passed
- `pnpm test` (vitest) = 73 pass / 9 files
- `pnpm exec tsc --noEmit` green
- release exe = `release/windows-gui/ai-api-route.exe` 11.47MB (chunk `index-CfSStSwB`, not rebuilt since commit ca36fe3)
- smoke baseline: pid MainWindowTitle="ai-api-route" WS 36.4MB + resin.exe child 50.3MB (commit ca36fe3)

## Completion Definition

After the 12 confirmed items in PLAN SCHEDULE finish:
1. Each numbered item records its commit hash in PLAN SCHEDULE right column (audit trail).
2. `release/windows-gui/ai-api-route.exe` carries all 12 changes with freshly embedded Vite chunk.
3. All cargo test + vitest + tsc + i18n:check green.
4. AGENTS.md §Storage locations updated to "Database: observed_keys SQLite at app_config_dir()/ai-api-route.db (WAL)".
5. Do NOT tag, do NOT publish, do NOT trigger GitHub Actions; notify user "mainline A functional closure 12 items all green". Wait for user to set next phase (mainline B / VPS phase / continue grill pending items).

---

## If blocked mid-polish (cannot close the loop on Ponytail path)

Per AGENTS §execution_protocols stop/escalate: if an item stays blocked after 3 retries (e.g. r2d2 pool + async runtime panics under release exe), stop, commit current progress, tell user "mainline A item X blocked: <root cause>" - do not force-push. Then grill a next pending question or adjust plan.
