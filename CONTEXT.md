# EgressAPIKEY Domain Glossary

> This file defines domain terms only. No implementation details, no specs,
> no decisions (those live in docs/adr/). Updated inline as terms resolve.
> See docs/adr/ for architectural decision records.

## Glossary

### Entry Port
A local socks5/http/https listening port that the software exposes to
upstream AI gateways (omniroute, litellm, etc). Each port IS a key identity:
the gateway configures per-key (or per-key-group) proxy ports, and the
shell maps each port to a Resin (Platform, Account) pair. No header
parsing needed — the port number is the identity.
_Avoid_: listener, endpoint, interceptor

### Platform
A Resin concept: a named grouping of nodes with egress-IP-sticky leases.
In EgressAPIKEY, a platform is bound to one or more Entry Ports. Traffic
arriving on a port is forwarded to the platform's Resin account, which
guarantees a distinct exit IP per (platform, account) pair.
_Avoid_: group, pool, channel

### Account
A Resin concept: a business identity string (e.g. "port-17990") that
Resin uses to anchor a sticky egress IP lease. In EgressAPIKEY, the
account string is derived from the Entry Port number, NOT from the
upstream API key. The same port always maps to the same account, so
the same exit IP is reused for that port's traffic (until lease expiry).
_Avoid_: user, key holder, identity

### Lease
A Resin concept: a time-bounded binding of (Platform, Account) to a
specific egress IP on a specific Node. Resin's P2C + TD-EWMA algorithm
selects the node; the lease guarantees IP stickiness for the duration
(typically 1h-168h). SSE streams lock the lease until completion to
prevent mid-stream IP rotation.
_Avoid_: session, connection, binding

### Subscription
A Clash-format proxy subscription URL or local file. EgressAPIKEY
fetches it (with clash-family User-Agent), converts flow-style YAML to
block-style, and POSTs it as a local subscription to the Resin sidecar.
Resin's scheduler parses the proxies and adds them to the global node
pool. Update interval is configurable (default 30s for local subs).
_Avoid_: feed, source, provider

### Node
A proxy endpoint (hash, display_tag, region, egress_ip, health, failure
count). Resin groups identical nodes across subscriptions via
GlobalNodePool and tracks per-node circuit-breaker state and
per-(node,domain) latency EWMA.
_Avoid_: server, proxy, relay

### Ghost Safety Net
The Tauri shell's observation-only health monitor. Polls the Resin
sidecar /healthz every 3s; after 3 consecutive failures it flips the
tray red, clears the OS system HTTP/HTTPS proxy, and emits a
"sidecar-status: unhealthy" event to the webview. Never auto-restarts
the sidecar — restart policy is owned by the shell lifecycle.
_Avoid_: watchdog, monitor, guardian

### Sidecar
The Resin Go binary (resin-x86_64-pc-<abi>.exe) spawned by the Tauri
shell as a child process. Owns the P2C scheduler, TD-EWMA latency
tracking, sticky-IP lease table, and mihomo node runtime. The shell
communicates with it via loopback REST (admin token never crosses to
the webview).
_Avoid_: kernel, engine, daemon

### Egress IP Policy
Resin's native allocation_policy enum: BALANCED, PREFER_LOW_LATENCY,
PREFER_IDLE_IP. These are the ONLY egress selection knobs in Resin v1.1.2.
Random/sequential/bandwidth/protocol-weight strategies are shell-side
modular extensions that bias the platform configuration, not Resin
internals.
_Avoid_: exit strategy, routing mode, selection algorithm

### Topology Canvas
The three-column ReactFlow canvas in the desktop GUI. A column = Entry
Ports (left), B column = Platforms (center), C column = Node Groups by
region (right). A->B edges are always-connected (every port routes to
its platform). B->C edges = region_filters binding (drag to connect a
platform to a node region). Dragging an edge = live PATCH to Resin.
_Avoid_: graph, diagram, map

### Strategy Layer
A modular, pluggable decision layer in the shell that biases platform
configuration and node selection. Each strategy is independent:
liveness probing, latency weighting, bandwidth weighting, IP quality
scoring, protocol weight (SSE/WS suitability), IP reputation (external
API). Users pick which strategies to enable per platform.
_Avoid_: optimizer, scheduler, balancer

### AI Stream Sensor
An independent module that detects AI API traffic characteristics
(SSE streaming, WebSocket, chunked transfer) and applies per-stream
policies: lease locking during SSE, connection keepalive tuning,
mid-stream failover prevention. Pluggable and independent from the
Strategy Layer.
_Avoid_: traffic analyzer, stream handler

### IP Reputation Provider
An external API that scores an egress IP's trustworthiness.
Pluggable providers: IPQualityScore (fraud_score 0-100), AbuseIPDB
(abuse confidence score), ip-api.com (proxy/hosting/mobile flags),
ipinfo.io (geo+ASN). The Strategy Layer queries these on boundary
scores; local sliding-window cache avoids burning API quotas.
_Avoid_: IP checker, fraud detector, blacklist

### Protocol Weight
A documented (not runtime-injected) suitability ranking of outbound
node protocols for AI API SSE/WebSocket streams: http/socks5/vmess/
vless-tcp/trojan-tls-tcp = 1.0; shadowsocks = 0.7; hysteria2/tuic/
wireguard = 0.1. Used by the Strategy Layer's protocol-weight strategy
to bias node selection toward protocols that maintain SSE connections
without interruption.
_Avoid_: protocol score, transport rating

### Backup
A zip archive of settings.json + Resin state directory, created before
any topology drag-edit (防呆) and manually via Settings. Stored in
app_data/backups with a crypto-random suffix. Uploadable to WebDAV
(Koofr-compatible). Path-traversal guarded (P14 fix: canonicalize +
starts_with confinement).
_Avoid_: snapshot, checkpoint, save

### Entry Port Mapping
The SQLite table (reuses DbPool infra) that maps each Entry Port number
to a (platform_name, account_string) pair. Written by the shell when the
user creates a port in the GUI; read by the forwarder on every inbound
connection to inject the correct X-Resin-Account header before forwarding
to Resin. Schema migration = hand-written PRAGMA user_version (A12-revision).
_Avoid_: port table, route map, binding table

### hotswap-config
The whitebox configuration layer that lets users edit the port->platform
mapping and strategy settings via a config file (YAML/TOML), with atomic
backup before apply and hot-reload without restarting the sidecar. GUI
edits and file edits are kept in atomic transaction sync (A4 decision).
_Avoid_: live config, dynamic config, reload

### Request Log
The tauri-plugin-tracing daily-rotating file appender (10MB max, 7 files
kept) that captures every Rust-side tracing::info/warn/error. The user
can open the log directory from Settings > Storage. Used for debugging
topology drag edits, subscription imports, and sidecar lifecycle events.
_Avoid_: audit trail, access log, debug log
