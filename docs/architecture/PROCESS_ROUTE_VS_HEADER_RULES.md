# Process Routes vs Account Header Rules (D-35)

> Round 5 T16. Two routing mechanisms coexist BY DESIGN — neither replaces
> the other (ADR-0063). This page is the decision aid for "which one do I
> reach for".

## The two mechanisms

- **project process_route_*** (ADR-0055): shell-side declarative memo with
  drift alerting. A rule maps a local process executable name to an entry
  port — it does NOT intercept or redirect that process's connections. The
  rule takes effect only once the process's own proxy settings point at the
  selected port; a route is exactly as live as that port's Resin listener
  (ADR-0055 D3). Stored in the L2 whitebox (`egressapikey-ports.json`),
  versioned + rolled back with the ports family, surfaced in the
  authoritative snapshot.
- **Resin account-header-rules** (ADR-0063): Resin-side, proxy-data-plane.
  A rule maps a URL prefix (`host[/path]`, longest-prefix trie, wildcard
  `*` fallback) to a list of HTTP header NAMES. On the REVERSE-proxy data
  plane, when a platform's `empty_account_behavior` is
  `ACCOUNT_HEADER_RULE` and neither `X-Resin-Account` nor a path account
  is present, Resin extracts the account identity from the FIRST of those
  request headers that carries a value, then routes to a sticky lease for
  that account. Wired to the shell as the four IPC commands
  `list/put/resolve/delete_account_header_rule*` (R32-R35).

## Six-dimension comparison

| 维度 | project process_route_* | Resin account-header-rules |
|---|---|---|
| 触发 | 无自动命中：白盒只记录「进程名 → 端口」意图；进程自行把代理指向该端口后流量才经由它 | HTTP 请求头（反向代理数据面按请求头提取账号身份；规则存的是头名列表，不是头值） |
| 匹配 | 进程名 → 入口端口 → 端口绑定的 Platform | URL 前缀 host[/path] 最长前缀匹配（trie，`*` 兜底）→ 头名列表 → 头值即账号 |
| 动态性 | 低：白盒文件编辑 + `watch_apply` 热载，重启前由文件说了算 | 高：控制面 PUT/DELETE 即时生效（matcher 原子热替换，无重启） |
| 跨进程 | 备忘可登记任意进程名，但不拦截连接——生效以进程自身代理指向为前提 | 否（仅反向代理数据面感知；`X-Resin-Account` 调试头在转发前被剥离，不会泄漏上游） |
| 用途 | 本地进程代理（开发 / 调试：让某个 exe 走指定出口） | 远程客户端账号路由（生产：同一 URL 前缀下按请求头把不同账号粘到不同租约） |
| 文档 | ADR-0055（L2 白盒字段 + 单写入口） | ADR-0063（本页 + RESIN_API_COVERAGE R32-R35） |

## When to use which

- "让本机的 Claude.exe / codex.exe 走某个平台的出口" → process_route
  (`process_route_add`) 登记意图，再把该进程自身的代理设置指向所选端口
  (如 HTTP_PROXY/HTTPS_PROXY 环境变量或应用内代理设置)——规则本身不接管
  流量，进程必须配合。
- "让远程调用方在同一个反向代理 URL 上声明自己用哪个账号" →
  account-header-rules（`put_account_header_rules`）+ 把该平台
  `empty_account_behavior` 设为 `ACCOUNT_HEADER_RULE`。调用方在请求里
  携带规则列出的头（如 `Authorization` / 自定义 `X-Account`）。
- 两者可以同时使用：本地调试走 process_route 固定端口；远程生产流量走
  header 规则。互不感知，互不替代。

## Upgrade path (archived — Round 8 ticket 05 / D-005)

If Resin upstream ever ships a real process-group API, the snapshot seam is
already shaped for it: `ProcessRouteSnapshot` carries the per-rule
`process` / `target_port` three-state and `merge_routes` consumes the L3
side as a normalized echo — today the caller feeds it the live entry-port
listener set (Resin has no process-group registry, ADR-0055 D3). The upgrade
is then a half-executed defensive path: swap the caller's echo source to the
process-group registry and the three-state merge, drift memory
(`route:<name>` keys) and acknowledgement vocabulary keep working
unchanged. INTERCEPTING a process's traffic is a different decision
entirely — a separate round on the same level as the Resin-fork decision
point (D-005), not part of this seam.

## Erratum vs the round5 research sketch

reports/03 §2 #4 and the issue F3 sketch described the rule headers as
"X-Resin-Account 值路由". The upstream source is more precise: the rule's
`headers` array holds header NAMES to EXTRACT the account from
(`reverse.go` `resolveReverseProxyAccount` phase 3 →
`extractAccountFromHeaders`), ``X-Resin-Account`` itself wins earlier
(phase 1) and is stripped before forwarding. This page carries the corrected
semantics; ADR-0063 records the same.
