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

### Subscription Refresh Lifecycle

What a per-subscription refresh in NodesView actually does. The shell
POSTs Resin `/api/v1/subscriptions/{id}/actions/refresh` (blocking —
fetch→parse→diff→apply run inline — but the body is only
`{"status":"ok"}`, no change data), then polls `list_subscriptions`
up to 5x at 500ms comparing `node_count` + `node_version` against the
pre-refresh row to decide `changed`. The `subscription_refresh` IPC
returns the real post-refresh `node_count` + `changed`; NodesView
re-reads the node list into the app store and counts from
`useAppStore.getState()` (never the pre-refresh closure snapshot):
count moved → "Refreshed, N nodes in pool", unchanged → "No node
count change; retry might be needed".
_Avoid_: fire-and-forget refresh, optimistic refresh toast

### Node

A proxy endpoint (hash, display_tag, region, egress_ip, health, failure
count). Resin groups identical nodes across subscriptions via
GlobalNodePool and tracks per-node circuit-breaker state and
per-(node,domain) latency EWMA.
_Avoid_: server, proxy, relay

### Ghost Safety Net

The Tauri shell's health monitor for the Resin sidecar. Polls /healthz
every 3s; after 3 consecutive failures it flips the tray red, clears the
OS system HTTP/HTTPS proxy, emits a "sidecar-status: unhealthy" event to
the webview, and then RESTARTS the sidecar for real (two-phase kill of the
child, respawn on the SAME control-plane port with the same admin token,
await /healthz, swap the child back into the handle; A-012). Each unhealthy
transition spends one of MAX_CRASH_RESTARTS=3 restart attempts with 1s/2s/4s
backoff; the transition after the budget is exhausted marks the sidecar
Terminated and stops restarting (the app must be restarted by the user).
Port identity is deliberately preserved across the restart because the
PortForwarder, diagnostics and strategy probe cache it at boot.
_Avoid_: watchdog, monitor, guardian

### Sidecar

The Resin Go binary (resin-x86_64-pc-<abi>.exe) spawned by the Tauri
shell as a child process. Owns the P2C scheduler, TD-EWMA latency
tracking, sticky-IP lease table, and node runtime. The shell
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
Pluggable providers (the three `ReputationProvider` variants in
`ip_reputation.rs`): IPQualityScore (fraud_score 0-100), AbuseIPDB
(abuse confidence score), ip-api.com (proxy/hosting/mobile flags).
Data source selection (ADR-0065): the shell calls these third-party HTTP
APIs directly and deliberately does NOT use Resin's built-in GeoIP
endpoints (`/api/v1/geoip/*`, R40-R43) — those return a geographic region
only, with no fraud/abuse dimension, while the shell's need is an
egress-IP trust verdict; GeoIP stays 故意不接 unless upstream ever adds
reputation signals (a new ADR is required to revisit). Local TTL cache
avoids burning API quotas.
_Avoid_: IP checker, fraud detector, blacklist, Resin GeoIP as the
reputation source

### Mixed Protocol

The default entry-port protocol value (round8 D-007): one port accepts
both HTTP-proxy and SOCKS5 clients, disambiguated by the connection's
first byte (0x05 = SOCKS5, otherwise HTTP). Explicit http and socks5
values remain for single-protocol ports; choosing socks5 no longer
implicitly opens HTTP forwarding on the engine. The shell forwarder
injects the port's (platform, account) credential in the dialect the
port declares (Proxy-Authorization for HTTP, RFC 1929 username/password
for SOCKS5). _Avoid_: auto-detect port, dual listener

### Protocol Weight

A documented (not runtime-injected) suitability ranking of outbound
node protocols for AI API SSE/WebSocket streams: http/socks5/vmess/
vless-tcp/trojan-tls-tcp = 1.0; shadowsocks = 0.7; hysteria2/tuic/
wireguard = 0.1. Used by the Strategy Layer's protocol-weight strategy
to bias node selection toward protocols that maintain SSE connections
without interruption.
_Avoid_: protocol score, transport rating

### Backup

A zip archive whose contents are fixed by the Backup Scope ledger below,
created before any topology drag-edit (防呆) and manually via Settings. It
carries a `manifest.json` (per-member SHA-256) and is optionally sealed
with a passphrase envelope — ChaCha20-Poly1305, with the random file key
wrapped by a PBKDF2-HMAC-SHA256 KEK (ADR-0070). Stored in app_data/backups
with a crypto-random suffix. Uploadable to WebDAV (Koofr-compatible) and
restorable from Settings: the config half re-enters the two authoritative
L2 write entries, while L3 and the audit chain are unpacked read-only as
evidence and never written back. Path-traversal guarded (P14 fix:
canonicalize + starts_with confinement).
_Avoid_: snapshot, checkpoint, save

### Backup Scope

The three-class ledger of what a backup may contain and what each class
means for config authority: (a) config-layer backups — the two whitebox
files plus their sibling `backup/` history (10 kept) and the write-audit
JSONL with its rotated archives, under `app_config_dir()`, restorable
through the same validate-before-swap write entry; (b) L3-derived backups —
`state.db` / `cache.db` packaged read-only into the user-facing zip export
by `backup_create` as `VACUUM INTO` snapshots, never written back
(ADR-0050-bis; snapshot method superseded by ADR-0070); (c)
`request_logs*.db` — never enter backups, in any class (leak prevention).
The packaging and restore mechanics of this ledger are legislated by
ADR-0070.
_Avoid_: backup layer, backup tier, backup kind

### Data-Plane Mode

Which process listens on an Entry Port and how the client authenticates
(round8 D-001). Mode A (shell forwarder): the Rust shell listens, the
client connects without credentials, and the forwarder injects the port's
(platform, account) as the proxy credential toward Resin. Mode B (engine
direct): Resin listens natively and the client supplies the
Platform.Account credential once. Desktop defaults to A; VPS/headless
defaults to B. A Resin fork with native per-port identity is the declared
fallback if A+B proves unmaintainable or misses the round-8 performance
targets. _Avoid_: zero-adapt (only mode A is credential-free), dual proxy

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

### Headless Control Plane

The browser SPA plus the Rust headless BFF that reproduce the desktop
control surface without Tauri (round8 D-003). The BFF holds the Resin
admin token server-side and never exposes it to the browser; it must
refuse to start when bound beyond loopback without an explicit auth
token, and rejects unexpected Host/Origin values. The browser UI is
the only first-class remote management surface; headless capability is
same-SPA parity with a documented disable list (ADR-0071).
_Avoid_: admin web panel, public dashboard

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

The bounded auto-retry policy the Ghost Safety Net applies when the Resin
sidecar is observed unhealthy (3 consecutive /healthz failures). Up to 3
retries with exponential backoff (1s, 2s, 4s), each one a REAL restart
(two-phase kill + respawn on the same port, A-012); a "restarting" event is
emitted during retries. After the 3 attempts are spent, the next unhealthy
transition marks the sidecar Terminated and the user is notified; the shell
does not attempt further spawns until the user restarts the app. The restart
re-binds the same control-plane port (RespawnSlot) instead of allocating a
new one.
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

### Write Retry

The bounded transient-failure retry applied to ResinClient write
verbs (T04/round5): POST/PATCH/PUT/DELETE go through
`send_with_retry` (2 retries, 500ms then 1000ms, warn log per retry
with the failure reason) on the same 500/502/503/504 + network-error
band as Read Retry; 4xx never retries (a rejected body is not
transient). GET keeps `send_read`. Safe because the create paths
are name-keyed (a duplicate POST returns the existing row or a 4xx,
never a second copy) and the action paths converge idempotently.
Lives in ResinClient, not in the IPC command layer, so all write IPC
commands benefit transparently.
_Avoid_: write retry loop, idempotent retry, backoff

### Subscription Update Interval

How often the Resin scheduler re-pulls a remote subscription
(Go `time.Duration` string on `POST/PATCH /api/v1/subscriptions`).
Resin v1.2.0 enforces a >= 30s floor on BOTH create and update
(`control_plane_subscription.go` `minSubscriptionUpdateInterval`):
the shell default is 30s, explicit values below the floor are
rejected 400 and NOT retried by Write Retry. The 5s default proposal
(T04) is blocked by this upstream contract — logged in
docs/research/OPENAPI-GAP.md for a future Resin-side change.
_Avoid_: poll interval, fetch interval, refresh interval

### Subscription Lifecycle

The full path a subscription travels: **import** (SubscriptionsView
POSTs Resin /api/v1/subscriptions; the row is data, not routing) ->
**bind** (the user names the subscription in a platform's whitebox
`subscriptions: Vec<String>` — either via the inline bind step after
import or the PlatformsView chips) -> **activate** (`strategy_apply`
derives region_filters from the node pool and PATCHes Resin,
diff-then-skip per ADR-0057) -> **observe** (the authoritative snapshot
reverse-lookup shows which platforms consume the subscription).
Importing is NOT binding, binding is NOT activating — the GUI now walks
the user through all three beats instead of implying that import alone
makes nodes routable. Per Round 5 T01 / R-A §3 (Gateway API
backendRefs + ProxySQL LOAD-TO-RUNTIME anchoring).
_Avoid_: import = effective, silent cascade, auto-bind-everything

### Imported-but-unbound

A subscription that exists on Resin (node_count > 0) but no whitebox
platform's `subscriptions` list names it. Visible by construction:
the authoritative snapshot's `subscriptions` reverse-lookup section
(`consumed_by: []`, `resolvable: true`) feeds the SubscriptionsView
row badge 未绑定 (amber) with a one-click inline bind step; a name
referenced by the whitebox but missing from Resin renders 引用失效
(red, `resolvable: false`). Zero consumers is not an error but must
never be silent (R-A Q2 patch 4).
_Avoid_: orphan subscription, dead import (nothing died — it was never
wired)

### Cascade Phase

The establish cascade a subscription walks after an import that asked for
it (Round 7 ticket 01, `subscription_add pipeline=establish`):
**create_subscription** (skip when the name is already on Resin) ->
**resolve** (node_count > 0 — Resin's own fetcher landed the nodes) ->
**establish platform** (the whitebox gains an a_class=subscription entry
named after the sub through the Service store entry, then the Resin row
is created when missing, ADR-0056) -> **strategy_config_put** (fused into
the same store entry as the previous beat) -> **strategy_apply**
(diff-then-skip, ADR-0057; generation write-back, ADR-0058). Level-
triggered and idempotent per step: a re-run over a converged world emits
zero writes. Terminal = consumed_by non-empty AND the target platform not
missing on Resin AND ConvergePhase in {Converged, Drifted(acknowledged)}.
A failed step is persistent state (backoff retries, then parked) — never
a silent auto-heal (ADR-0054).
_Avoid_: callback cascade (nothing chained by callbacks — one reconciler
drains a queue), auto-establish (the user's import opt-in IS the trigger)

### SubscriptionPhase

The per-subscription establish-phase STATUS record (Round 7 ticket 02,
D-C1.2) — the chip on each subscription row. Persisted in the strategy
whitebox top-level `subscriptions` array (NOT the per-platform name refs);
six states: **Never** (no row = the identity element — a cascade never ran),
**Importing** (data-landing beats: create_subscription + resolve),
**Establishing** (whitebox beats; the `stage` column names the beat:
platform/bind/port/apply), **Converged** (terminal green: every step wrote
or skipped), **Failed** (`stage` + `phase_error` say where and why —
persistent, survives restarts; a fresh user action re-walks the machine),
**NeedsApproval** (reserved — T03's conflict flow is the first producer).
STATUS, not spec: writes go through the ONE store entry (validate, backup
ring, audit row) but never bump the generation counter — the k8s
status-subresource rule; a status write must not flip ADR-0058's top-level
ConvergePhase into a fake PendingApply. Wire shape: parent field camelCase
(`subscriptionPhases`), row fields snake_case; a malformed phase degrades
to Never on the TS side (never to a terminal state).
_Avoid_: subscription status field vs `PlatformStrategy::subscriptions`
(the latter is the name-reference list), phase as desired state (it is an
observation — the cascade report is the truth), applying-generating
(counter never moves on status writes)

### Cascade Error Rollback

The partial-failure compensation of a failed establish cascade (Round 7
ticket 04, D-C1.4) — the ADR-0054-compliant EXPLICIT rollback, not a
silent auto-heal. When a cascade pass fails, ONLY the resources THIS pass
created are undone: the Resin-side platform row is deleted when this pass
created it AND the apply failure belongs to it (the failure-owner probe in
`apply_strategy`); the subscription (user data) is NEVER deleted; the
strategy whitebox (L2 desired state) is NEVER touched — whitebox deletion
is a separate ticket's path. The outcome persists as **last_cascade_error**
(`{ stage, reason, rollback_actions[] }`, schema LOCKED) on the
subscription's status row — `rollback_actions` is one ordered marking
entry per cascade step (sub/plat/port/apply) — written via
`record_cascade_failure` (status subresource, generation never moves);
the next Converged write clears it.
_Avoid_: auto-heal (the compensation runs once per failed pass, never
reconciles toward a hidden target), whitebox rollback (L2 is immutable
here), generic rollback (only cascade-created resources, only this pass)

### Generation

The whitebox write-authority counter (round5 T09 / ADR-0058, k8s
`metadata.generation` anchoring). Every sanctioned store-path write of
`egressapikey-strategy.json` (IPC put, deep region edit, rollback,
apply's own write-back) bumps it by one inside the write entry, after
validate and before the file lands. `egressapikey-ports.json` carries
its own SINGLE-generation counter (Crossplane local-type rule: writer
and applier are one process, the apply completes synchronously).
`generation = 0` means the file is untouched since the counter was
introduced — never surface that as "pending apply".
_Avoid_: revision (UI copy uses rev N for display, the field is
generation), version (that is the schema `version` byte), epoch

### AppliedGeneration

The observed half of the pair (k8s `status.observedGeneration`): the
generation the last FULLY GREEN `strategy_apply` pass landed at,
written back inside the same store entry after the pass returns
(R-B Q2). A green pass — including a diff-then-skip zero-PATCH pass —
converges the pair and refreshes `last_apply_at`; a failed pass keeps
the old value and records `last_apply_error` instead (faking
convergence is the bug class the counter exists to expose). The ports
half has NO applied generation by design.
_Avoid_: last applied revision, apply attempt counter (failures are
not counted here)

### ConvergePhase

The top-level, six-state convergence phase derived in the
authoritative snapshot (D-28), orthogonal to the per-entry three-state
of ADR-0051: `NeverApplied` (generation 0) / `Unknown` (Resin down —
honesty outranks guessing) / `ApplyFailed` (applied < generation with
an error record) / `PendingApply` (applied < generation, no error) /
`Drifted` (applied == generation but unacknowledged entry drift) /
`Converged` (applied == generation, no unacknowledged drift).
EffectiveConfigView renders it as the header chip: green 已生效于
HH:MM (rev N), red ApplyFailed · reason, amber 待应用 · 点击立即收敛
(clicking opens the existing reconcile preview). Zero new requests —
pure derivation over the snapshot already pulled.
_Avoid_: sync status (that is the per-entry three-state), health
(nothing here probes Resin health), auto-reconcile trigger

### A-Class Strategy

A strategy that controls which IP nodes enter a Platform. Modes (mutually
exclusive for auto): manual (user-selected node hashes), region (filter by
geo region), quality (filter by IP quality score threshold), subscription
(filter by subscription source). Manual + one auto mode can coexist. A
mandatory liveness gate (Resin ProbeManager + circuit breaker) excludes
unhealthy nodes before any strategy applies. Implemented in the shell-side
strategy_engine.rs, not in Resin.
Per round8 D-002, top-N and manual node selection land on Resin as the
selected nodes' region set: the documented semantics are region-filter,
not per-node pinning.
_Avoid_: ingress filter, node selector, admission policy

### B-Class Strategy

The legislated truth set for how a platform allocates an exit IP
(round8 D-002): exactly one of the three Resin-native allocation
policies — BALANCED, PREFER_LOW_LATENCY, PREFER_IDLE_IP. The former six
shell-side options are display-only and withdrawn from the UI; the
desired allocation_policy participates in strategy apply's diff-then-skip
and the snapshot's Consistent judgement.
_Avoid_: fake strategy, display-only parameter, six-strategy catalog

### Strategy Engine

The shell-side modular decision layer (strategy_engine.rs) that owns A-class
and B-class strategy evaluation. Periodically polls Resin /nodes for health +
latency, applies A-class filters to produce region_filters/regex_filters
PATCHes, and biases B-class lease selection. Config stored in
egressapikey-strategy.json (whitebox, hotswap-config atomic backup).
_Avoid_: optimizer, scheduler, balancer

### Strategy Pipeline Vocabularies

The three strategy vocabularies and their single composition point
(ADR-0052): the catalog is `strategy.rs` (StrategyId — the three Resin
allocation policies, the UI-facing names; the six legacy shell tokens
survive only as accepted input spellings for the one-time migration);
the planner is
`strategy_engine.rs` (StrategyConfig whitebox document, compute_plan,
parse_nodes — the region-computation vocabulary); the display mapping is
frontend `src/lib/strategy.ts` (StrategyId -> i18n key; the former
many-to-one Resin allocation_policy mapping collapsed to the identity in
round8 ticket 01). They are deliberately NOT merged into one enum;
`resin_core::StrategyService` (strategy_service.rs) is the only owner of
the strategyConfig lifecycle — read/validate/store/apply/snapshot
read-side/deep region edit — and the only sanctioned write path for
egressapikey-strategy.json. Per ADR-0056, apply establishes
missing-on-resin platforms (name-only create, then the region_filters
PATCH) and never deletes whitebox entries — removal happens by editing
the whitebox, never as an apply side effect. Per ADR-0057, apply
PATCHes only platforms whose live state actually drifts from the whitebox
(diff-then-skip on the snapshot's comparison rules); round8 ticket 01
extended the drift axes to include `allocation_policy`, and only the
drifting axes are sent, so a converged runtime takes zero strategy writes.
Storage form (round8 ticket 01): `PlatformStrategy::b_class` holds the
catalog value verbatim (the Resin `allocation_policy` spelling), so the
desired-vs-observed comparison needs no mapping table; the six legacy shell
tokens survive as accepted input spellings only and are rewritten once at
boot (`StrategyService::migrate_b_class_values_once`, generation untouched
because the mapping is many-to-one onto the policy already in effect), and
`BClassParams` is withdrawn from both the UI and the API surface.
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

### Drift Notification

The OS tray notification fired at the tail of `authoritative_snapshot`
(`src-tauri/src/tray.rs`). Semantics are per drift EPISODE, not once per
process (ADR-0060, revising ADR-0054 §E): the pure edge predicate
`should_fire_drift_notice(has_drift, prev_has_drift)` fires only on the
false→true rising edge of unacknowledged drift — sustained drift is
silent, the falling edge (drift cleared OR absorbed by acknowledged
exemptions) only updates the baseline and never emits, and drift
reappearing after a clear starts a new episode. Industrial isomorphs:
ArgoCD notifications `when` + `oncePer`, AWS Config compliance-state
transition SNS. Sidecar-down absence never notifies (ADR-0051).
_Avoid_: one-shot notify, armed/re-arm, once per process

### Subscription Intent

The declaration "this platform routes through this subscription". Binding a
platform to a subscription on the canvas (or via
`strategy_platform_subscriptions_set`) records the subscription itself as
the chosen strategy; the set of regions the subscription covers is a
DERIVED PROJECTION computed by the strategy engine at apply time. A platform
with a subscription bound draws unlabeled unconditional edges (see
Strategy-Labeled Edge). Rejected the older "subscription group = a bundle of
regions" reading, where dragging a subscription silently expanded into a
hand-written region list: it desynchronized the recorded intent from the
applied projection (ADR-0075).
_Avoid_: region bundle, subscription expansion, subscription group drag-fill

### Strategy-Labeled Edge

A topology canvas edge (B->C) annotated with the A-class strategy name that
caused the connection, in strategy:value form: "manual:N", "region:US",
"quality>75", "subscription:subA". Labels mark the CONDITION that made the
connection; an UNCONDITIONAL edge (platform with no subscription restriction)
carries NO label, per the Kiali/Grafana/LangGraph convention that default
flows stay unlabeled and only exceptions get annotated. For manual aClass,
N = total manual_nodes on the platform; the edge targets every
subscription/region group containing >=1 selected node_hash. Multiple edges
from one platform to different nodes indicate multiple strategies coexist.
Inspired by Kiali's edge labels for Istio routing rules. Auto-strategy edges
are non-deletable; only manual edges can be dragged/deleted.
_Avoid_: routing line, connection tag, policy edge, "all allowed" label
(the all-allowed state renders as an unlabeled edge, not a label)

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

### Echo Command

An IPC command that survives in the manifest with its full input
validation but returns a constant success without touching Resin — the
semantics live on the Resin sidecar side, so the shell body is an echo.
Echo commands exist for two reasons: (1) IPC contract stability — the
frontend command surface never shrinks behind a caller's back, so old
GUI builds and test stubs keep working; (2) validation stays at the
shell boundary (length caps, control chars, lane range) even when the
body forwards nothing. `account_add` and `account_bind_ip` are the
two surviving echoes since ADR-0050 deleted the kernel face; each call
logs `tracing::warn!` (target `ipc.account_deprecated`). Removal
condition: an echo is deleted only when the corresponding Resin REST
endpoint disappears or changes shape (see the AGENTS.md §7.6 echo
command list) — not merely because it has no caller.
_Avoid_: no-op, stub, dead command (echoes are alive contract surface)

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

### L2-First Write Order

The legislated order for configuration mutations (round8 D-006): persist
the intent to the L2 whitebox first, then mutate the L3 engine runtime.
A second-step failure leaves the system intent-declared and
action-pending — drift visible, converge retry recovers — never a silent
L3-only change or a lost intent. Saga-style compensation of L3 stays
reserved for the already-legislated narrow cascade case.
_Avoid_: rollback-first, transactional write

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
same reason). A run is previewed before it executes and wire-idempotent by
two distinct mechanisms that must not be conflated: the strategy half
diff-then-skips platforms whose live region set already matches (a
converged runtime takes zero writes, ADR-0057), and the ports half stays
inside its in-process TTL window; the snapshot re-check after either
outcome is the evidence of convergence, not a promise.
_Avoid_: self-heal, auto-sync, accept current state

### Deletion Authority (删除权威)

Desired-state (L2) entries may only be removed through an explicit
deletion act - a hand edit to the whitebox or a confirmed cleanup
action - never as a side effect of a save/sync path that reads the live
(L3) state (r11-wave-c D-002, ADR-0073). "Missing on Resin" is a drift
phase healed by re-assertion, not a deletion signal; a write-back fed by
a live list must hard-reject when the list is empty.
_Avoid_: auto-prune on save, trusting an empty live list

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

### Audit Event

One append-only JSONL row in `app_config_dir()/audit.jsonl` recording a
mutation of the L2 authoritative config (strategy JSON write entry or
whitebox `write_atomic`), per ADR-0059. Eight required fields: `schema`,
`ts`, `audit_id`, `target`, `op` (`put`/`apply`/`rollback`), `actor` (e.g.
`gui:strategy_config_put`), `before_hash`/`after_hash` (SHA-256 of full
written content), `outcome` (`ok`/`failed`); a rollback row additionally
carries `source_backup`, and every row chains to the previous one via
`prev_hash` (row-to-row, survives rotation). Audit logging is best-effort:
a failed append degrades to a tracing warning and never blocks the write
or app startup (Argus principle). The log answers actor history; the
10-deep backup ring answers content history.
_Avoid_: change log, event sourcing, security audit trail

### G4 (Signal Channel)

How the shell learns Resin-side state changes (T14 Round 5; revises the
"G4 webhook" misnomer per ADR-0024 / ADR-0050): there is NO HTTP webhook —
the Resin upstream Admin API is a pure pull model (GET/POST/PATCH/DELETE)
with zero webhook/callback/push egress (grep evidence in ADR-0062 D6). G4's
real shape = IPC retarget (shell `#[tauri::command]`s call ResinClient
REST) + sidecar-status event subscription: the shell-side Ghost G3 health
poll emits `sidecar-status` (healthy/unhealthy/restarting/terminated), and
TopologyView `app.listen("sidecar-status")` raises the red banner. The
signal origin is the shell's own poller, never a Resin push.
_Avoid_: Resin webhook, push notification, callback channel

### Global Converge Loop

The whole-app feedback circuit that keeps "did my change take effect?"
always on screen (Round 7 ticket 07, spec D-C2.2/D-C2.3): one
`subscribeToConverge()` subscription in `src/store/appStore.ts` (mounted
once in App.tsx) owns the `AuthoritativeSnapshot` cadence — 5s polling in
the foreground, backoff to 30s once ConvergePhase is `Converged` AND
`lastApplyAt` is more than 60s old (a settled green world earns the slow
heartbeat; a fresh apply stays watched closely), an immediate refresh on
the existing G4 `sidecar-status` event, and a full pause while the
document is hidden (visibility API). Every surface reads the same
`convergeSnapshot` store field (header pill, rail dot) and the Rust tray
mirrors the same phase edge-driven (repaint only on phase CHANGE):
Converged silent / Drifted + PendingApply amber / ApplyFailed red until
the next green apply / Unknown + NeverApplied grey. Display truth only —
the loop writes nothing (ADR-0051; writes keep flowing through the
Authoritative Write Entries).
_Avoid_: per-view polling ownership (views may refresh, only the
subscription owns cadence), webhook trigger (the loop rides polling +
the G4 event), write path (a mirror never mutates config)

### Account Header Rule

A Resin control-plane rule (round5 T16; R32-R35) that maps a URL prefix
(`host[/path]`, longest-prefix match, `*` wildcard fallback) to a list
of HTTP header NAMES. On the reverse-proxy data plane, when a platform's
`empty_account_behavior` is `ACCOUNT_HEADER_RULE`, Resin extracts the
account identity from the first of those request headers that carries a
value and routes to that account's sticky lease. Shell-side it is four thin
IPC pass-throughs (`list/put/resolve/delete_account_header_rule*`) over
the ResinClient seam — L3 control-plane state, NOT one of the three config
layers, and coexists with the OS-level process routes (ADR-0063, D-35).
_Avoid_: X-Resin-Account value routing (the rule stores header NAMES to
extract from), header-rule-to-port mapping (that is process_route)

### Name-to-UUID Resolution

How a shell-side caller turns a user-visible name ("alpha") into the Resin
UUID a single-resource endpoint needs ("DELETE /platforms/{id}"). Resin's
endpoint design is hybrid — list endpoints are name-keyed (rows carry
`name`+`id`), single-resource endpoints are UUID-keyed — so every caller
once re-implemented the "list all → match name → take id" two-step hop
(crack #6: 9 command bodies at Round 5 research time). T17 centralized it:
`ResinClient::resolve_platform_id_by_name` / `resolve_subscription_id_by_name`
(one list GET + §7.5 name validation: 253-char DNS-host cap, NUL/control
rejected as `IpcError::InvalidInput`) for command bodies, and the pure
`resin_core::resolve_id_in` for the fn-pointer seams
(`strategy_service::apply`/`reconcile`, whose single-GET wire shape is
pinned) and for `subscription_refresh` (the same list response also feeds
the pre-refresh row stats). A name miss surfaces as the typed
`IpcError::NotFound` carrying the historical message text ("platform not
found: {name}") with the pre-existing `error.notFound` locale key.
_Avoid_: name→id two-step hop in command bodies, stringly not-found error,
duplicate id_for_name helpers

### Platform Action

One of three upstream Resin POST endpoints under `/api/v1/platforms` that
trigger a server-side operation instead of reading or writing a resource:
`reset-to-default` (R13, recompiles a platform from env defaults),
`rebuild-routable-view` (R14, rebuilds Resin's internal routable node view),
and `preview-filter` (R09, dry-run node listing for a filter spec — no state
change). All three are deliberate non-adoptions per ADR-0066 (round5 T22):
reset-to-default would let the shell mutate L3 behind the L2 whitebox as the
source of truth; the routable view is maintained by Resin itself and kept
converged by ADR-0057 diff-then-skip on apply; filter preview already exists
shell-side as the ADR-0054 in-memory snapshot. preview-filter stays the one
wiring candidate for a later ticket (T23); any change to this classification
must revisit ADR-0066 and flip the RESIN_API_COVERAGE rows in the same commit.
_Avoid_: wiring reset-to-default through the shell, treating the action
endpoints as unexamined blanks, resurrecting them without ADR-0066

### Request Log Detail

The single-entry view over one Resin request-log row (round5 T21; R45/R46):
`request_log_detail` re-reads the full wire face of one row (timing,
first-byte, byte counts, upstream error fields) and `request_log_payloads`
fetches the captured request/response halves — base64-encoded upstream,
display-decoded shell-side with a 1 MB per-part cap plus upstream truncation
flags. The drawer opens by clicking a `request_log_tail` row (the row `id`
UUID rides the tail rows as of this ticket); payload capture depends on
Resin's payload-logging setting and surfaces 200-with-empty-strings when off.
_Avoid_: re-scanning request_logs*.db directly, rendering multi-MB bodies
unsliced, treating an empty payload body as an error

### License Layering

The repo-wide license model (ADR-0067 D1): one root `LICENSE`
(GPL-3.0-or-later, official verbatim text) with per-layer effective values —
own shell source GPL-3.0-or-later, vendored Resin source MIT, compiled
sidecar binary GPL-3.0-or-later by dependency-tree truth. Declared vs
dependency-tree values are different facts and both are recorded
(`THIRD_PARTY.md`); the declared value never erases obligations inherited
through dependencies.
_Avoid_: bare "MIT" for the Resin sidecar, editing the LICENSE text, one
flat license value for the whole tree

### Mere Aggregation

The legal boundary that keeps the shell (GPL-3.0-or-later) and the Resin
sidecar (MIT source / GPL-conveying binary) independent works distributed
side by side (ADR-0067 D3): they interact exclusively over the loopback REST
seam (`ResinClient`), with no in-process linking, no source embedding, no
shared struct surface. Under mere aggregation, each side keeps its own
terms; breaking the seam (FFI, vendoring one into the other) voids this
analysis and requires revisiting ADR-0067.
_Avoid_: FFI into resin, embedding the Go tree in the shell, calling the
REST seam "linking"

### Third-Party Provenance

The evidence discipline for third-party components (ADR-0067 D2): every
vendored/bundled component is registered in `THIRD_PARTY.md` with {version,
distribution form, declared license, dependency-tree truth, upstream link},
and every license claim cites upstream originals (go.mod, LICENSE at the
exact tag URL) — never memory. On any upstream version bump, the registry is
re-verified and updated in the same commit (ADR-0017 amendment).
_Avoid_: license claims without a source link, updating the manifest without
the registry, relying on a component's declared license alone

### Strategy Template

A preset snapshot of strategy choices (B-class allocation_policy plus
A-class region shaping) applied to a platform through the single
authoritative write entry, exactly like a hand edit (round11 D-002).
A template is a convenience over the same write path - never a second
config layer and never a bypass of generation or audit semantics.
_Avoid_: strategy presets stored outside the whitebox, applying a
template without a generation bump

### Strategy Orchestration

The platform-level conditional-automation state machine that reacts to
risk signals (a burned or challenged exit IP) by probing candidates,
switching region or strategy, holding an observation window, then
committing or rolling back, with ejection-style cooldown to prevent
flapping (round11 D-002). It reuses existing Resin probe and platform
PATCH APIs and introduces no new data-plane behavior. Automation is
tiered on the same state machine (r11-wave-c D-003): reversible switching
runs full-auto under a conservative parameter pack by default on
unattended headless, or holds at a suggest gate presenting a diff for
human approval by default on the attended desktop; large actions
(deletion, backup rollback, cross-platform moves, account-binding
changes) are never in its repertoire, and every transition is
audit-logged.
_Avoid_: per-request routing, content-aware switching (fork-line
territory), silent lease restarts

### Fork Line

The separate product line where data-plane innovations the vendored
Resin kernel cannot express are pursued (round11 D-001/D-002). Pinned
to exactly one trigger: per-request model-field-driven exit switching
inside one account; node-level pinning also lives here. Until the
trigger fires the fork line must not start, and the shell line must
not re-enter the kernel (ADR-0050).
_Avoid_: patching Resin casually, shell-side kernel forks

### Deployment Profile (部署剖面)

A named runtime context the product must satisfy (r12-wave-a D-002):
**low-end VPS headless** (1C1G-class Linux, egressapikey-headless + sidecar,
unattended) and **ordinary Windows desktop** (Tauri GUI + WebView2 process
tree + sidecar). Acceptance-line numbers are legislated per profile;
shared-runner measurements are warn-only reference columns and absolute
pins wait on representative hardware.
_Avoid_: benchmarking CI runners as if they were target hardware, one
profile silently covering both form factors

### Acceptance Line (验收线)

The legislated dual-threshold (target / danger) performance bar, per
deployment profile, that arbitrates ticket priority (r12-wave-a
D-001/D-002): tickets protecting or advancing a line upgrade; orthogonal
work must justify itself against the line or defer. Lines gate added-latency
deltas and good-event ratios rather than raw absolutes, and their domain is
deliberately narrow (no competitor benchmarking; Mode B data-plane
performance is Resin's domain).
_Avoid_: vanity metrics, throughput gates without latency/error caliber,
silently widening the domain

### Registered Exemption (登记型豁免)

A Known Exemption carrying a written removal condition and a named
registration point (r12-wave-a D-002): the defect stays visible and the
exemption is deleted when its condition fires (Mode B SSE buffering is
exempt only until upstream implements per-event flush). What separates it
from a silent waiver is the revocation path.
_Avoid_: permanent unowned exemptions, exemptions with no removal condition

### Trigger Line (触发线)

The suspension mechanism for deferred work (r12-wave-a D-003/D-005): an
item parks until a machine-checkable condition fires, then re-enters
adjudication (⑤c churn trigger, R12-01 unblock conditions, the four i18n
revisit lines). Every trigger line is covered by the per-round grill
inspection obligation so parked items cannot become orphans.
_Avoid_: "later" without a named condition, suspended items with no
inspector

### Good-Event Ratio (好事件占比)

The preferred SLI expression over bare percentiles (r12-wave-a D-002):
events meeting threshold / total valid events — e.g. "SSE per-event added
delta <=10ms in >=99% of events". It separates an occasional tail spike
from systemic degradation and feeds error-budget accounting directly.
_Avoid_: bare p99 as a gate, averaged latencies, thresholds on absolute WAN
latency instead of added delta

### Probe Verdict (探针裁决)

The typed per-port/tick signal-plane outcome (ADR-0080):
**ok | local_fail | remote_fail | skipped | environment_suspect**. Each
probed Entry Port joins its loopback health check with the end-to-end
egress probe; the join classifies WHERE a failure lives (shell-side entry
vs egress path vs environment) instead of collapsing to a boolean. A
verdict may carry a <=64-byte detail for IPC/audit surfaces; the ring
stores the enum only.
_Avoid_: boolean health flags, majority-rule port counts, unclassified "fail"

### Common-mode Suppressor (共模抑制器)

The tick-level guard (ADR-0080) that stamps an entire tick
**environment_suspect** when >=80% of all probed enabled bound ports fail
loopback in the same tick — one local outage is one environmental event,
never N platform failures. Suspect ticks enter the platform rings as time
barriers (evaluators skip them; denominators count valid verdicts only),
and three consecutive suspect ticks emit an audit row plus the
tick-level environment_status field.
_Avoid_: per-platform quorum tricks, suppressing partial-failure ticks,
user-tunable suppression thresholds
