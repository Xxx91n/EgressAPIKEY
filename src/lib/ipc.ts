/**
 * Re3 IPC bridge: typed wrappers over Tauri commands exposing the Resin
 * Platform/Account registry. TS-layer validation per AGENTS s7.6.
 */
import { invoke as _invoke, Channel } from "@tauri-apps/api/core";
// Ticket 09 (tauri-specta pilot): type contract comes from the
// specta-generated src/bindings.ts. TYPE-ONLY by design: the generated
// runtime wrappers bypass this module's trace_id injection and the headless
// CMD_TO_HTTP dual-mode, so runtime calls stay on invoke().
import type { LogLevel, PortMapping } from "../bindings";

// --- T17 dual-mode: isTauri detection + cmd → REST route map (ADR-0043 Q2=A) ---
// In the Tauri webview we use the native invoke(). In a plain browser
// (headless npm server) we fall back to fetch("/api/v1/...") which the
// headless axum reverse-proxy forwards to the local Resin sidecar.
function isTauri(): boolean {
  if (typeof window === "undefined") return false;
  // T17-audit: check ALL Tauri v2 injection variables.
  // __TAURI_INTERNALS__ is the IPC bootstrap (invoke/transformCallback).
  // window.isTauri is the official isTauri() flag (PR #9539).
  // Both are injected by the same AddScriptToExecuteOnDocumentCreated call.
  // If both are undefined, we're either in a plain browser (headless) or
  // there's a Tauri injection bug — log the diagnostic so we can tell.
  const w = window as unknown as {
    __TAURI_INTERNALS__?: unknown;
    isTauri?: unknown;
  };
  const hasInternals = !!w.__TAURI_INTERNALS__;
  const hasIsTauri = !!w.isTauri;
  if (!hasInternals && !hasIsTauri && typeof console !== "undefined") {
    // Diagnostic: only log once per session to avoid spam.
    try {
      const key = "__egressapikey_isTauri_diag";
      if (!(w as Record<string, unknown>)[key]) {
        (w as Record<string, unknown>)[key] = true;
        console.warn(
          "[isTauri] both __TAURI_INTERNALS__ and window.isTauri are undefined.",
          "location.href:", typeof location !== "undefined" ? location.href : "N/A",
          "userAgent:", typeof navigator !== "undefined" ? navigator.userAgent.slice(0, 100) : "N/A",
        );
      }
    } catch { /* swallow diagnostic errors */ }
  }
  return hasInternals || hasIsTauri;
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
  request_log_tail:        { method: "GET",    path: "/api/v1/request-logs" },
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
  const json = await r.json();
  // T22: Resin wraps list endpoints as { items: [...], total, limit, offset }.
  // Unwrap items for callers expecting a bare array (matches Rust items_arr helper).
  if (json && typeof json === 'object' && Array.isArray(json.items)) return json.items as T;
  return json as T;
}

const NAME_MAX = 128;

// User-facing entry-port range (1024..65535), single source for every
// TS-side user-range check in this module. Mirror of Rust
// resin_core::MIN_USER_PORT (crates/resin-core/src/port_forwarder.rs);
// the upper bound mirrors u16::MAX (Rust defines no dedicated MAX constant).
// Keep both sides aligned when the range changes.
const USER_PORT_MIN = 1024;
const USER_PORT_MAX = 65535;

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

/// T19-P2: refresh a subscription by re-fetching its url and PATCHing content.
/// Resin-side endpoint is subscription_refresh; returns the post-refresh node count.
export async function ipcSubscriptionRefresh(name: string): Promise<number> {
  assertShortName(name, "subscription");
  return invoke<number>("subscription_refresh", { name });
}

/// T19-P3: on-demand per-node probe. kind is "egress" or "latency". Returns
/// {egress_ip, region, latency_ewma_ms} for egress and {latency_ewma_ms} for
/// latency. The TS boundary validates hash length + kind membership so a
/// buggy caller can't POST to an arbitrary node path.
export interface NodeProbeEgressResult {
  egress_ip: string;
  region?: string;
  latency_ewma_ms?: number;
}
export interface NodeProbeLatencyResult {
  latency_ewma_ms: number;
}
export async function ipcNodeProbe(
  hash: string,
  kind: "egress" | "latency"
): Promise<NodeProbeEgressResult | NodeProbeLatencyResult> {
  if (!hash || hash.length > 128 || /[\x00-\x1f\x7f]/.test(hash)) {
    throw new Error("node_hash invalid (1..128 chars, no control)");
  }
  return invoke("node_probe", { nodeHash: hash, kind });
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

// ---- Process routing (ticket 17 / ADR-0055: L2 whitebox family) ----
export interface ProcessRouteRule {
  process: string;
  target_port: number;
}

export async function ipcProcessRouteAdd(process: string, targetPort: number): Promise<void> {
  assertShortName(process, "process");
  if (!Number.isInteger(targetPort) || targetPort < USER_PORT_MIN || targetPort > USER_PORT_MAX) {
    throw new Error(`port ${targetPort} out of range (${USER_PORT_MIN}..${USER_PORT_MAX})`);
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
export type { PortMapping };

function assertPort(port: number): void {
  if (!Number.isInteger(port) || port < USER_PORT_MIN || port > USER_PORT_MAX) {
    throw new Error(`port out of range (${USER_PORT_MIN}..${USER_PORT_MAX}): ${port}`);
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
  if (port < USER_PORT_MIN || port > USER_PORT_MAX) throw new Error(`port ${port} out of range (${USER_PORT_MIN}..${USER_PORT_MAX})`);
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
  if (port < USER_PORT_MIN || port > USER_PORT_MAX) throw new Error(`port ${port} out of range (${USER_PORT_MIN}..${USER_PORT_MAX})`);
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
  // T22: In non-Tauri (headless browser) mode, Channel constructor and
  // _invoke both touch window.__TAURI_INTERNALS__ which doesn't exist.
  // Return a no-op unsubscribe to avoid throwing inside React useEffect.
  if (!isTauri()) return () => {};
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
  if (port < USER_PORT_MIN || port > USER_PORT_MAX) throw new Error(`port ${port} out of range (${USER_PORT_MIN}..${USER_PORT_MAX})`);
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

// T6-5 (ticket 11): Request log tail via Resin GET /api/v1/request-logs.
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
  /** Ticket 12 (ADR-0054 §D): optional exemption list (decimal port numbers). */
  acknowledged?: string[];
}

export async function ipcWhiteboxPath(): Promise<string> {
  return invoke<string>("whitebox_path");
}

/** Ticket 12: sanitize a whitebox acknowledged exemption array (§7.6). */
function snapAckList(v: unknown): string[] | undefined {
  if (v === undefined || v === null) return undefined;
  if (!Array.isArray(v)) return undefined;
  return v
    .slice(0, 64)
    .map((x) => (typeof x === "string" ? x : ""))
    .filter((x) => x.length > 0 && x.length <= 128 && !/[\x00-\x1f\x7f]/.test(x));
}

export async function ipcWhiteboxGet(): Promise<WhiteboxConfig> {
  const raw = await invoke<WhiteboxConfig>("whitebox_get");
  return {
    version: Number(raw?.version ?? 1),
    entry_ports: Array.isArray(raw?.entry_ports) ? raw.entry_ports : [],
    network: raw?.network ?? {},
    acknowledged: snapAckList(raw?.acknowledged),
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

/**
 * Ticket 15 (ADR-0054 section B): one versioned backup copy of a whitebox
 * file. file_name is a server-generated `<original>.<unixts>[-N].bak` name - it
 * is NEVER interpolated into any URL or passed raw to another command; the
 * rollback wrappers validate the shape before invoking (section 7.6).
 */
export interface WhiteboxBackupEntry {
  file_name: string;
  unix_ts: number;
  size_bytes: number;
}

/** Ticket 15 (section 7.6): a backup name must look like <name>.<digits>[-N].bak. */
function assertBackupName(name: string): void {
  if (!name || name.length > 200 || !/^[A-Za-z0-9._-]+$/i.test(name) ||
      !/\.bak$/.test(name) || !/\.\d+(-\d+)?\.bak$/.test(name)) {
    throw new Error("whitebox_backup: backup_name invalid");
  }
}

/** Sanitize an untrusted backup entry list from the backend (section 7.6). */
function snapBackupEntries(v: unknown): WhiteboxBackupEntry[] {
  if (!Array.isArray(v)) return [];
  const out: WhiteboxBackupEntry[] = [];
  for (const raw of v.slice(0, 128)) {
    const r = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
    const fileName = typeof r.file_name === "string" ? r.file_name : "";
    try {
      assertBackupName(fileName);
    } catch {
      continue;
    }
    const ts = Number(r.unix_ts);
    const size = Number(r.size_bytes);
    if (!Number.isFinite(ts) || ts < 0 || ts > 4_102_444_800) continue;
    if (!Number.isFinite(size) || size < 0) continue;
    out.push({ file_name: fileName, unix_ts: ts, size_bytes: Math.floor(size) });
  }
  return out;
}

/** Ticket 15: list the versioned backups of the ports whitebox (newest first). */
export async function ipcWhiteboxBackupList(): Promise<WhiteboxBackupEntry[]> {
  return snapBackupEntries(await invoke("whitebox_backup_list"));
}

/**
 * Ticket 15: roll the ports whitebox back to a listed backup. The backend
 * re-enters the validate-before-swap -> apply chain and reconciles L3; the
 * response is the restored entry-port count.
 */
export async function ipcWhiteboxRollback(backupName: string): Promise<number> {
  assertBackupName(backupName);
  const n = await invoke<number>("whitebox_rollback", { backupName });
  return Number(n) || 0;
}

/** Ticket 15: list the versioned backups of the strategy whitebox (newest first). */
export async function ipcStrategyBackupList(): Promise<WhiteboxBackupEntry[]> {
  return snapBackupEntries(await invoke("strategy_backup_list"));
}

/** Ticket 15: roll the strategy whitebox back to a listed backup (backend re-validates + re-applies). */
export async function ipcStrategyRollback(backupName: string): Promise<unknown> {
  assertBackupName(backupName);
  return invoke("strategy_rollback", { backupName });
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
  /** Ticket 12 (ADR-0054 §D): optional exemption list (platform names). */
  acknowledged?: string[];
}

export async function ipcStrategyConfigGet(): Promise<StrategyConfig> {
  const raw = await invoke<StrategyConfig>("strategy_config_get");
  return {
    version: raw?.version,
    platforms: Array.isArray(raw?.platforms) ? raw.platforms : [],
    acknowledged: snapAckList(raw?.acknowledged),
  };
}

export async function ipcStrategyConfigPut(config: StrategyConfig): Promise<void> {
  if (config.version !== 1) throw new Error("strategy config version must be 1");
  if (!Array.isArray(config.platforms)) throw new Error("platforms must be an array");
  // Ticket 12 (§7.6): exemption list is optional; when present it must be a
  // bounded string array (≤64 × 1..128 chars, no control chars, no dupes).
  if (config.acknowledged !== undefined) {
    if (!Array.isArray(config.acknowledged)) throw new Error("acknowledged must be a string array");
    if (config.acknowledged.length > 64) throw new Error("acknowledged list too long (max 64)");
    const seenAck = new Set<string>();
    for (const a of config.acknowledged) {
      if (typeof a !== "string" || a.length === 0 || a.length > 128 || /[\x00-\x1f\x7f]/.test(a)) {
        throw new Error("acknowledged entry invalid (1..128 chars, no control)");
      }
      if (seenAck.has(a)) throw new Error("acknowledged entry duplicated: " + a);
      seenAck.add(a);
    }
  }
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

/// Ticket 10 (ADR-0052): deep edit — set one platform's region list through
/// the StrategyService (single sanctioned write path). The canvas no longer
/// assembles strategyConfig JSON client-side. Response is the stored document
/// (untrusted): re-validated shape before returning to callers.
export async function ipcStrategyPlatformRegionsSet(
  platformName: string,
  regions: string[],
): Promise<StrategyConfig> {
  assertShortName(platformName, "platform");
  if (!Array.isArray(regions) || regions.length > 64) {
    throw new Error("regions list too long (max 64)");
  }
  for (const r of regions) {
    if (typeof r !== "string" || r.length === 0 || r.length > 32) {
      throw new Error("region code invalid (1..32 chars)");
    }
  }
  const raw = await invoke<unknown>("strategy_platform_regions_set", { platformName, regions });
  const doc = raw as { version?: unknown; platforms?: unknown };
  if (doc.version !== 1 || !Array.isArray(doc.platforms)) {
    throw new Error("strategy_platform_regions_set: unexpected response shape");
  }
  return doc as StrategyConfig;
}

// ---------------------------------------------------------------------------
// Architecture-recovery ticket 07: authoritative effective-config snapshot
// (CONTEXT.md: Authoritative Snapshot; ARCHITECTURE.md §Config Authority).
// ONE pre-merged read-back of L2 strategy whitebox + L2 ports whitebox +
// L3 Resin runtime. The merge lives in resin-core (snapshot.rs); views
// consume this and must NOT re-merge stores. Responses are treated as
// untrusted: every field passes through a sanitizer before use.
// ---------------------------------------------------------------------------

export type SnapshotState = "consistent" | "divergent" | "missingOnResin";

export interface StrategySnapshotConsistent {
  state: "consistent";
  platform_name: string;
  platform_id: string;
  regions: string[];
  resin_allocation_policy: string;
  b_class: string;
  a_class: string;
  manual_nodes: string[];
  subscriptions: string[];
  /** Ticket 12 (ADR-0054 §D): read-side exemption flag; never influences the merge. */
  acknowledged: boolean;
}
export interface StrategySnapshotDivergent {
  state: "divergent";
  platform_name: string;
  platform_id: string;
  whitebox_regions: string[];
  resin_regions: string[];
  resin_allocation_policy: string;
  b_class: string;
  a_class: string;
  manual_nodes: string[];
  subscriptions: string[];
  /** Ticket 12 (ADR-0054 §C): Unix seconds of first in-process drift; undefined while fresh. */
  divergent_since?: number;
  /** Ticket 12 (ADR-0054 §D): read-side exemption flag. */
  acknowledged: boolean;
}
export interface StrategySnapshotMissingOnResin {
  state: "missingOnResin";
  platform_name: string;
  platform_id: string;
  regions: string[];
  a_class: string;
  b_class: string;
  manual_nodes: string[];
  subscriptions: string[];
  /** Ticket 12 (ADR-0054 §C): Unix seconds of first in-process drift. */
  divergent_since?: number;
  /** Ticket 12 (ADR-0054 §D): read-side exemption flag. */
  acknowledged: boolean;
}
export type StrategySnapshot =
  | StrategySnapshotConsistent
  | StrategySnapshotDivergent
  | StrategySnapshotMissingOnResin;

export interface PortSnapshotConsistent {
  state: "consistent";
  port: number;
  platform_name: string;
  protocol: string;
  account: string;
  label: string;
  enabled: boolean;
  auth_required: boolean;
  /** Ticket 12 (ADR-0054 §D): read-side exemption flag. */
  acknowledged: boolean;
}
export interface PortSnapshotMissingOnResin {
  state: "missingOnResin";
  port: number;
  platform_name: string;
  protocol: string;
  account: string;
  label: string;
  auth_required: boolean;
  /** Ticket 12 (ADR-0054 §C): Unix seconds of first in-process drift. */
  divergent_since?: number;
  /** Ticket 12 (ADR-0054 §D): read-side exemption flag. */
  acknowledged: boolean;
}
export type PortSnapshot = PortSnapshotConsistent | PortSnapshotMissingOnResin;

/** Ticket 17 (ADR-0055 D3): per-route three-state. The live side of a route
 *  IS its target port (Resin has no per-process object), so the variants
 *  mirror the ports family. */
export interface ProcessRouteSnapshotConsistent {
  state: "consistent";
  process: string;
  target_port: number;
  /** ADR-0055 D6: read-side exemption flag; never influences the merge. */
  acknowledged: boolean;
}
export interface ProcessRouteSnapshotMissingOnResin {
  state: "missingOnResin";
  process: string;
  target_port: number;
  /** Ticket 17: Unix seconds of first in-process drift; undefined while fresh. */
  divergent_since?: number;
  acknowledged: boolean;
}
export type ProcessRouteSnapshot = ProcessRouteSnapshotConsistent | ProcessRouteSnapshotMissingOnResin;

export interface AuthoritativeSnapshot {
  strategyVersion: number;
  platforms: StrategySnapshot[];
  ports: PortSnapshot[];
  /** Ticket 17 (ADR-0055 D3): route family; empty when the whitebox has none. */
  routes: ProcessRouteSnapshot[];
  resinReachable: boolean;
  /** Ticket 12 (ADR-0054 §C): Unix seconds when this snapshot was generated. */
  lastCheckedAt: number;
}

const MAX_SNAPSHOT_ENTRIES = 4096;
const SNAPSHOT_STR_MAX = 512;
/** Ticket 12: cap for the per-entity divergent_since / lastCheckedAt timestamps. */
const SNAPSHOT_TS_MAX = 4_102_444_800; // 2100-01-01 UTC, sane ceiling per AGENTS 7.5

function snapStr(v: unknown): string {
  const s = typeof v === "string" ? v : "";
  return s.length > SNAPSHOT_STR_MAX ? s.slice(0, SNAPSHOT_STR_MAX) : s;
}

function snapStrArr(v: unknown): string[] {
  if (!Array.isArray(v)) return [];
  return v.slice(0, 64).map((x) => snapStr(x)).filter((x) => x.length > 0);
}

/** Ticket 12: sanitize an optional Unix-seconds timestamp (undefined passthrough). */
function snapTs(v: unknown): number | undefined {
  if (v === undefined || v === null) return undefined;
  const n = Number(v);
  if (!Number.isFinite(n) || n < 0 || n > SNAPSHOT_TS_MAX) return undefined;
  return n;
}

/** Ticket 12: sanitize the read-side acknowledged flag (strict boolean). */
function snapAck(v: unknown): boolean {
  return v === true;
}

function snapPlatform(v: unknown): StrategySnapshot | null {
  if (!v || typeof v !== "object") return null;
  const r = v as Record<string, unknown>;
  const name = snapStr(r.platform_name);
  const aClass = snapStr(r.a_class);
  const bClass = snapStr(r.b_class);
  switch (r.state) {
    case "consistent":
      return {
        state: "consistent", platform_name: name,
        platform_id: snapStr(r.platform_id),
        regions: snapStrArr(r.regions),
        resin_allocation_policy: snapStr(r.resin_allocation_policy),
        b_class: bClass, a_class: aClass,
        manual_nodes: snapStrArr(r.manual_nodes),
        subscriptions: snapStrArr(r.subscriptions),
        acknowledged: snapAck(r.acknowledged),
      };
    case "divergent":
      return {
        state: "divergent", platform_name: name,
        platform_id: snapStr(r.platform_id),
        whitebox_regions: snapStrArr(r.whitebox_regions),
        resin_regions: snapStrArr(r.resin_regions),
        resin_allocation_policy: snapStr(r.resin_allocation_policy),
        b_class: bClass, a_class: aClass,
        manual_nodes: snapStrArr(r.manual_nodes),
        subscriptions: snapStrArr(r.subscriptions),
        divergent_since: snapTs(r.divergent_since),
        acknowledged: snapAck(r.acknowledged),
      };
    case "missingOnResin":
      return {
        state: "missingOnResin", platform_name: name,
        platform_id: snapStr(r.platform_id),
        regions: snapStrArr(r.regions),
        a_class: aClass, b_class: bClass,
        manual_nodes: snapStrArr(r.manual_nodes),
        subscriptions: snapStrArr(r.subscriptions),
        divergent_since: snapTs(r.divergent_since),
        acknowledged: snapAck(r.acknowledged),
      };
    default:
      return null;
  }
}

function snapPort(v: unknown): PortSnapshot | null {
  if (!v || typeof v !== "object") return null;
  const r = v as Record<string, unknown>;
  const port = Number(r.port);
  if (!Number.isInteger(port) || port < 0 || port > 65535) return null;
  const name = snapStr(r.platform_name);
  const proto = snapStr(r.protocol);
  const account = snapStr(r.account);
  const label = snapStr(r.label);
  const authRequired = r.auth_required === true;
  if (r.state === "consistent") {
    return { state: "consistent", port, platform_name: name, protocol: proto, account, label, enabled: r.enabled === true, auth_required: authRequired, acknowledged: snapAck(r.acknowledged) };
  }
  if (r.state === "missingOnResin") {
    return { state: "missingOnResin", port, platform_name: name, protocol: proto, account, label, auth_required: authRequired, divergent_since: snapTs(r.divergent_since), acknowledged: snapAck(r.acknowledged) };
  }
  return null;
}

/** Ticket 17: sanitize one route snapshot variant (untrusted response). */
function snapRoute(v: unknown): ProcessRouteSnapshot | null {
  if (!v || typeof v !== "object") return null;
  const r = v as Record<string, unknown>;
  const process = snapStr(r.process);
  if (!process) return null;
  const port = Number(r.target_port);
  if (!Number.isInteger(port) || port < 0 || port > 65535) return null;
  if (r.state === "consistent") {
    return { state: "consistent", process, target_port: port, acknowledged: snapAck(r.acknowledged) };
  }
  if (r.state === "missingOnResin") {
    return { state: "missingOnResin", process, target_port: port, divergent_since: snapTs(r.divergent_since), acknowledged: snapAck(r.acknowledged) };
  }
  return null;
}

/** Round 5 T01 (F4): sanitize one subscription reverse-lookup row (untrusted). */
function snapSubscriptionRow(v: unknown): SubscriptionReverseRow | null {
  if (!v || typeof v !== "object") return null;
  const r = v as Record<string, unknown>;
  const name = snapStr(r.name);
  if (!name) return null;
  const count = (x: unknown): number => {
    const n = Number(x);
    return Number.isFinite(n) && n >= 0 && n <= Number.MAX_SAFE_INTEGER ? n : 0;
  };
  return {
    name,
    node_count: count(r.node_count),
    healthy_node_count: count(r.healthy_node_count),
    consumed_by: snapStrArr(r.consumed_by),
    resolvable: r.resolvable === true,
  };
}

function snapSnapshot(v: unknown): AuthoritativeSnapshot {
  const r = (v && typeof v === "object" ? v : {}) as Record<string, unknown>;
  const platformsRaw = Array.isArray(r.platforms) ? r.platforms : [];
  const portsRaw = Array.isArray(r.ports) ? r.ports : [];
  const routesRaw = Array.isArray(r.routes) ? r.routes : [];
  const subscriptionsRaw = Array.isArray(r.subscriptions) ? r.subscriptions : [];
  return {
    strategyVersion: Number(r.strategyVersion) === 1 ? 1 : 0,
    platforms: platformsRaw
      .slice(0, MAX_SNAPSHOT_ENTRIES)
      .map(snapPlatform)
      .filter((x): x is StrategySnapshot => x !== null),
    ports: portsRaw
      .slice(0, MAX_SNAPSHOT_ENTRIES)
      .map(snapPort)
      .filter((x): x is PortSnapshot => x !== null),
    routes: routesRaw
      .slice(0, MAX_SNAPSHOT_ENTRIES)
      .map(snapRoute)
      .filter((x): x is ProcessRouteSnapshot => x !== null),
    subscriptions: subscriptionsRaw
      .slice(0, MAX_SNAPSHOT_ENTRIES)
      .map(snapSubscriptionRow)
      .filter((x): x is SubscriptionReverseRow => x !== null),
    resinReachable: r.resinReachable === true,
    // Ticket 12: untrusted timestamp sanitized to a bounded Unix-seconds
    // number; a malformed value degrades to 0 instead of leaking junk.
    lastCheckedAt: snapTs(r.lastCheckedAt) ?? 0,
  };
}

/// Read back the merged effective configuration in one call. No inputs to
/// validate (read-only, no args); the response is sanitized as untrusted.
export async function ipcAuthoritativeSnapshot(): Promise<AuthoritativeSnapshot> {
  const raw = await invoke("authoritative_snapshot");
  return snapSnapshot(raw);
}

// ---------------------------------------------------------------------------
// Architecture-recovery ticket 14 / ADR-0054 §A: one-way reconcile. The
// preview list is computed FROM data the snapshot pass already reads
// (compute_plan vs live rows); reconcile_now re-runs the serial
// strategy-apply + ports-restore chain (fail-fast) and the view re-pulls
// the snapshot afterwards. One-way: the whitebox always wins; no "accept
// current state" write exists by contract.
// ---------------------------------------------------------------------------

export interface ReconcilePlanPlatform {
  platform: string;
  desired_regions: string[];
  live_regions: string[];
  action: string;
}

export interface ReconcilePlanPort {
  port: number;
  platform: string;
  action: string;
}

export interface ReconcilePlan {
  platforms: ReconcilePlanPlatform[];
  ports: ReconcilePlanPort[];
}

/** Ticket 14: narrow an untrusted reconcile preview into a bounded plan. */
export function snapReconcilePlan(v: unknown): ReconcilePlan {
  const r = (v && typeof v === "object" ? v : {}) as Record<string, unknown>;
  const platformsRaw = Array.isArray(r.platforms) ? r.platforms : [];
  const portsRaw = Array.isArray(r.ports) ? r.ports : [];
  const action = (x: unknown): string =>
    typeof x === "string" && x.length <= 32 ? x : "";
  return {
    platforms: platformsRaw
      .slice(0, MAX_SNAPSHOT_ENTRIES)
      .map((p): ReconcilePlanPlatform | null => {
        if (!p || typeof p !== "object") return null;
        const e = p as Record<string, unknown>;
        return {
          platform: snapStr(e.platform),
          desired_regions: snapStrArr(e.desired_regions),
          live_regions: snapStrArr(e.live_regions),
          action: action(e.action),
        };
      })
      .filter((x): x is ReconcilePlanPlatform => x !== null && x.platform.length > 0),
    ports: portsRaw
      .slice(0, MAX_SNAPSHOT_ENTRIES)
      .map((p): ReconcilePlanPort | null => {
        if (!p || typeof p !== "object") return null;
        const e = p as Record<string, unknown>;
        const port = Number(e.port);
        if (!Number.isInteger(port) || port < 0 || port > 65535) return null;
        return { port, platform: snapStr(e.platform), action: action(e.action) };
      })
      .filter((x): x is ReconcilePlanPort => x !== null),
  };
}

export interface ReconcileReport {
  strategy: StrategyApplyResult;
  portsRestored: number[];
  portsSkipped: number;
}

export async function ipcReconcileNow(): Promise<ReconcileReport> {
  const raw = await invoke<unknown>("reconcile_now");
  // Untrusted response: bounded sanitize before returning to callers (§7.6).
  const r = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  const portsRestored = Array.isArray(r.portsRestored)
    ? r.portsRestored
        .map((x) => Number(x))
        .filter((n) => Number.isInteger(n) && n >= USER_PORT_MIN && n <= USER_PORT_MAX)
        .slice(0, MAX_SNAPSHOT_ENTRIES)
    : [];
  const skip = Number(r.portsSkipped);
  return {
    strategy: (r.strategy ?? { platforms: [] }) as StrategyApplyResult,
    portsRestored,
    portsSkipped: Number.isFinite(skip) && skip >= 0 ? skip : 0,
  };
}

/// Narrow an unknown view-layer object (e.g. cached zustand data) back into
/// a PlatformFull-ish shape without trusting its fields. Exported for
/// TopologyView's canvas mapping so the old cfgRaw merge stays deleted.
export function snapshotPlatformName(p: StrategySnapshot): string {
  return p.platform_name;
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
   | { kind: "InvalidInput"; data: { msg: string; i18n_key: string } }
   | { kind: "Internal"; data: { msg: string; i18n_key: string } };
 
 /** Narrow a thrown/unknown value from invoke() into a typed IpcErr.
  *  Tauri rejects with a string by default; if the Rust side returns
  *  IpcError via serde, Tauri serialises it as a JS object. */
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
  // level is narrowed to the generated union by the runtime guard above.
  return invoke<string>("set_log_level", { level: level as LogLevel });
}

export async function ipcGetLogLevel(): Promise<string> {
  return invoke<string>("get_log_level");
}
