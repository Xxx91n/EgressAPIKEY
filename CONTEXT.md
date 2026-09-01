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

The SQLite table (reuses DbPool infra) that stores each Entry Port number
paired with its (platform_name, protocol, label, account_string) metadata.
Written by the shell when the GUI creates a port (the same call forwards
the listener lifecycle to the Resin v1.2.0 /api/v1/endpoints admin API).
The port being live IS the account identity: Resin receives the inbound
connection on that port and binds traffic to the platform's sticky-IP
lease without any shell-side header injection (ADR-0014 deleted
interceptor.rs and route_id; ADR-0015 confirmed thin-shell forwarder).
Schema migration = hand-written PRAGMA user_version.
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

### RunningMode

The enum (Sidecar | NotRunning) that says whether the Resin Go binary
is currently alive as a child process of the desktop shell. Stored in
an ArcSwap for lock-free reads from any IPC command or tray handler.
Transitions: NotRunning -> Sidecar on boot_resin; Sidecar -> NotRunning
on kill. Not to be confused with the Ghost Safety Net health state
(healthy/unhealthy), which is a separate observation of the same
running sidecar.
_Avoid_: process state, alive flag

### Ring Buffer

A bounded (500-entry) VecDeque<String> that drains Resin sidecar
stderr line-by-line via tauri-plugin-shell CommandEvent::Stderr.
Oldest line evicts when full. Exposed to the GUI as an IPC snapshot
(get_sidecar_logs -> Vec<String>) and as a real-time Tauri event
push (sidecar-stderr). Resin emits ~30 stderr lines at boot then
goes quiet; 500 lines covers days of operation.
_Avoid_: log buffer, pipe drain, stderr cache

### Crash Restart

The bounded auto-retry policy when the Resin CommandChild is detected
dead (try_wait returns Some). Up to 3 retries with exponential backoff
(1s, 2s, 4s). The tray shows a "restarting" spinner during retries.
After 3 failures the sidecar is marked dead and the user is notified;
the shell does not attempt further spawns until the user triggers a
manual restart. Distinct from Ghost Safety Net which polls /healthz
while the process is alive.
_Avoid_: watchdog respawn, auto-recover

### Two-Phase Shutdown

The kill sequence used when the desktop exits or the user restarts the
sidecar: SIGTERM the entire process group -> wait 500ms -> try_wait ->
if still alive, SIGKILL -> wait (reap zombie). The process-group kill
ensures any child processes spawned by Resin are also terminated,
preventing orphans that hold the sidecar port.
_Avoid_: graceful kill, soft terminate

### Port Cleanup

Before spawning a new sidecar, check for a stale process holding the
configured free port. If found, SIGTERM the orphan and wait for it to
release the port. This prevents the "port occupied" boot failure where
a prior sidecar crashed without releasing its listen socket.
_Avoid_: port preflight, stale-process sweep

### Upstream Manifest

A YAML file (docs/RESIN_UPSTREAM_MANIFEST.yaml) that records the
pinned Resin sidecar version, per-platform SHA256 hashes, the release
URL, the API version surface, and any breaking changes or compat
notes from the last upgrade. fetch_resin.{ps1,sh} read the version
from this file instead of hardcoding a REL tag. Each version bump is
a manual edit + test + commit cycle; the shell does not auto-follow
upstream releases.
_Avoid_: version pin, compat matrix, version tracker

### Read Retry

The automatic bounded retry (2 attempts, 500ms interval) applied
only to ResinClient GET methods (list/get/snapshot). Write methods
(POST/PATCH/DELETE) do not retry to avoid duplicate mutations. Lives
in ResinClient, not in the IPC command layer, so all read-only IPC
commands benefit transparently.
_Avoid_: GET retry, idempotent retry, backoff

### A-Class Strategy

A strategy that controls which IP nodes enter a Platform. Modes (mutually
exclusive for auto): manual (user-selected node hashes), region (filter by
geo region), quality (filter by IP quality score threshold), subscription
(filter by subscription source). Manual + one auto mode can coexist. A
mandatory liveness gate (Resin ProbeManager + circuit breaker) excludes
unhealthy nodes before any strategy applies. Implemented in the shell-side
strategy_engine.rs, not in Resin.
_Avoid_: ingress filter, node selector, admission policy

### B-Class Strategy

A strategy that controls how an Entry Port selects an exit IP from a
Platform's node pool. Single-IP platforms are fixed (no strategy). Multi-IP
platforms pick one (mutually exclusive): random (OsRng true random),
round_robin (N requests per IP before rotating), low_latency (real-time
sort by EWMA). Implemented in the shell-side strategy_engine.rs; maps onto
Resin allocation_policy where possible, biases lease selection otherwise.
_Avoid_: egress selector, exit picker, rotation mode

### Strategy Engine

The shell-side modular decision layer (strategy_engine.rs) that owns A-class
and B-class strategy evaluation. Periodically polls Resin /nodes for health +
latency, applies A-class filters to produce region_filters/regex_filters
PATCHes, and biases B-class lease selection. Config stored in
egressapikey-strategy.json (whitebox, hotswap-config atomic backup).
_Avoid_: optimizer, scheduler, balancer

### Strategy Pipeline Vocabularies

The three strategy vocabularies and their single composition point
(ADR-0052): the catalog is `strategy.rs` (StrategyId 6 shell options +
protocol-weight table — the UI-facing names); the planner is
`strategy_engine.rs` (StrategyConfig whitebox document, compute_plan,
parse_nodes — the region-computation vocabulary); the display mapping is
frontend `src/lib/strategy.ts` (StrategyId -> i18n key, many-to-one ->
Resin allocation_policy). They are deliberately NOT merged into one enum;
`resin_core::StrategyService` (strategy_service.rs) is the only owner of
the strategyConfig lifecycle — read/validate/store/apply/auto-clean/snapshot
read-side/deep region edit — and the only sanctioned write path for
egressapikey-strategy.json.
_Avoid_: strategy monolith, vocabulary merge, three-source config

### Port Auth Info

The SOCKS5 credentials for an Entry Port: username = Platform.Account
string (e.g. Default.port-17990), password = RESIN_PROXY_TOKEN. Exposed
to the GUI via port_auth_info IPC for copy-to-clipboard. The token is
loopback-only; exposing it to the local webview does not increase attack
surface. SOCKS5 no-auth mode is not supported per-port (Resin architecture
limit: token is global).
_Avoid_: socks credentials, proxy auth, port password

### Port Health Check

An IPC command that TCP-connects to an Entry Port and optionally performs
a SOCKS5 handshake. Returns { reachable, auth_required, latency_ms }.
GUI shows green/red status per port. Auto-triggered after port create/update.
Modeled after clash-verge-rev CoreManager health check pattern.
_Avoid_: port probe, listener test, connectivity check

### Strategy-Labeled Edge

A topology canvas edge (B->C) annotated with the A-class strategy name that
caused the connection: "manual", "region:US", "quality>75". Multiple edges
from one platform to different nodes indicate multiple strategies coexist.
Inspired by Kiali's edge labels for Istio routing rules. Auto-strategy edges
are non-deletable; only manual edges can be dragged/deleted.
_Avoid_: routing line, connection tag, policy edge

### Trace ID

A UUID v4 string generated by the frontend invokeWithTrace wrapper on
every Tauri IPC call, injected as __trace_id into the args object. The
Rust command entry extracts it and opens a tracing::info_span! so every
log line in the daily-rotated file carries the same trace_id. Enables
end-to-end bug reproduction: a single grep in the log file traces the
full frontend -> command -> ResinClient -> Resin HTTP call chain.
_Avoid_: request id, correlation id, span id

### IpcError

A typed error enum returned by every #[tauri::command] instead of a
bare String. Variants: BindConflict(port), InvalidStrategy(value,accepted),
ResinUpstream(status,excerpt), Internal(msg). Each variant carries an
i18n_key so the frontend can render a locale-specific message without
parsing English error text. Externally-tagged serde for TS discriminated
union narrowing.
_Avoid_: IPC exception, command error, tauri error

### Account Strategy Tag

The B-class strategy encoded into the Resin account string (ADR-0026
scheme c). Format: port_label + "::" + tag where tag is random, rr-N,
latency, or fixed. Resin treats the full string as an opaque lease key;
the shell owns the strategy semantics. Different tags on the same platform
produce different leases, achieving per-strategy exit IP isolation without
forking Resin.
_Avoid_: strategy hint, account suffix, egress mode tag

### White-box Config

The user-editable configuration that the app reads back and honors as truth:
strategy configuration, entry-port configuration, and per-process routing
rules (ADR-0055), stored as JSON files in the app config directory. A user may edit these files with an external
editor; external edits are re-read and applied without restarting the
sidecar. Distinct from GUI preferences (which never change proxy behavior)
and from Resin runtime state (derived and rebuildable from white-box config).
_Avoid_: raw config, hand-edit zone, config dump

### Process Route

One rule binding an OS process name to an entry port: the process's traffic
enters through that port (and thereby that platform's exit). Lives in the
ports whitebox document (ADR-0055); a route is as live as the port it points
to — the shell does not claim per-process enforcement (Resin owns
per-request auth; the rule registry is shell-side metadata). The former
settings.json key was a mis-layering and is purged at boot.
_Avoid_: lane route (lanes are gone), app firewall rule

### Authoritative Write Entry

The rule that each configuration layer has exactly one module through which
every write flows, and that this write entry — not a cache, copy, or runtime
view — is the authority for that layer. All editors (GUI surfaces, external
file edits, IPC commands) converge on the same entry; a write that bypasses
the entry is a bug. Decided for strategy config by ADR-0036 and for port
config by ADR-0042; legislated across all three layers (GUI preference /
white-box / runtime) in docs/architecture/ARCHITECTURE.md § Config
Authority.
_Avoid_: golden config, master copy, single-writer lock

### Authoritative Snapshot

The read-back answer to "is my configuration actually in effect": one call
that reads every configuration layer, merges them at one sanctioned point,
and reports per item whether the white-box layer and the runtime agree,
disagree (both values surfaced, never silently reconciled), or one side is
missing. It turns "did my change take effect" from a guess into an assertable
fact and is the only legitimate place where stores are merged; views consume
it and must not re-merge stores themselves. Contract legislated in
docs/architecture/ARCHITECTURE.md § Config Authority.
Ticket 12 metadata (ADR-0054 §C/§D): `lastCheckedAt` stamps the generation
instant; `divergentSince` is the first-drift instant kept in PROCESS-LOCAL
memory only — a restart clears it and a re-drift re-times — and
`acknowledged` marks entities the user exempted ("known drift") in the
whitebox; exemptions NEVER alter the three-state merge, they only change
presentation (grey "known" badge) and silence notifications.
_Avoid_: status poll, health check, merged view

### Desired / Live (期望态 / 实况)

The two columns the Effective Config view (architecture-recovery ticket 13)
renders side by side: "desired" (期望态) is the whitebox layer's intent (L2
strategy/ports config), "live" (实况) is what the Resin runtime actually has
(L3). The pair replaces the vague "sync" vocabulary (ArgoCD #7629
precedent); the three-state badge (consistent / divergent / missingOnResin)
compares them and the view never picks a winner — divergence is surfaced,
and reconciliation stays an explicit one-way action (whitebox wins), never
an automatic one (ADR-0054).
_Avoid_: sync, merged view, current config

### Known Exemption (已知豁免)

A user-acknowledged drift entry: the entity id is listed in the whitebox
`acknowledged` array, the Effective Config view degrades its badge to a grey
"known" tag, and the tray notification stays silent. An exemption NEVER
alters the three-state merge or the snapshot data — it is presentation and
notification suppression only, and must be revoked explicitly.
_Avoid_: ignored, muted, auto-healed drift

### Reconcile (单向)

The explicit convergence action of the reconciliation loop: one user-
triggered run (Effective Config view "sync to desired state" button, or the
`reconcile_now` IPC) that re-asserts the L2 whitebox onto the L3 Resin
runtime — strategy apply first, then the ports restore, stopping at the
first failure. Direction is ONE-WAY: the whitebox ALWAYS wins; there is no
"accept current state" reverse write, no automatic self-heal, and no
background loop (ADR-0054 §A; ArgoCD ships selfHeal default-off for the
same reason). A run is previewed before it executes and idempotent within
its in-process TTL window; the snapshot re-check after either outcome is
the evidence of convergence, not a promise.
_Avoid_: self-heal, auto-sync, accept current state

### Whitebox Versioning (白盒版本化)

Every atomic write to either whitebox file first copies the current file to
the sibling `backup/` directory as `<original>.<unixts>[-N].bak` and keeps
the newest 10 per file (same-second collisions take a `-N` suffix, names
never overwrite). A listed backup can be rolled back; a rollback re-enters
the SAME validate-before-swap → apply chain as a hand edit (ADR-0036 /
ADR-0042 write entries, never a bypass), is itself backed up (reversible),
and is followed by an automatic snapshot re-check. Backups are runtime data
under `app_config_dir()` and never enter git (ADR-0054 §B).
_Avoid_: undo stack, autosave, config trash bin
