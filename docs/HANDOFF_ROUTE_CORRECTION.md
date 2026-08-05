# HANDOFF — Route Correction Execution Plan (2026-08-05)

> Post-grill handoff. Q1-Q11 all resolved. ADR-0012/0013/0014 ACCEPTED.
> This document supersedes HANDOFF_POLISH_PHASE.md for the route-correction
> execution phase. The old polish-phase items that survive (C1-2/3/4,
> C2-1/2/5/7/8/9/10/11/12/13) remain in GRILL_ISSUES_BACKLOG.md and execute
> AFTER the route-correction blocking items land.

## 启动提示词 (copy-paste to next Codex session)

遵循 ADR-0012/0013/0014 和 GRILL_ISSUES_BACKLOG.md 路线纠正后的执行计划，
设定目标，开始大型构建。
始终遵循 AGENTS.md，加载并使用 ctx_* 插件，必要时用 1mcp 的 exa/perplexity
联网搜索，不要产生幻觉推理。Ponytail full 模式。

[$ponytail:ponytail](C:\Users\Administrator\.codex\plugins\cache\ponytail\ponytail\4.8.4\skills\ponytail\SKILL.md)
[$ultragoal](C:\Users\Administrator\.codex\skills\ultragoal\SKILL.md)

## 必读上下文 (按顺序)

1. docs/adr/0012-route-correction-thin-shell-multi-port.md — 路线纠正架构决策
2. docs/adr/0013-project-rename-egressapikey.md — 项目改名
3. docs/adr/0014-dead-code-deletion-interceptor-line.md — 死代码删除清单
4. docs/GRILL_ISSUES_BACKLOG.md — 失效/保留/重定义/新增项完整审计
5. docs/MEMORY_REUSE_DECISION.md — 轮子复用决策 + 路线纠正结论
6. CONTEXT.md — 领域术语表 (已更新为 EgressAPIKEY)
7. AGENTS.md — 项目操作规范

## 执行顺序 (dependency graph)

### Phase 1: Clean break (blocking, serial)
1. NEW-9: Delete dead code (interceptor.rs, route_id, observed_keys schema, A4-3 test)
2. NEW-8: Rename project to EgressAPIKEY (Cargo.toml, tauri.conf, package.json, README, AGENTS)
3. Verify: cargo build + pnpm build + pnpm test + tsc green after deletion+rename

### Phase 2: Core architecture (blocking, serial) — **DONE (P2)**
4. NEW-1: Multi-port socks5/http listener (tokio TcpListener + protocol detection) ✅
5. NEW-2: Port -> (platform, account) mapping table (SQLite, reuses DbPool) ✅
6. NEW-3: Port-identity injection on forward (Resin V1 Platform.Account via SOCKS5 RFC1929 / HTTP Basic) ✅
7. Verify: closed-loop test — two ports -> two accounts -> two distinct egress IPs

### Phase 3: Strategy + config (parallel after Phase 2)
8. NEW-7: hotswap-config (whitebox config layer, atomic backup, hot-reload)
9. NEW-4: Modular strategy layer (pluggable strategies)
10. NEW-5: AI stream sensor module (SSE/WS awareness)

### Phase 4: IP reputation (after Phase 3)
11. NEW-6: IP reputation integration (IPQualityScore + AbuseIPDB + ip-api, pluggable)

### Phase 5: GUI redefinition (after Phase 2)
12. **DONE (P5a-7dd3c1d)** TopologyView A column = entry ports (multiple, each port = identity)
13. **DONE (P5b-7dd3c1d)** TopologyView B column = platforms with port-based chips
14. **DONE (P21)** PlatformsView left pane = entry ports (already refactored in P21)
15. **DONE (P21)** PlatformsView right pane = platforms (drag port -> platform already in P21)
16. **DONE (P5c-7dd3c1d)** ProcessRouteView = process -> port mapping (lane deprecated)

### Phase 6: Surviving polish items (after Phase 5)
17. C2-9: log system maturation
18. C2-10: path protection / privilege escalation
19. C2-11: env var / OS side-effect protection
20. C2-12: second instance guard + focus
21. C2-13: close GUI keeps tray icon
22. C1-2/3/4: topology edge atomicity + viewport (if not already done in Phase 5)

## Constraints

- Single-threaded (AGENTS §8 — no subagents)
- ctx_* first for all file edits and analysis
- Ponytail full: reuse wheels, don't reinvent
- Every code change: build + test + stage release exe + smoke launch
- Small step commits (Q6 = A6)
- git push after every commit (local push if remote token expired)
- codegraph sync after every code change

## Completion Definition

1. All Phase 1-5 items have commit hashes in GRILL_ISSUES_BACKLOG.md
2. release/windows-gui/ai-api-route.exe (or egressapikey.exe) carries all changes
3. All cargo test + vitest + tsc + i18n:check green
4. AGENTS.md updated with new architecture sections
5. CONTEXT.md reflects EgressAPIKEY terminology
6. Do NOT tag, do NOT publish, do NOT trigger GitHub Actions
7. Notify user "route correction execution complete, all phases green"
