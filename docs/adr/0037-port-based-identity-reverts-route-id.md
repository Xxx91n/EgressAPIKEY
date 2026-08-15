# Port-Based Identity as Single Source of Truth: route_id/interceptor reverted; port = identity

P1 决策（commit c71bab5，2026-08-05）：删除 `crates/resin-core/src/interceptor.rs`、`a4_3_live.rs`、`lane.rs::route_id/normalize_auth`、`InterceptorPort`。Commit message 明确记载 "Delete interceptor.rs, a4_3_live.rs, route_id/normalize_auth, InterceptorPort" 作为 P1 dead-code deletion + EgressAPIKEY rename 的一部分。

背景：P24-Q3 (commit dfd84b2) 引入 `route_id(auth_value, body_model, request_path) -> u64` 三元 FxHash + `normalize_auth` 把同一 key 在不同 scheme 下折叠为单一身份，配套 8 个 cargo 测试。P24-A4-3 (commit 紧随其后) 写入 axum interceptor.rs，在 omniroute/litellm 与 Resin sidecar 间充当三重组身份注入层，注入 `X-Resin-Account: ar-<16hex>`。AGENTS.md §28 / §30 当时按 "Live:" 描述这一层。

删除原因：P1 死代码审计判定 route_id/normalize_auth 在 Rust + TS 源码中零 caller（仅 lane.rs 自身测试调用），interceptor.rs 仅被 main.rs 实验性 setup spawn 且无用户路径触达。同期项目方向已从 lane-based 切到 port-based identity（见 GRILL_ISSUES_BACKLOG route-corrected 重定义项：A 列多端口、ProcessRouteView process->port、PlatformsView 左 pane entryPorts），port 号取代三个组成为身份字段。保留无 caller 的代码就是 Ponytail 反模式。

决策：不复接回 route_id/normalize_auth/interceptor。Port-based identity 已全面落地：
- TopologyView.tsx L572-579 ports.forEach 产生独立 entry-port-<port> 节点，L236 测试 "ADR-0012: A->B edge connects port to platform"
- PlatformsView.tsx L390 `t("platform.entryPorts")` 左 pane 为端口列表，KeyCandidates 概念已删除
- ProcessRouteView.tsx L30 `r.target_port`、L50 `processRoute.conflict { port: tgt }` 已 process->port
- Rust `process_route_add` (commands/mod.rs L407-425) `target_port` 字段 + `process_route_conflict_check` 基于 port 冲突检测

Rejected alternative：复活 route_id + interceptor 做 omniroute/litellm 前置注入层。代价大且与 port-based 方向冲突——port 号已经是稳定身份字段，再加三组 hash 是冗余的另一层身份，违反单一真源原则。

Consequence：AGENTS.md §28 / §30 的 "Live:" 描述必须校正为 "Reverted by P1 (c71bab5): killed by dead-code deletion as part of port-based-identity pivot; route_id/interceptor concept superseded by port-based identity model." 后续 agent 不应基于陈旧描述去"修不存在的 bug"或"接回已删除代码"。如果未来真的需要 omniroute 前置注入（比如 P24-A4-3 的 use case 真实出现），重写为 port-based 的版本，不复用 route_id 三元组。