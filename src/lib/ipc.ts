/**
 * Re3 IPC bridge: typed wrappers over Tauri commands exposing the Resin
 * Platform/Account registry. TS-layer validation per AGENTS s7.6.
 */
import { invoke as _invoke, Channel } from "@tauri-apps/api/core";

// --- T17 dual-mode: isTauri detection + cmd → REST route map (ADR-0043 Q2=A) ---
// In the Tauri webview we use the native invoke(). In a plain browser
// (headless npm server) we fall back to fetch("/api/v1/...") which the
// headless axum reverse-proxy forwards to the local Resin sidecar.
function isTauri(): boolean {
  return typeof window !== "undefined" &&
    !!(window as unknown as { __TAURI_INTERNALS?: unknown }).__TAURI_INTERNALS;
}

// Maps Tauri command names to the HTTP route the headless reverse-proxy exposes.
// Entries with undefined are Tauri-only (tray/desktop features with no headless surface).
type HttpMethod = "GET" | "POST" | "PATCH" | "DELETE";
interface HttpRoute { method: HttpMethod; path: string; }
const CMD_TO_HTTP: Record<string, HttpRoute | undefined> = {
  // Platforms
  platform_add:            { method: "POST",   path: "/api/v1/platforms" },
  platform_remove:         { method: "DELETE", path: "/api/v1/platforms" }, // needs name → id; done by a list+match in fetch mode
  platform_list:           { method: "GET",    path: "/api/v1/platforms" },
  platform_list_full:      { method: "GET",    path: "/api/v1/platforms" },
  platform_snapshot:      { method: "GET",    path: "/api/v1/platforms" },
  platform_update:        { method: "PATCH",  path: "/api/v1/platforms" }, // name → id lookup before patch
  platform_create_with_fields: { method: "POST", path: "/api/v1/platforms" },
  platform_leases:        { method: "GET",    path: "/api/v1/platforms" },
  // Subscriptions + nodes
  subscription_add:       { method: "POST",   path: "/api/v1/subscriptions" },
  subscription_remove:    { method: "DELETE", path: "/api/v1/subscriptions" },
  subscription_list:      { method: "GET",    path: "/api/v1/subscriptions" },
  node_list:              { method: "GET",    path: "/api/v1/nodes" },
  node_pool_snapshot:     { method: "GET",    path: "/api/v1/metrics/snapshots/node-pool" },
  // Port + gateway mirrors
  gateway_snapshot:       { method: "GET",    path: "/api/v1/metrics/realtime/leases" },
  request_log_tail:        { method: "GET",    path: "/api/v1/metrics/realtime/leases" },
  // Config / whitebox / system pass through the same /api/v1/* prefix
  config_export:          { method: "GET",    path: "/api/v1/config/export" },
  config_import:          { method: "POST",   path: "/api/v1/config/import" },
  system_config_get:      { method: "GET",    path: "/api/v1/system/config" },
  system_config_patch:    { method: "PATCH",  path: "/api/v1/system/config" },
};

async function invokeHttp<T>(route: HttpRoute, args?: Record<string, unknown>): Promise<T> {
  const init: RequestInit = {
    method: route.method,
    headers: { "Content-Type": "application/json" },
  };
  if (route.method !== "GET" && args) {
    init.body = JSON.stringify(args);
  }
  const r = await fetch(route.path, init);
  if (!r.ok) {
    const body = await r.text().catch(() => "");
    throw new Error(`IPC ${route.method} ${route.path} -> ${r.status}: ${body.slice(0, 256)}`);
  }
  if (r.status === 204) return undefined as T;
  return (await r.json()) as T;
}

const NAME_MAX = 128;

// --- Phase 5-1: trace_id passthrough (ADR-0026 Q1-Q5) ---
// Every IPC call automatically gets a UUID v4 trace_id injected into args.
// The Rust side extracts __trace_id and opens a tracing::info_span! so
// log files carry a per-call trace_id for end-to-end bug reproduction.
function genTraceId(): string {
  return crypto.randomUUID();
}

async function invoke<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const traceId = genTraceId();
  if (typeof console !== "undefined" && console.debug) {
    console.debug(`[trace_id=${traceId}] ipc.${cmd}`);
  }
  if (!isTauri()) {
    const route = CMD_TO_HTTP[cmd];
    if (!route) {
      throw new Error(`[ipc] command ${cmd} has no HTTP route mapping (Tauri-only in headless mode)`);
    }
    return invokeHttp<T>(route, args);
  }
  const enriched = { ...(args ?? {}), __trace_id: traceId };
  return _invoke<T>(cmd, enriched);
}

function assertShortName(v: string, field: string): void {
  if (!v || v.length > NAME_MAX || /[\x00-\x1f\x7f]/.test(v)) {
    throw new Error(`${field} invalid (1..${NAME_MAX} chars, no control)`);
  }
}

function assertIp(v: string): void {
  if (!v || v.length > 253 || /[\x00-\x1f\x7f\s]/.test(v)) {
    throw new Error("exit_ip invalid");
  }
}

export interface Account {
  id: string;
  platform: string;
  exit_ip: string | null;
  lane: number;
  active: boolean;
}

export interface LaneSnapshot {
  lane_count: number;
  busy: number;
  latencies: [string, number, number, number][];
  /// Per-platform (name, active_count) pairs from Resin /metrics/realtime/leases
  /// joined with /platforms to resolve platform_id -> user-visible name.
  per_platform_active: [string, number][];
}

export async function ipcPlatformAdd(name: string): Promise<void> {
  assertShortName(name, "platform");
  await invoke("platform_add", { name });
}

export async function ipcPlatformRemove(name: string): Promise<boolean> {
  assertShortName(name, "platform");
  return invoke<boolean>("platform_remove", { name });
}

export async function ipcPlatformList(): Promise<string[]> {
  return invoke<string[]>("platform_list");
}

/// Phase R2: full platform objects for the topology canvas. Returns raw JSON
/// (the Resin items-wrapper); the caller parses name/regex_filters/region_filters/
/// allocation_policy/routable_node_count.
export async function ipcPlatformListFull(): Promise<unknown> {
  return invoke("platform_list_full");
}

export async function ipcPlatformSnapshot(name: string): Promise<unknown> {
  assertShortName(name, "platform");
  // Returns the Resin items-wrapper: { items: NodeSummary[], total, limit, offset }.
  return invoke("platform_snapshot", { name });
}

/// T6-Bug2: Re-export from strategy.ts — shell 6-option is the sole UI source of truth.
export { STRATEGY_IDS, type StrategyId, type AllocationPolicy, strategyToI18nKey, strategyToResinPolicy, isValidStrategyId } from "./strategy";
import type { StrategyId, AllocationPolicy } from "./strategy";
import { strategyToResinPolicy, isValidStrategyId, STRATEGY_IDS } from "./strategy";
/// Back-compat: keep ALLOCATION_POLICIES for any call site that still imports it.
export const ALLOCATION_POLICIES = ["BALANCED", "PREFER_LOW_LATENCY", "PREFER_IDLE_IP"] as const;

/// Phase R1: PATCH a platform's allocation_policy / regex_filters / sticky_ttl.
/// TS-boundary validation mirrors the Rust side (AGENTS s7.6): policy enum,
/// filter count + length, ttl length + control chars. Only provided fields
/// are sent; the Rust side rebuilds the body.
export async function ipcPlatformUpdate(
  name: string,
  allocationPolicy?: StrategyId | AllocationPolicy,
  regexFilters?: string[],
  regionFilters?: string[],
  stickyTtl?: string,
  circuitBreakerDisabled?: boolean,
): Promise<unknown> {
  assertShortName(name, "platform");
  // T6-Bug2: translate shell StrategyId → Resin enum before sending to backend.
  let resinPolicy: AllocationPolicy | undefined;
  if (allocationPolicy !== undefined) {
    // Accept shell StrategyId (random/sequential/latency/quality/bandwidth/protocol_weight)
    // or Resin-native enum (BALANCED/PREFER_LOW_LATENCY/PREFER_IDLE_IP). Reject anything else.
    const knownResin: string[] = ["BALANCED", "PREFER_LOW_LATENCY", "PREFER_IDLE_IP"];
    if (!isValidStrategyId(allocationPolicy) && !knownResin.includes(allocationPolicy)) {
      throw new Error("allocation_policy must be one of " + [...STRATEGY_IDS, ...knownResin].join(", "));
    }
    resinPolicy = strategyToResinPolicy(allocationPolicy);
  }
  if (regexFilters !== undefined) {
    if (regexFilters.length > 64) throw new Error("regex_filters: too many (max 64)");
    for (const f of regexFilters) {
      if (f.length > 253 || /[\x00-\x1f\x7f]/.test(f)) throw new Error("regex_filter invalid (max 253, no control)");
    }
  }
  if (regionFilters !== undefined) {
    if (regionFilters.length > 64) throw new Error("region_filters: too many (max 64)");
    for (const r of regionFilters) {
      // lowercase ISO 3166-1 alpha-2 or !negation, max 16 chars
      if (r.length > 16 || /[\x00-\x1f\x7f\s]/.test(r)) throw new Error("region_filter invalid (max 16, no control/space)");
    }
  }
  if (stickyTtl !== undefined) {
    if (stickyTtl.length > 32 || /[\x00-\x1f\x7f]/.test(stickyTtl)) throw new Error("sticky_ttl invalid (max 32, no control)");
  }
  return invoke("platform_update", {
    name,
    allocationPolicy: resinPolicy ?? null,
    regexFilters: regexFilters ?? null,
    regionFilters: regionFilters ?? null,
    stickyTtl: stickyTtl ?? null,
    passive_circuit_breaker_disabled: circuitBreakerDisabled ?? null,
  });
}

/// Phase R1: GET /api/v1/nodes - the full node list (ip/ip channels) with
/// egress IPs, protocol, health. Returned as raw JSON; the frontend renders it.
export async function ipcNodeList(): Promise<unknown> {
  return invoke("node_list");
}

/// P21-B: POST /api/v1/platforms with full field schema (name, allocation_policy,
/// regex_filters, region_filters, sticky_ttl, etc). The body is a JSON object;
/// name is validated at the TS boundary. Resin re-validates server-side.
export async function ipcPlatformCreateWithFields(body: unknown): Promise<unknown> {
  if (!body || typeof body !== "object") throw new Error("platform body must be a JSON object");
  const b = body as Record<string, unknown>;
  if (typeof b.name !== "string") throw new Error("platform body missing 'name' string");
  assertShortName(b.name, "platform");
  return invoke("platform_create_with_fields", { body });
}

/// P21-B: GET /api/v1/platforms/{id}/leases - live leases for a platform,
/// used by the right pane to show which keys already have an exit IP bound.
export async function ipcPlatformLeases(name: string): Promise<unknown> {
  assertShortName(name, "platform");
  return invoke("platform_leases", { name });
}

/// P21-C: IP channel IPC wrappers — semantic aliases over the existing Resin
/// platform + node endpoints. The GUI sees "IP channels" which are Resin
/// Region-grouped nodes; the management surface is the platform PATCH surface
/// (allocation_policy, region_filters). These thin wrappers make the user's
/// mental model explicit in the call site without inventing a new backend.

/// List all IP channels (= Resin nodes, grouped by region in the GUI).
export async function ipChannelList(): Promise<unknown> {
  return invoke("node_list");
}

/// Set an IP channel's egress policy (= PATCH platform allocation_policy).
/// T6-Bug2: accepts shell StrategyId, translates to Resin enum internally.
export async function ipChannelPolicySet(
  platformName: string,
  policy: StrategyId | AllocationPolicy,
): Promise<unknown> {
  assertShortName(platformName, "platform");
  const knownResin: string[] = ["BALANCED", "PREFER_LOW_LATENCY", "PREFER_IDLE_IP"];
  if (!isValidStrategyId(policy) && !knownResin.includes(policy)) {
    throw new Error("ip_channel_policy_set: allocation_policy must be one of " + [...STRATEGY_IDS, ...knownResin].join(", "));
  }
  const resinPolicy = strategyToResinPolicy(policy);
  return invoke("platform_update", { name: platformName, allocationPolicy: resinPolicy, regexFilters: null, regionFilters: null, stickyTtl: null });
}

/// Create a new IP channel (= POST /platforms with fields).
export async function ipChannelCreate(body: unknown): Promise<unknown> {
  return ipcPlatformCreateWithFields(body);
}

/// Delete an IP channel (= DELETE /platforms/{id}, resolved by name).
export async function ipChannelDelete(name: string): Promise<boolean> {
  assertShortName(name, "ip_channel");
  return invoke<boolean>("platform_remove", { name });
}

export async function ipcAccountBindIp(platform: string, account: string, ip: string): Promise<boolean> {
  assertShortName(platform, "platform");
  assertShortName(account, "account");
  assertIp(ip);
  return invoke<boolean>("account_bind_ip", { platform, account, ip });
}

export async function ipcRefreshTray(): Promise<void> {
  await invoke("tray_refresh_labels").catch((e) => console.warn("[ipc] tray_refresh_labels failed", e));
}

export async function ipcGatewaySnapshot(): Promise<LaneSnapshot> {
  return invoke<LaneSnapshot>("gateway_snapshot");
}

// ---- Subscriptions + node pool (G4) ----
/// Resin-level (name, node_count) for each imported subscription.
export interface SubscriptionSnapshotEntry {
  name: string;
  node_count: number;
  healthy_node_count: number;
  last_error: string;
  last_checked: string;
}

export async function ipcSubscriptionAdd(name: string, url: string, updateInterval?: string): Promise<void> {
  assertShortName(name, "subscription");
  if (!url || url.length > 4096) throw new Error("subscription url invalid");
  if (!/^https?:\/\//.test(url)) throw new Error("subscription url must start with http:// or https://");
  await invoke("subscription_add", { name, url, updateInterval: updateInterval ?? null });
}

export async function ipcSubscriptionRemove(name: string): Promise<boolean> {
  assertShortName(name, "subscription");
  return invoke<boolean>("subscription_remove", { name });
}

export async function ipcSubscriptionList(): Promise<SubscriptionSnapshotEntry[]> {
  return invoke<SubscriptionSnapshotEntry[]>("subscription_list");
}

export async function ipcNodePoolSnapshot(): Promise<{ total_nodes: number; healthy_nodes: number; egress_ip_count: number; healthy_egress_ip_count: number }> {
  return invoke("node_pool_snapshot");
}


export type ReputationProvider = "ip_quality_score" | "abuse_ip_db" | "ip_api";
export interface ReputationEntry {
  ip: string;
  provider: ReputationProvider;
  score: number | null;
  proxy: boolean | null;
  vpn: boolean | null;
  tor: boolean | null;
  country_code: string | null;
  checked_at: number;
  cached: boolean;
}
export interface ReputationSnapshot {
  provider: ReputationProvider | null;
  status: "disabled" | "not_configured" | "ok" | string;
  entries: ReputationEntry[];
}

/** P4: server-side lookup of Resin's real public lease egress IPs. No free-form IP/URL reaches Rust. */
export async function ipcIpReputationSnapshot(): Promise<ReputationSnapshot> {
  const raw = await invoke<ReputationSnapshot>("ip_reputation_snapshot");
  return raw && Array.isArray(raw.entries) ? raw : { provider: null, status: "disabled", entries: [] };
}


/// Backup: create zip of settings + resin state, return temp path.
export async function ipcBackupCreate(): Promise<string> {
  return invoke<string>("backup_create");
}

/// Backup: upload zip to WebDAV server.
export async function ipcBackupUpload(url: string, username: string, password: string, zipPath: string): Promise<void> {
  if (!url || url.length > 2048) throw new Error("webdav url invalid");
  if (!/^https?:\/\//.test(url)) throw new Error("webdav url must start with http:// or https://");
  if (!zipPath) throw new Error("zip path must be non-empty");
  await invoke("backup_upload", { url, username, password, zipPath });
}

/// Backup: list backups on WebDAV server.
export async function ipcBackupList(url: string, username: string, password: string): Promise<string[]> {
  if (!url || url.length > 2048) throw new Error("webdav url invalid");
  if (!/^https?:\/\//.test(url)) throw new Error("webdav url must start with http:// or https://");
  return invoke<string[]>("backup_list", { url, username, password });
}

// ---- Process routing (issue 3+10) ----
export interface ProcessRouteRule {
  process: string;
  target_port: number;
}

export async function ipcProcessRouteAdd(process: string, targetPort: number): Promise<void> {
  assertShortName(process, "process");
  if (!Number.isInteger(targetPort) || targetPort < 1024 || targetPort > 65535) {
    throw new Error(`port ${targetPort} out of range (1024..65535)`);
  }
  await invoke("process_route_add", { process, targetPort });
}

export async function ipcProcessRouteRemove(process: string): Promise<boolean> {
  assertShortName(process, "process");
  return invoke<boolean>("process_route_remove", { process });
}

export async function ipcProcessRouteList(): Promise<ProcessRouteRule[]> {
  return invoke<ProcessRouteRule[]>("process_route_list");
}

/// Phase R4: export current platform + subscription config as JSON.
export async function ipcConfigExport(): Promise<unknown> {
  return invoke("config_export");
}

/// Phase R4: import a config JSON. Auto-backs up before applying.
/// Returns a summary { backup_path, platforms_created, platforms_skipped, subscriptions_created, subscriptions_skipped, errors }.
export async function ipcConfigImport(config: unknown): Promise<{
  backup_path: string;
  platforms_created: number;
  platforms_skipped: number;
  subscriptions_created: number;
  subscriptions_skipped: number;
  errors: string[];
}> {
  return invoke("config_import", { config });
}

/// Live lease row from Resin /api/v1/metrics/realtime/leases.
export interface LeaseEntry {
  platform_id: string;
  account: string;
  egress_ip: string;
  node_tag: string;
  target_domain: string;
  ts: string;
}

/// Live active lease map. Polled in the Topology canvas together with
/// platform_list + node_list so each platform card can show its active leases.
export async function ipcLeaseMap(): Promise<LeaseEntry[]> {
  const raw = await invoke<LeaseEntry[]>("lease_map");
  // Tolerate undefined/null (vitest with no IPC mock / sidecar down): empty array.
  if (!Array.isArray(raw)) {
    return [];
  }
  const cap = (s: unknown): string => (typeof s === "string" ? s.slice(0, 253) : "");
  return raw.map((e) => ({
    platform_id: cap(e?.platform_id),
    account: cap(e?.account),
    egress_ip: cap(e?.egress_ip),
    node_tag: cap(e?.node_tag),
    target_domain: cap(e?.target_domain),
    ts: cap(e?.ts),
  }));
}


// Phase 2 / ADR-0012: Entry Port = identity. Shell multi-port forwarder.
export interface PortMapping {
  port: number;
  protocol: string;
  platform_name: string;
  account: string;
  label: string;
  enabled: boolean;
  auth_required: boolean;
}

function assertPort(port: number): void {
  if (!Number.isInteger(port) || port < 1024 || port > 65535) {
    throw new Error(`port out of range (1024..65535): ${port}`);
  }
}

export async function ipcPortList(): Promise<PortMapping[]> {
  const raw = await invoke<PortMapping[]>("port_list");
  return Array.isArray(raw) ? raw : [];
}
export async function ipcPortSuggest(): Promise<number> {
  return invoke<number>("port_suggest");
}

export async function ipcPortUpsert(m: {
  port: number;
  protocol: string;
  platform_name: string;
  account?: string;
  label?: string;
  enabled?: boolean;
  auth_required?: boolean;
}): Promise<PortMapping> {
  assertPort(m.port);
  const protocol = (m.protocol || "socks5").toLowerCase();
  if (protocol !== "socks5" && protocol !== "http") {
    throw new Error("protocol must be socks5 or http");
  }
  // platform_name empty = unbound port (T8-7 ADR-0029). Allow empty.
  if (m.platform_name) assertShortName(m.platform_name, "platform_name");
  const account = m.account ?? "";
  const label = m.label ?? "";
  if (account.length > 128 || /[\x00-\x1f\x7f]/.test(account)) throw new Error("account invalid");
  if (label.length > 128 || /[\x00-\x1f\x7f]/.test(label)) throw new Error("label invalid");
  return invoke<PortMapping>("port_upsert", {
    port: m.port,
    protocol,
    platformName: m.platform_name,
    account,
    label,
    enabled: m.enabled !== false,
    authRequired: m.auth_required !== false,
  });
}

export async function ipcPortRemove(port: number): Promise<boolean> {
  assertPort(port);
  return invoke<boolean>("port_remove", { port });
}

/// T18-S2 (ADR-0042): Toggle enabled flag on an entry-port without
/// touching any other field. Patches the Resin endpoint `{enabled: bool}`
/// and persists the flag into the shell whitebox. The entry-port's other
/// fields (protocol, platform_name, account, auth_required) are preserved.
export async function ipcPortToggle(port: number, enabled: boolean): Promise<PortMapping> {
  assertPort(port);
  return invoke<PortMapping>("port_toggle", { port, enabled });
}

/// T8-1 (ADR-0029): Bind a port to a platform without touching auth_required.
/// Calls port_bind_platform IPC (not port_upsert) to avoid auth flip.
export async function ipcPortBindPlatform(port: number, platformName: string): Promise<boolean> {
  assertPort(port);
  // platformName empty = unbind, non-empty = validate short name
  if (platformName) {
    assertShortName(platformName, "platform_name");
  }
  return invoke("port_bind_platform", { port, platformName });
}


export async function ipcPortRunning(): Promise<number[]> {
  const raw = await invoke<number[]>("port_running");
  return Array.isArray(raw) ? raw : [];
}

export async function ipcPortReload(): Promise<number> {
  return invoke<number>("port_reload");
}

/// ADR-0021 Q1: SOCKS5/HTTP credentials a gateway must present to reach an
/// entry-port. Username is the port's bound Platform.Account string,
/// password is the sidecar global proxy_token. `auth_required` is read from
/// the port_mapping row (ADR-0027: per-port auth control via
/// require_proxy_auth_info).
export interface PortAuthInfo {
  port: number;
  username: string;
  password: string;
  auth_required: boolean;
  platform_name: string;
}

export async function ipcPortAuthInfo(port: number): Promise<PortAuthInfo> {
  if (port < 1024 || port > 65535) throw new Error(`port ${port} out of range (1024..65535)`);
  return invoke<PortAuthInfo>("port_auth_info", { port });
}

/// ADR-0021 Q1: live TCP probe + SOCKS5 method-negotiation so the GUI can
/// show a green/red health chip per port (clash-verge-rev CoreManager mode).
export interface PortHealthCheck {
  port: number;
  reachable: boolean;
  socks5_ok: boolean;
  protocol_mismatch: boolean;
  latency_ms: number;
  reason: "ok" | "refused" | "timeout" | "noop_no_reply" | "protocol_mismatch";
}

/// ADR-0026 Q9: protocol-aware health probe. For socks5 ports the Rust side
/// sends a SOCKS5 greeting; for http ports it sends an HTTP CONNECT probe.
/// Defaults to "socks5" when omitted (back-compat).
export async function ipcPortHealthCheck(port: number, protocol?: string): Promise<PortHealthCheck> {
  if (port < 1024 || port > 65535) throw new Error(`port ${port} out of range (1024..65535)`);
  const proto = (protocol ?? "socks5").toLowerCase();
  if (proto !== "socks5" && proto !== "http") throw new Error("protocol must be socks5 or http");
  return invoke<PortHealthCheck>("port_health_check", { port, protocol: proto });
}

// T18 Phase 1: streaming port health. One Tauri Channel<PortHealthSnapshot>
// per TopologyCanvas mount; the Rust side spawns a single Tokio task that
// probes every enabled entry-port concurrently (cap 10) and streams
// snapshots down this channel. The watcher ends when the channel is closed.
export type PortHealthState = "alive" | "degraded" | "dead" | "restarting";

export interface PortHealthEntry {
  port: number;
  state: PortHealthState;
  reachable: boolean;
  fails: number;
  latency_ms: number | null;
  interval_secs: number;
}

export interface PortHealthSnapshot {
  revision: number;
  entries: PortHealthEntry[];
}

/// Subscribe to port-health snapshots. Returns an unsubscribe function that
/// closes the channel (the Rust task ends on its next emit).
export function ipcWatchPortHealth(
  onSnapshot: (snap: PortHealthSnapshot) => void,
  onError?: (err: unknown) => void,
): () => void {
  const channel = new Channel<PortHealthSnapshot>();
  channel.onmessage = (snap) => {
    try { onSnapshot(snap); }
    catch (e) { if (onError) onError(e); }
  };
  // Stream commands bypass the trace_id wrapper — stream spans have no single
  // call boundary; the watcher itself logs via tracing::info! per tick.
  Promise.resolve(_invoke("watch_port_health", { onEvent: channel })).catch((e) => {
    if (onError) onError(e);
  });
  return () => {
    // Best-effort: Tauri 2 Channel has no explicit close; the Rust task
    // ends on its next emit when the webview GCs the JS Channel object.
    // To force a prompt stop, we null the handler so any in-flight message
    // becomes a no-op.
    channel.onmessage = () => {};
  };
}

// T6-4: Exit IP probe — routes a request to 1.1.1.1/cdn-cgi/trace through the
// given entry port and parses the exit IP from the Cloudflare trace body.
export interface ExitIpProbe {
  port: number;
  protocol: string;
  exit_ip: string;
  latency_ms: number;
  status: number;
}

export function ipcProbeExitIp(port: number, protocol: string): Promise<ExitIpProbe> {
  if (port < 1024 || port > 65535) throw new Error(`port ${port} out of range (1024..65535)`);
  const proto = protocol.toLowerCase();
  if (proto !== "socks5" && proto !== "http") throw new Error("protocol must be socks5 or http");
  return invoke<ExitIpProbe>("probe_exit_ip", { port, protocol: proto });
}

// T6-5: Firewall status check (Windows-only, read-only).
export interface FirewallStatus {
  platform: string;
  firewall_on: boolean;
  inbound_blocked: boolean;
  detail: string;
}

export function ipcCheckFirewallStatus(): Promise<FirewallStatus> {
  return invoke<FirewallStatus>("check_firewall_status");
}

// T6-5: Request log tail from Resin's request_logs SQLite DB.
export interface RequestLogEntry {
  ts: string;
  platform_name: string;
  account: string;
  target_host: string;
  egress_ip: string;
  http_method: string;
  http_status: number;
  duration_ms: number;
  resin_error: string;
}

export function ipcRequestLogTail(limit?: number): Promise<RequestLogEntry[]> {
  return invoke<RequestLogEntry[]>("request_log_tail", limit ? { limit } : {});
}

export interface NetworkConfig {
  dns_upstreams?: string[];
  max_idle_conns?: number;
  max_idle_conns_per_host?: number;
  idle_conn_timeout_secs?: number;
  probe_timeout_secs?: number;
  probe_concurrency?: number;
  proxy_bypass?: string[];
}

export interface WhiteboxConfig {
  version: number;
  entry_ports: PortMapping[];
  network?: NetworkConfig;
}

export async function ipcWhiteboxPath(): Promise<string> {
  return invoke<string>("whitebox_path");
}

export async function ipcWhiteboxGet(): Promise<WhiteboxConfig> {
  const raw = await invoke<WhiteboxConfig>("whitebox_get");
  return {
    version: Number(raw?.version ?? 1),
    entry_ports: Array.isArray(raw?.entry_ports) ? raw.entry_ports : [],
    network: raw?.network ?? {},
  };
}

/** Reload hand-edited egressapikey-ports.json into DB + listeners. */
export async function ipcWhiteboxReload(): Promise<number> {
  return invoke<number>("whitebox_reload");
}

/** T6-3: Save network-layer config (DNS + idle + probe + bypass) to whitebox JSON. */
export function ipcWhiteboxSaveNetwork(network: NetworkConfig): Promise<number> {
  return invoke<number>("whitebox_save_network", { network });
}

export interface StreamSensorSnapshot {
  unary: number;
  sse: number;
  websocket: number;
  unknown: number;
}

export async function ipcStreamSensorSnapshot(): Promise<StreamSensorSnapshot> {
  return invoke<StreamSensorSnapshot>("stream_sensor_snapshot");
}


export interface SidecarStatus {
  api_port: number;
  api_base: string;
  mode: string;
  /** T6-7: sidecar process PID (0 if not running). */
  pid: number;
  /** T6-7: RFC3339 timestamp of the last successful /healthz probe. */
  healthz_last_check: string;
  /** T6-7: round-trip latency of the get_sidecar_status IPC call (microseconds). */
  ipc_latency_us: number;
}

export async function ipcGetSidecarStatus(): Promise<SidecarStatus> {
  return invoke<SidecarStatus>("get_sidecar_status");
}


// ---------------------------------------------------------------------------
// Strategy Engine (T4-4 / ADR-0022) — whitebox per-platform strategy config.
// ---------------------------------------------------------------------------

/// T18-3 (ADR-0042 S3): B-class strategy parameters (shell-side whitebox only).
/// Mirrors Rust `crates/resin-core/src/strategy_engine.rs::BClassParams`.
export interface BClassParams {
  round_robin_n?: number;
  latency_threshold_ms?: number;
  quality_score?: number;
  bandwidth_weight?: number;
}

export interface PlatformStrategy {
  platform_name: string;
  a_class: "manual" | "region" | "quality" | "subscription";
  b_class: string; // StrategyId serialised as snake_case
  manual_nodes?: string[];
  regions?: string[];
  subscriptions?: string[];
  top_n?: number;
  b_class_params?: BClassParams;
}

export async function ipcGetConfigDir(): Promise<string> {
  return invoke<string>("get_config_dir");
}

export interface StrategyConfig {
  version: number;
  platforms: PlatformStrategy[];
}

export async function ipcStrategyConfigGet(): Promise<StrategyConfig> {
  return invoke<StrategyConfig>("strategy_config_get");
}

export async function ipcStrategyConfigPut(config: StrategyConfig): Promise<void> {
  if (config.version !== 1) throw new Error("strategy config version must be 1");
  if (!Array.isArray(config.platforms)) throw new Error("platforms must be an array");
  for (const ps of config.platforms) {
    if (!ps.platform_name || ps.platform_name.length > 128)
      throw new Error("platform_name must be 1..128 chars");
    if (ps.regions && ps.regions.length > 64)
      throw new Error("regions list too long (max 64)");
    if (ps.subscriptions && ps.subscriptions.length > 64)
      throw new Error("subscriptions list too long (max 64)");
    if (ps.top_n !== undefined && ps.top_n > 1000)
      throw new Error("top_n too large (max 1000)");
  }
  return invoke<void>("strategy_config_put", { config });
}

export interface StrategyApplyResult {
  platforms: Array<{
    platform: string;
    region_filters: string[];
    patched: boolean;
    reason?: string;
  }>;
}

export async function ipcStrategyApply(): Promise<StrategyApplyResult> {
  return invoke<StrategyApplyResult>("strategy_apply");
}
 
 // ---------------------------------------------------------------------------
 // Phase 5-2: typed IPC error contract (ADR-0026 Q6-Q8).
 // Discriminated union matching the Rust IpcError enum (externally-tagged serde).
 // Each variant carries an i18n_key so the GUI renders a locale-specific message
 // without parsing English error text.
 // ---------------------------------------------------------------------------
 
 export type IpcErr =
   | { kind: "BindConflict"; data: { port: number; i18n_key: string } }
   | { kind: "InvalidStrategy"; data: { value: string; accepted: string[]; i18n_key: string } }
   | { kind: "ResinUpstream"; data: { status: number; excerpt: string; i18n_key: string } }
   | { kind: "Internal"; data: { msg: string; i18n_key: string } };
 
 /** Narrow a thrown/unknown value from invoke() into a typed IpcErr.
  *  Tauri rejects with a string by default; if the Rust side returns
  *  IpcError via serde, Tauri serialises it as a JS object. */
 // ponytail: DONE (P31-T6, commit 900ddc5) — extractIpcErr now used by translateError in all GUI catch blocks
export function extractIpcErr(e: unknown): IpcErr {
   if (e && typeof e === "object" && "kind" in e && "data" in e) {
     const kind = (e as { kind: string }).kind;
     const data = (e as { data: Record<string, unknown> }).data;
     switch (kind) {
       case "BindConflict":
         return {
           kind: "BindConflict",
           data: {
             port: Number(data?.port ?? 0),
             i18n_key: String(data?.i18n_key ?? ""),
           },
         };
       case "InvalidStrategy":
         return {
           kind: "InvalidStrategy",
           data: {
             value: String(data?.value ?? ""),
             accepted: Array.isArray(data?.accepted) ? data.accepted.map(String) : [],
             i18n_key: String(data?.i18n_key ?? ""),
           },
         };
       case "ResinUpstream":
         return {
           kind: "ResinUpstream",
           data: {
             status: Number(data?.status ?? 0),
             excerpt: String(data?.excerpt ?? ""),
             i18n_key: String(data?.i18n_key ?? ""),
           },
         };
       case "Internal":
         return {
           kind: "Internal",
           data: {
             msg: String(data?.msg ?? ""),
             i18n_key: String(data?.i18n_key ?? ""),
           },
         };
     }
   }
   // Fallback: Tauri string rejection or unknown error -> Internal.
   const msg = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
   return { kind: "Internal", data: { msg, i18n_key: "error.internal" } };
 }
 
 /** Extract the i18n key from an IpcErr for direct use with t(). */
 export function ipcErrI18nKey(e: unknown): string {
   return extractIpcErr(e).data.i18n_key || "error.internal";
 }

/// T8-1: GET /api/v1/system/config — read system-level config.
export async function ipcSystemConfigGet(): Promise<unknown> {
  return invoke("system_config_get");
}

/// T8-1: PATCH /api/v1/system/config — update system-level config.
export async function ipcSystemConfigPatch(body: {
  max_consecutive_failures?: number;
  [key: string]: unknown;
}): Promise<unknown> {
  if (body.max_consecutive_failures !== undefined) {
    if (typeof body.max_consecutive_failures !== "number" ||
        body.max_consecutive_failures < 1 || body.max_consecutive_failures > 100) {
      throw new Error("max_consecutive_failures must be between 1 and 100");
    }
  }
  return invoke("system_config_patch", { body });
}
/// T8-6: Close all in-flight connections (kill + restart sidecar).
export async function ipcCloseAllConnections(): Promise<void> {
  return invoke("close_all_connections");
}

/// T8-6: Reset kernel (kill + restart sidecar — same impl, different semantic).
export async function ipcResetKernel(): Promise<void> {
  return invoke("reset_kernel");
}

/// T8-2: Strategy verification — probe N requests, collect exit IP + latency.
export async function ipcStrategyVerify(platformName: string, sampleCount: number): Promise<unknown> {
  assertShortName(platformName, "platform");
  if (typeof sampleCount !== "number" || sampleCount < 3 || sampleCount > 50) {
    throw new Error("sampleCount must be between 3 and 50");
  }
  return invoke("strategy_verify", { platformName, sampleCount });
}

/// T14-8: get lightweight mode config {enabled, delay_minutes}.
export async function ipcLightweightGet(): Promise<{ enabled: boolean; delay_minutes: number }> {
  try {
    const r = await invoke("lightweight_get") as { enabled: boolean; delay_minutes: number };
    return r;
  } catch {
    // Outside Tauri (vitest) — return defaults
    return { enabled: true, delay_minutes: 10 };
  }
}

/// T14-8: set lightweight mode config (enabled + delay_minutes).
export async function ipcLightweightSet(enabled: boolean, delayMinutes: number): Promise<void> {
  if (typeof delayMinutes !== "number" || delayMinutes < 1 || delayMinutes > 1440) {
    throw new Error("delayMinutes must be 1..=1440");
  }
  try {
    await invoke("lightweight_set", { enabled, delayMinutes });
  } catch (e) {
    // Outside Tauri — swallow
  }
}

export async function ipcSetLogLevel(level: string): Promise<string> {
  if (!["error", "warn", "info", "debug"].includes(level)) {
    throw new Error("invalid log level: must be error/warn/info/debug");
  }
  return invoke<string>("set_log_level", { level });
}

export async function ipcGetLogLevel(): Promise<string> {
  return invoke<string>("get_log_level");
}
