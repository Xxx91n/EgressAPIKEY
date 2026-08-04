# ADR-0012: Route correction — thin-shell multi-port forwarder (port = identity)

Date: 2026-08-05
Status: ACCEPTED
Decision Type: Architecture route correction (supersedes ADR-0003, ADR-0011)

## Context

The original architecture (P24-A4-3) assumed the shell could intercept HTTPS
traffic and read the Authorization header to identify each (api_key, upstream
v1 endpoint) pair. Source-level research proved this is impossible: when the
upstream AI gateway (omniroute/litellm) connects to an HTTPS v1 endpoint, the
proxy layer only sees CONNECT tunnel bytes — the Authorization header is inside
the TLS envelope and never visible.

The user's actual flow is: agent -> AI gateway (omniroute/litellm) -> our
software (proxy layer) -> upstream v1 provider. The AI gateway already holds
the api key and the upstream endpoint. Our software is purely a network proxy
layer between the gateway and the upstream.

The only viable identity mechanism is port-based: the AI gateway configures
per-key (or per-key-group) socks5/http proxy ports on our software. Each port
IS the identity. The shell injects X-Resin-Account based on which port the
request arrived on — no header parsing needed.

This aligns with the user's original MEMORY_REUSE_DECISION.md: "Process/Account/
订阅管理直接拥抱 Resin...保留 mihomo 订阅编译 + Tauri 壳 + 进程路由三大差异化".
Resin's three entry types (HTTP forward proxy / SOCKS5 / reverse proxy) already
support multi-port listening. The shell adds the thin port->platform mapping
layer on top.

## Decision

**Path A (thin shell, not fork Resin):** the software exposes multiple
socks5/http/https entry ports. Each port maps to a Resin (platform, account)
pair. The shell injects X-Resin-Account based on the inbound port number —
no Authorization header parsing, no route_id, no interceptor.

Architecture:
1. Multi-port listener (tokio TcpListener + socks5/http protocol detection)
2. Port -> (platform_name, account_string) mapping table (SQLite, reuses DbPool)
3. X-Resin-Account injection on forward to Resin reverse-proxy
4. Resin sidecar unchanged (handles P2C, TD-EWMA, sticky exit IP, SSE lease)
5. Modular strategy layer (pluggable: liveness, latency, bandwidth, quality,
   protocol weight, IP reputation) — shell-side, does NOT fork Resin
6. AI stream sensor (SSE/WebSocket awareness) — independent pluggable module
7. hotswap-config (A5 decision) for whitebox config layer with atomic backup

## What becomes dead code (Q10 = a, delete)

- crates/resin-core/src/interceptor.rs (axum proxy_handler + route_id injection)
- crates/resin-core/src/lane.rs route_id + normalize_auth (8 cargo tests)
- crates/resin-core/src/db.rs observed_keys table + upsert (DbPool infra kept)
- ADR-0003 (key+endpoint identification via header) — SUPERSEDED
- ADR-0011 (B-B-3 route_id-derived key identity) — SUPERSEDED
- TopologyView B-column route_id chips (C1-1 entire line)
- PlatformsView left pane keyCandidates (endpoint+apiKey manual input)

## What is preserved

- Resin sidecar lifecycle (boot_resin, ghost health poll, SidecarHandle)
- Subscriptions CRUD + clash UA fetch + flow->block convert
- Node pool / IP channels (GET /api/v1/nodes)
- TopologyView C column (nodes by region)
- Backup/config export-import
- i18n / tray / settings persistence / Ghost safety net
- All C1-2/3/4, C2-1/2/5/7/8/9/10/11/12/13 backlog items (unrelated to identity)

## Consequences

- The shell is simpler than the interceptor: port number is identity, no
  header parsing, no FxHash, no normalize_auth.
- Resin stays un-forked; every upstream release can be re-vendored.
- Multi-port is the primary mode (one port per key/group). The architecture
  also supports a single unified port for advanced users who accept that
  all keys share one entry (no per-key isolation in that mode).
- IP reputation scoring (Q8) integrates as a strategy module, not a core path.
- The project name changes to EgressAPIKEY (ADR-0013).
