/**
 * Re3 IPC bridge: typed wrappers over Tauri commands exposing the Resin
 * Platform/Account registry. TS-layer validation per AGENTS s7.6.
 */
import { invoke } from "@tauri-apps/api/core";

const NAME_MAX = 128;

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

/// The allocation policies Resin v1.1.2 actually accepts (must match the Rust
/// ALLOWED_ALLOCATION_POLICIES in commands/mod.rs).
export const ALLOCATION_POLICIES = ["BALANCED", "PREFER_LOW_LATENCY", "PREFER_IDLE_IP"] as const;
export type AllocationPolicy = (typeof ALLOCATION_POLICIES)[number];

/// Phase R1: PATCH a platform's allocation_policy / regex_filters / sticky_ttl.
/// TS-boundary validation mirrors the Rust side (AGENTS s7.6): policy enum,
/// filter count + length, ttl length + control chars. Only provided fields
/// are sent; the Rust side rebuilds the body.
export async function ipcPlatformUpdate(
  name: string,
  allocationPolicy?: AllocationPolicy,
  regexFilters?: string[],
  regionFilters?: string[],
  stickyTtl?: string,
): Promise<unknown> {
  assertShortName(name, "platform");
  if (allocationPolicy !== undefined && !ALLOCATION_POLICIES.includes(allocationPolicy)) {
    throw new Error(`allocation_policy must be one of ${ALLOCATION_POLICIES.join(", ")}`);
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
    allocationPolicy: allocationPolicy ?? null,
    regexFilters: regexFilters ?? null,
    regionFilters: regionFilters ?? null,
    stickyTtl: stickyTtl ?? null,
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
/// Maps the GUI label to the Resin enum: random/sequential -> BALANCED,
/// latency -> PREFER_LOW_LATENCY, quality -> PREFER_IDLE_IP.
export async function ipChannelPolicySet(
  platformName: string,
  policy: AllocationPolicy,
): Promise<unknown> {
  assertShortName(platformName, "platform");
  if (!ALLOCATION_POLICIES.includes(policy)) {
    throw new Error("ip_channel_policy_set: allocation_policy must be one of " + ALLOCATION_POLICIES.join(", "));
  }
  return invoke("platform_update", { name: platformName, allocationPolicy: policy, regexFilters: null, regionFilters: null, stickyTtl: null });
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
  await invoke("tray_refresh_labels").catch(() => {});
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

export async function ipcSubscriptionAdd(name: string, url: string): Promise<void> {
  assertShortName(name, "subscription");
  if (!url || url.length > 4096) throw new Error("subscription url invalid");
  if (!/^https?:\/\//.test(url)) throw new Error("subscription url must start with http:// or https://");
  await invoke("subscription_add", { name, url });
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

export async function ipcPortUpsert(m: {
  port: number;
  protocol: string;
  platform_name: string;
  account?: string;
  label?: string;
  enabled?: boolean;
}): Promise<PortMapping> {
  assertPort(m.port);
  const protocol = (m.protocol || "socks5").toLowerCase();
  if (protocol !== "socks5" && protocol !== "http") {
    throw new Error("protocol must be socks5 or http");
  }
  assertShortName(m.platform_name, "platform_name");
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
  });
}

export async function ipcPortRemove(port: number): Promise<boolean> {
  assertPort(port);
  return invoke<boolean>("port_remove", { port });
}

export async function ipcPortRunning(): Promise<number[]> {
  const raw = await invoke<number[]>("port_running");
  return Array.isArray(raw) ? raw : [];
}

export async function ipcPortReload(): Promise<number> {
  return invoke<number>("port_reload");
}

/// ADR-0021 Q1: SOCKS5 credentials a gateway must present to reach an
/// entry-port. Username is the port's bound Platform.Account string,
/// password is the sidecar global proxy_token. `auth_required` is always
/// true in the thin-shell stack (Resin sets RESIN_PROXY_TOKEN at boot).
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

export async function ipcPortHealthCheck(port: number): Promise<PortHealthCheck> {
  if (port < 1024 || port > 65535) throw new Error(`port ${port} out of range (1024..65535)`);
  return invoke<PortHealthCheck>("port_health_check", { port });
}

export interface WhiteboxConfig {
  version: number;
  entry_ports: PortMapping[];
}

export async function ipcWhiteboxPath(): Promise<string> {
  return invoke<string>("whitebox_path");
}

export async function ipcWhiteboxGet(): Promise<WhiteboxConfig> {
  const raw = await invoke<WhiteboxConfig>("whitebox_get");
  return {
    version: Number(raw?.version ?? 1),
    entry_ports: Array.isArray(raw?.entry_ports) ? raw.entry_ports : [],
  };
}

/** Reload hand-edited egressapikey-ports.json into DB + listeners. */
export async function ipcWhiteboxReload(): Promise<number> {
  return invoke<number>("whitebox_reload");
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
}

export async function ipcGetSidecarStatus(): Promise<SidecarStatus> {
  return invoke<SidecarStatus>("get_sidecar_status");
}


// ---------------------------------------------------------------------------
// Strategy Engine (T4-4 / ADR-0022) — whitebox per-platform strategy config.
// ---------------------------------------------------------------------------

export interface PlatformStrategy {
  platform_name: string;
  a_class: "manual" | "region" | "quality" | "subscription";
  b_class: string; // StrategyId serialised as snake_case
  regions?: string[];
  subscriptions?: string[];
  top_n?: number;
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
