# Resin API Coverage (RESIN_API_COVERAGE)

> Upstream surface: `resin/internal/api/server.go:65-157` (`NewServerWithAddress` mux
> registrations) + `resin/cmd/resin/main.go` bootstrap wiring. Bundled sidecar: **v1.2.0**
> (`docs/RESIN_UPSTREAM_MANIFEST.yaml`). Audited: 2026-09-04 (round5 T13).
> Legislation + maintenance discipline: [ADR-0062](../adr/0062-resin-api-coverage.md).

This is the complete consumption ledger for every upstream Resin control-plane route:
1 unauthenticated (`/healthz`) + 57 authenticated = **58 rows**. Any agent wiring a new
ResinClient method, IPC command, or doc claim about a Resin endpoint MUST check the row
here first and keep it in lockstep (ADR-0062 D5). ResinClient public endpoint methods
carry `/// @see docs/architecture/RESIN_API_COVERAGE.md #R<NN>` doc lines pointing at
their row id.

## Five-bucket semantics

| Bucket | Meaning | Rows |
| --- | --- | --- |
| 已对接 wired | ResinClient method exists AND is called by shell code (IPC command or resin-core service) | 22 |
| 装饰 decorative | IPC command registered but its body does NOT call the endpoint it is named after | 0 |
| 孤儿 orphan | ResinClient method exists with zero external callers (client unit tests only) | 4 |
| 空白 blank | No ResinClient method, no IPC command — upstream-only surface | 26 |
| 故意不接 deliberate | Unwired BY DECISION; reason recorded in ADR-0062 D3 or the cited ticket | 6 |

> The 装饰 bucket is currently EMPTY. reports/03 §2 #3 flagged `system_config_get/patch`
> as decorative commands, but their bodies have called `client.system_config_*` since T8-1
> (commit 856a0ff; the file only moved in arch/08). T15 must re-verify its premise against
> this table before deleting or rewiring anything.

## Out-of-table surfaces

- Embedded WebUI: `resin/internal/api/webui.go` registers `/`, `/ui`, `/ui/` (SPA assets). The shell never calls these.
- Data-plane token actions: `POST /{proxy_token}/api/v1/{platform}/actions/inherit-lease` (`handler_token_action.go:22`, mounted by `cmd/resin/inbound_mux.go:399/456`) — proxy-token authenticated, NOT part of the admin mux. `ResinClient` must never construct `/{token}/api/v1/...` paths.
- `/healthz` is unauthenticated on every listener; the shell consumes it with direct reqwest in `src-tauri/src/sidecar.rs` (boot await 15s, G3 ghost 3s poll), never through ResinClient.
- Headless BFF mode (ADR-0043): `src/lib/ipc.ts CMD_TO_HTTP` forwards a subset of the same IPC commands to `/api/v1/*` server-side; the rows below are identical, only the transport differs.

## D-34 finding: refresh-completion hook shape

`cmd/resin/main.go` wires bootstrap only — Resin exposes **no webhook and no event push**
(grep for webhook/callback/notify in `resin/internal/api/*.go` + `cmd/resin/main.go`:
zero hits). `POST /api/v1/subscriptions/{id}/actions/refresh` (R30) is a synchronous
blocking call that returns `{"status":"ok"}` with **no `changed` flag** in v1.2.0.
Therefore the T01/T14 refresh hook shape is: synchronous POST return + poll-based
observation of `GET /api/v1/subscriptions` field deltas (the round5-T03 30s edge poll).
The upstream-side `changed` output extension (spec D-24) is a Resin-side contract change
and stays outside T13 scope (issue 不做清单).

## Coverage table

Version column: `v1.2.0` rows are proven by the upstream release note (Endpoint
Management) or the ResinClient doc comments citing v1.2.0 handlers. `v1.0` rows mean
"present before the bundled v1.2.0 and documented in DESIGN.md"; the vendored `resin/`
checkout is a single shallow commit, so finer per-endpoint attribution is not derivable —
and upstream v1.3.0 (published, NOT bundled) added no new endpoints (its release note
lists only the tag-filter DB migration), so this table matches the bundled v1.2.0 binary.

| # | HTTP method + path | Resin handler (file:line) | ResinClient method (resin_client.rs:line) | IPC command / consumer | 状态 | 文档位置 | 引入版本 | 备注 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| R01 | GET /healthz | handler_healthz.go:7 | — (direct reqwest, not via client) | — (sidecar boot await + G3 ghost poll) | 已对接 | ARCHITECTURE.md sidecar section; sidecar.rs:371/956 | v1.0 | 唯一无鉴权端点；shell 直连轮询，不经 ResinClient |
| R02 | GET /api/v1/system/info | handler_system.go:57 | — | — | 空白 | — | v1.0 | WebUI 顶部信息用；shell 无消费场景（诊断卡候选） |
| R03 | GET /api/v1/system/config | handler_system.go:64 | system_config_get:441 | system_config_get (settings.rs:62) | 已对接 | settings.rs:58 (T8-1) | v1.0 | 熔断阈值等只读展示 |
| R04 | GET /api/v1/system/config/default | handler_system.go:75 | — | — | 故意不接 | ADR-0062 D3 | v1.0 | 出厂默认值由 Resin 自身管理；shell 无需展示不可改值 |
| R05 | GET /api/v1/system/config/env | handler_system.go:82 | — | — | 故意不接 | ADR-0062 D3 | v1.0 | env 由 boot_resin 注入（含 token）；读回无信息量且不得进 webview |
| R06 | PATCH /api/v1/system/config | handler_system.go:89 | system_config_patch:447 | system_config_patch (settings.rs:76) | 已对接 | settings.rs:72 (T8-1 / T19-P4) | v1.0 | max_consecutive_failures 1..=100 壳侧校验 |
| R07 | GET /api/v1/platforms | handler_platform.go:75 | list_platforms:274 | platform_list / platform_list_full / strategy_verify / authoritative_snapshot / reconcile_now / strategy_apply (Service) | 已对接 | ADR-0051/0052/0057 | v1.0 | name→id 两步跳模式核心（T17 helper 票收敛） |
| R08 | POST /api/v1/platforms | handler_platform.go:118 | create_platform:247 / create_platform_from_name:254 / create_platform_with_fields:394 | platform_add / platform_create_with_fields / strategy_apply (ADR-0056 建缺失平台) | 已对接 | ADR-0056 | v1.0 | 三方法同端点；from_name 镜像 Resin V1 名字规则 |
| R09 | POST /api/v1/platforms/preview-filter | handler_platform.go:202 | — | — | 故意不接 | ADR-0066 | v1.0 | dry-run 只读不改 Resin 状态；与 ADR-0054 壳侧内存快照重叠 — 唯一下轮接线候选 T23 (ADR-0066) |
| R10 | GET /api/v1/platforms/{id} | handler_platform.go:101 | get_platform:279 | — | 孤儿 | — | v1.0 | 列表已覆盖单读；仅 client 单测调用 |
| R11 | PATCH /api/v1/platforms/{id} | handler_platform.go:135 | update_platform:342 | platform_update / strategy_apply (region_filters diff-then-skip) | 已对接 | ADR-0057 | v1.0 | 收敛只 PATCH 真实漂移 |
| R12 | DELETE /api/v1/platforms/{id} | handler_platform.go:156 | delete_platform:285 | platform_remove | 已对接 | — | v1.0 | 先 list 反查 id 再删 |
| R13 | POST /api/v1/platforms/{id}/actions/reset-to-default | handler_platform.go:171 | — | — | 故意不接 | ADR-0066 | v1.0 | 从 env 默认重编译平台配置，绕过 L2 白盒权威面 (control_plane_platform.go:546) — 故意不接 (ADR-0066) |
| R14 | POST /api/v1/platforms/{id}/actions/rebuild-routable-view | handler_platform.go:187 | — | — | 故意不接 | ADR-0066 | v1.0 | Resin 内部 Pool 视图重建，节点变化自动维护；ADR-0057 diff-then-skip 已保收敛 — 故意不接 (ADR-0066) |
| R15 | GET /api/v1/endpoints | handler_endpoint.go:9 | list_endpoints:411 | port_list / port_upsert / port_remove / port_toggle / restore_ports_from_whitebox / reconcile_now / authoritative_snapshot | 已对接 | ADR-0042 S6 | v1.2.0 | 端口监听面读中枢 |
| R16 | POST /api/v1/endpoints | handler_endpoint.go:35 | create_endpoint:416 | port_upsert / restore_ports_from_whitebox / reconcile_now | 已对接 | ADR-0042 S6 | v1.2.0 | Resin 重启后按白盒重建监听 |
| R17 | GET /api/v1/endpoints/{id} | handler_endpoint.go:24 | get_endpoint:421 | — | 孤儿 | — | v1.2.0 | 封装无调用 |
| R18 | PATCH /api/v1/endpoints/{id} | handler_endpoint.go:51 | update_endpoint:427 | port_upsert / port_toggle | 已对接 | ADR-0042 | v1.2.0 | hot-reload 端口/能力 |
| R19 | DELETE /api/v1/endpoints/{id} | handler_endpoint.go:66 | delete_endpoint:433 | port_remove | 已对接 | ADR-0042 | v1.2.0 | 关闭监听 |
| R20 | GET /api/v1/platforms/{id}/leases | handler_lease.go:51 | platform_leases:403 | platform_leases | 已对接 | — | v1.0 | items-wrapper (account, egress_ip, node_hash, expiry) |
| R21 | DELETE /api/v1/platforms/{id}/leases | handler_lease.go:153 | — | — | 故意不接 | ADR-0062 D3 | v1.0 | 全平台清租破坏性大；close_all_connections 语义走 sidecar 重启（settings.rs:98） |
| R22 | GET /api/v1/platforms/{id}/leases/{account} | handler_lease.go:112 | — | — | 空白 | — | v1.0 | 单租约详情（租约抽屉候选） |
| R23 | DELETE /api/v1/platforms/{id}/leases/{account} | handler_lease.go:133 | — | — | 空白 | — | v1.0 | 单账户释放（候选） |
| R24 | GET /api/v1/platforms/{id}/ip-load | handler_lease.go:168 | — | — | 空白 | — | v1.0 | IP 负载分布（T19 邻接） |
| R25 | GET /api/v1/subscriptions | handler_subscription.go:46 | list_subscriptions:312 | subscription_list / subscription_refresh / subscription_remove / authoritative_snapshot / strategy_apply (引用解析) | 已对接 | ADR-0051; round5 T01/T03 | v1.0 | last_error 晋升 + 30s 边沿轮询的数据源 |
| R26 | POST /api/v1/subscriptions | handler_subscription.go:98 | create_subscription:306 | subscription_add | 已对接 | ADR-0045; round5 T04 write-retry | v1.0 | update_interval ≥30s 上游地板（GAP-01） |
| R27 | GET /api/v1/subscriptions/{id} | handler_subscription.go:82 | — | — | 空白 | — | v1.0 | 列表已覆盖单读 |
| R28 | PATCH /api/v1/subscriptions/{id} | handler_subscription.go:115 | — | — | 空白 | GAP-01 (T04) | v1.0 | shell 现走 delete+recreate；PATCH 可免节点抖动（候选） |
| R29 | DELETE /api/v1/subscriptions/{id} | handler_subscription.go:135 | delete_subscription:318 | subscription_remove | 已对接 | — | v1.0 | 204 → Null |
| R30 | POST /api/v1/subscriptions/{id}/actions/refresh | handler_subscription.go:150 | refresh_subscription_native:332 | subscription_refresh | 已对接 | ADR-0047 | v1.0 | 同步阻塞至 scheduler tick；出参仅 {status:ok} 无 changed（D-24 上游扩展不在本票）——见 D-34 finding |
| R31 | POST /api/v1/subscriptions/{id}/actions/cleanup-circuit-open-nodes | handler_subscription.go:166 | — | — | 空白 | — | v1.0 | 返回 cleaned_count；订阅节点卫生 UI 候选 |
| R32 | GET /api/v1/account-header-rules | handler_rules.go:41 | — | — | 空白 | T16 | v1.0 | keyword/limit/offset 参数面 |
| R33 | PUT /api/v1/account-header-rules/{prefix...} | handler_rules.go:59 | — | — | 空白 | T16 | v1.0 | url_prefix 只能走 path（DESIGN.md 规范路由） |
| R34 | POST /api/v1/account-header-rules:resolve | handler_rules.go:106 | — | — | 空白 | T16 | v1.0 | matcher 调试辅助 |
| R35 | DELETE /api/v1/account-header-rules/{prefix...} | handler_rules.go:94 | — | — | 空白 | T16 | v1.0 | 与 T16 CRUD 一并接入 |
| R36 | GET /api/v1/nodes | handler_node.go:95 | list_nodes:352 / list_nodes_for_platform:359 (孤儿) | node_list / authoritative_snapshot / strategy_apply | 已对接 | — | v1.0 | limit=500 显式分页（v1.2 分页契约变化）；platform_id 变体方法零调用 |
| R37 | GET /api/v1/nodes/{hash} | handler_node.go:177 | — | — | 空白 | — | v1.0 | 单节点详情列表已覆盖 |
| R38 | POST /api/v1/nodes/{hash}/actions/probe-egress | handler_node.go:190 | probe_node_egress:369 | node_probe | 已对接 | T19-P3 | v1.2.0 | 副作用更新 egress_ip/TD-EWMA |
| R39 | POST /api/v1/nodes/{hash}/actions/probe-latency | handler_node.go:203 | probe_node_latency:379 | node_probe | 已对接 | T19-P3 | v1.2.0 | TD-EWMA 更新 |
| R40 | GET /api/v1/geoip/status | handler_geoip.go:11 | — | — | 空白 | T20 | v1.0 | ip_reputation_snapshot 走第三方（IPQS/AbuseIPDB）；T20 立 ADR 解释 |
| R41 | GET /api/v1/geoip/lookup | handler_geoip.go:19 | — | — | 空白 | T20 | v1.0 | 单 IP 查询 |
| R42 | POST /api/v1/geoip/lookup | handler_geoip.go:50 | — | — | 空白 | T20 | v1.0 | 批量查询 |
| R43 | POST /api/v1/geoip/actions/update-now | handler_geoip.go:39 | — | — | 空白 | T20 | v1.0 | 手动触发 GeoIP 更新 |
| R44 | GET /api/v1/request-logs | handler_requestlog.go:16 | request_logs:464 | request_log_tail | 已对接 | arch-recovery 11 | v1.0 | 8 过滤参数 + cursor；REST 化封死 request_logs*.db 直读 |
| R45 | GET /api/v1/request-logs/{log_id} | handler_requestlog.go:174 | get_request_log:500 | — | 孤儿 | T21 | v1.0 | 单条详情待 IPC 暴露 |
| R46 | GET /api/v1/request-logs/{log_id}/payloads | handler_requestlog.go:198 | get_request_log_payloads:507 | — | 孤儿 | T21 | v1.0 | 仅 payload logging 开启时有 body |
| R47 | GET /api/v1/metrics/realtime/throughput | handler_metrics.go:147 | — | — | 空白 | T19 | v1.0 | 实时吞吐 |
| R48 | GET /api/v1/metrics/realtime/connections | handler_metrics.go:172 | — | — | 空白 | T19 | v1.0 | 实时连接 |
| R49 | GET /api/v1/metrics/realtime/leases | handler_metrics.go:197 | active_leases:292 | lease_map / ip_reputation_snapshot | 已对接 | P21/R2 | v1.0 | 顶替已删 resin-core LeaseTable |
| R50 | GET /api/v1/metrics/history/traffic | handler_metrics.go:236 | — | — | 空白 | T19 | v1.0 | 历史流量 |
| R51 | GET /api/v1/metrics/history/requests | handler_metrics.go:269 | — | — | 空白 | T19 | v1.0 | 访问成功率同源 |
| R52 | GET /api/v1/metrics/history/access-latency | handler_metrics.go:308 | — | — | 空白 | T19 | v1.0 | 访问延迟 |
| R53 | GET /api/v1/metrics/history/probes | handler_metrics.go:351 | — | — | 空白 | T19 | v1.0 | 主动探测次数 |
| R54 | GET /api/v1/metrics/history/node-pool | handler_metrics.go:383 | — | — | 空白 | T19 | v1.0 | 节点数量历史 |
| R55 | GET /api/v1/metrics/history/lease-lifetime | handler_metrics.go:417 | — | — | 空白 | T19 | v1.0 | 租约存活分布 |
| R56 | GET /api/v1/metrics/snapshots/node-pool | handler_metrics.go:458 | node_pool_snapshot:298 | node_pool_snapshot | 已对接 | — | v1.0 | 全局节点池快照 |
| R57 | GET /api/v1/metrics/snapshots/platform-node-pool | handler_metrics.go:479 | — | — | 空白 | T19 | v1.0 | 平台节点池快照 |
| R58 | GET /api/v1/metrics/snapshots/node-latency-distribution | handler_metrics.go:509 | — | — | 空白 | T19 | v1.0 | 节点延迟分布 |

## Maintenance

Every add / remove / reshape of an upstream route or of a ResinClient method MUST update
this table and ADR-0062 in the same commit (ADR-0062 D5). Bumping the bundled sidecar
version in `docs/RESIN_UPSTREAM_MANIFEST.yaml` requires re-auditing the row set against
the new binary (route strings / contract tests) before landing.
