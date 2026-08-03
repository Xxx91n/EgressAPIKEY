# ai-api-route Domain Glossary

> This file defines domain terms only. No implementation details, no specs,
> no decisions (those live in docs/adr/). Updated inline as terms resolve.
> See docs/adr/ for architectural decision records.

## Glossary

### Platform
An isolated node pool with routing filters (region/regex) and an egress
allocation policy. Maps to Resin's Platform concept: each platform
maintains its own lease table and egress IP lease stats. The Default
platform contains all available nodes.

### Account
A unique business identity (e.g. an API key hash) bound to a platform.
Resin anchors traffic for each (Platform, Account) pair to a dedicated
egress IP via a sticky lease. If the bound node fails, Resin falls back
to another node with the same egress IP.

### Lease
A binding from (Platform, Account) to a specific (NodeHash, EgressIP)
with an expiry time (UnixNano, fixed - no renewal). Tracked per-platform
in xsync.Map<Account, Lease>. When expired, a new node is selected
for that (Platform, Account). Resin maintains IPLoadStats per
platform to count leases per egress IP.

### Subscription
A source of proxy node configurations (remote URL or local content).
Resin parses each subscription into a set of nodes (ManagedNodes),
updates them on an interval (min 30s), and marks nodes as
ephemeral/permanent. Cross-subscription dedup merges identical nodes
and shares health state.

### Node
A proxy endpoint (hash, display_tag, region, egress_ip, health, failure
count). Resin groups identical nodes across subscriptions via
GlobalNodePool and tracks per-node circuit-breaker state and
per-(node,domain) latency EWMA.

### Lane
Shell-side concept: a hash slot (0..MAX_LANES-1) that an (api_key,
upstream endpoint) pair maps to. In the Resin sidecar architecture,
this is implicit - Resin's Platform/Account/Lease mechanism replaces
the explicit lane-slot model. The term survives only in the Tauri
shell's IPC contract for backward compatibility (gateway_reserve,
lane_count).

### Ghost Safety Net
The 3-second /healthz poll that detects Resin sidecar death. After 3
consecutive failures, the tray turns red, the OS system proxy is
cleared, and an unhealthy event is emitted to the webview. Observation-
only; never restarts the sidecar.

### Sidecar
The Resin Go binary (resin-<triple>.exe) spawned as a Tauri shell
child process via tauri_plugin_shell::ShellExt::sidecar. Owns the
entire Platform/Account/Lease/Node runtime; the Rust shell only does
lifecycle, IPC forwarding, and the Ghost safety net.

### Egress IP Policy
The algorithm Resin uses to select which node an Account's lease binds
to. Resin v1.1.2 supports: BALANCED (round-robin quality weight),
PREFER_LOW_LATENCY (lowest EWMA latency first), PREFER_IDLE_IP
(fewest active leases per egress IP).

### Topology Canvas
The three-column A/B/C ReactFlow canvas in the desktop shell:
A (entry proxy port) - B (platforms) - C (node-region groups). Dragging
a B-to-C edge PATCHes the platform's region_filters live.

### Key Candidate
Shell-side display concept: a (v1_endpoint, apiKey) pair detected from
upstream AI gateway traffic, stored in settings.json#keyCandidates.
UID is an int32 hash of endpoint::apiKey for display uniqueness.

### Reverse-proxy Header Rule
The Resin platform field reverse_proxy_fixed_account_header (default
"Authorization") tells Resin which request header to parse for the Account
identity when the request has no explicit X-Resin-Account header. Combined
with reverse_proxy_empty_account_behavior, this is the zero-intrusion
Account extraction mechanism. The platform can also be configured with
reverse_proxy_miss_action to control what happens when no platform matches.

### Egress Policy
Shell-side alias for Resin's allocation_policy enum (BALANCED |
PREFER_LOW_LATENCY | PREFER_IDLE_IP). The shell exposes five UI labels
(random, sequential, latency, quality, bandwidth) that collapse to the
three Resin values; random/sequential/bandwidth map to BALANCED (documented
limitation).

### Protocol Weight
An advisory SSE-suitability ranking of node protocols (documented in
docs/PROTOCOL_WEIGHT_RESEARCH.md, NOT runtime-injected): http/socks5/
vmess-vless-tcp/trojan-tls-tcp = 1.0; shadowsocks = 0.7; hysteria2/tuic/
wireguard = 0.1. Resin v1.1.2 has no protocol-weighted selection endpoint;
this is reference for manual node choice, not a runtime selector.

### Backup
A JSON snapshot of the live Resin config (platforms + subscriptions +
account header rules) saved to app_data/backups with a crypto-random
suffix, BEFORE a topology drag PATCH or a config import. Best-effort:
backup failure never blocks the user routing change.

### Ghost
The 3-second /healthz poller in src-tauri/src/sidecar.rs. Detection-only.
After 3 consecutive failures (9s) it flips the tray red, clears the OS
system HTTP/HTTPS proxy, and emits a sidecar-status "unhealthy" event to
the webview. Recovery flips the tray green and emits "healthy".

### Request Log
Resin's per-request structured log entry (the audit trail by platform,
account, target site, and egress IP), separate from the Rust-shell
tracing file. Resin's README advertises it as a dashboard query surface;
the shell-desktop port does NOT yet expose it (ADR-0005 Q6). Likely lives
behind an admin endpoint that must be live-probed (DESIGN.md mentions the
log table but no public URL was confirmed).

### Observed Key Pool
A shell-side SQLite table (observed_keys, ADR-0011) keyed by route_id
(ar-<16hex>) that reverse-maps the interceptor-injected X-Resin-Account
back to a readable `(apiKeyMask, endpoint, first_seen, last_seen,
request_count)` tuple. Append-only; INSERT OR IGNORE on every new
tuple routed through the interceptor. Survives app restart in
`app_config_dir()/ai-api-route.db` (WAL mode). The GUI joins LeaseEntry.
account (ar-<16hex>) back to this table so each Topology B-column box
shows `key[0..4]...key[-4..] · endpoint` instead of an opaque hash.
_Avoid_: key pool, key registry (both noun-collision with keyCandidates
and Resin-internal cache names).

