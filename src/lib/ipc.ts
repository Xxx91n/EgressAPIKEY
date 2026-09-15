/**
 * Re3 IPC bridge: typed wrappers over Tauri commands exposing the Resin
 * Platform/Account registry. TS-layer validation per AGENTS s7.6.
 */
import { invoke as _invoke, Channel } from "@tauri-apps/api/core";
// (tauri-specta pilot): type contract comes from the
// specta-generated src/bindings.ts. TYPE-ONLY by design: the generated
// runtime wrappers bypass this module's trace_id injection and the headless
// CMD_TO_HTTP dual-mode, so runtime calls stay on invoke().
import type { LogLevel, PortMapping } from "../bindings";

// --- dual-mode: isTauri detection + cmd → REST route map (ADR-0043 Q2=A) ---
// In the Tauri webview we use the native invoke(). In a plain browser
// (headless npm server) we fall back to fetch("/api/v1/...") which the
// headless axum reverse-proxy forwards to the local Resin sidecar.
function isTauri(): boolean {
  if (typeof window === "undefined") return false;
  // check ALL Tauri v2 injection variables.
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
//
// Every route is DECLARATIVE so the three historical defect classes cannot
// recur:
//   - wrong resource path        -> `path` is the real Resin route (R-numbers
//      cite docs/architecture/RESIN_API_COVERAGE.md)
//   - GET args silently dropped  -> `query` / `queryConst`
//   - body contract drift        -> `bodyArg` (unwrap the Tauri arg envelope)
//      and `bodyKeys` (camelCase -> Resin snake_case)
// Commands that have NO reachable HTTP semantics in headless mode are NOT
// absent-by-accident: they are listed in DISABLED_COMMANDS below with a typed
// reason, and the UI renders an explicit disabled state instead of the old
// runtime `Tauri-only` throw.
type HttpMethod = "GET" | "POST" | "PATCH" | "PUT" | "DELETE";

interface HttpRoute {
  method: HttpMethod;
  /** Path template; `{argName}` placeholders are filled from the Tauri args
   *  and URL-encoded as a single path segment. */
  path: string;
  /** GET only: Tauri arg key -> query-param name. Mirrors the Rust command's
   *  own parameter surface, so a dropped arg is a visible omission. */
  query?: Record<string, string>;
  /** GET only: constant query params the Rust command hard-codes
   *  (e.g. list_nodes always sends limit=500, resin_client.rs). */
  queryConst?: Record<string, string>;
  /** Non-GET: the Tauri arg that carries the real Resin body. Unwrapped so
   *  Resin never receives the {"body":{...}} envelope. */
  bodyArg?: string;
  /** Non-GET: camelCase -> Resin snake_case key renames applied to the body. */
  bodyKeys?: Record<string, string>;
  /** Resin wraps list reads as {items:[...],total,limit,offset}. Set only when
   *  the caller expects a bare array (mirrors the Rust items_arr call sites);
   *  object-returning reads keep the wrapper. */
  unwrapItems?: boolean;
  /** After unwrapping, project each row to this field (platform_list returns
   *  string[] in Tauri mode, so headless must match). */
  project?: string;
}

const CMD_TO_HTTP: Record<string, HttpRoute> = {
  // --- Platforms (R07 list / R08 create / R11 patch / R12 delete / R20 leases) ---
  platform_add:            { method: "POST",   path: "/api/v1/platforms" },
  // DELETE + PATCH on the collection: the BFF resolves name->UUID server-side
  // (browser never holds Resin UUIDs; resolve+mutate stay in one process).
  platform_remove:         { method: "DELETE", path: "/api/v1/platforms" },
  platform_list:           { method: "GET",    path: "/api/v1/platforms", unwrapItems: true, project: "name" },
  platform_list_full:      { method: "GET",    path: "/api/v1/platforms" },
  // PATCH body stays in the IPC (camelCase) shape: the BFF owns the Resin
  // snake_case translation for this route (rewrite_patch_body_snake_case), so
  // the SPA must NOT pre-rename - one place owns the Resin shape.
  // The TS-side bodyKeys here was REVERTED, because the pre-existing
  // contract test proved the SPA/BFF split was intentional, not an oversight.
  platform_update:         { method: "PATCH",  path: "/api/v1/platforms" },
  // R08 full-schema create. The Tauri arg is already named `body`; unwrap it.
  platform_create_with_fields: { method: "POST", path: "/api/v1/platforms", bodyArg: "body" },
  // R20: the real resource is /platforms/{id}/leases. The BFF resolves the
  // business name carried in `leases_for` to a UUID (wrong-route fix).
  platform_leases:         { method: "GET",    path: "/api/v1/platforms", query: { name: "leases_for" }, unwrapItems: true },

  // --- Subscriptions (R25 list / R26 create / R29 delete / R30 refresh) ---
  subscription_add:        { method: "POST",   path: "/api/v1/subscriptions", bodyKeys: {
    updateInterval: "update_interval", defaultPort: "default_port",
  } },
  subscription_remove:     { method: "DELETE", path: "/api/v1/subscriptions" },
  subscription_list:       { method: "GET",    path: "/api/v1/subscriptions", unwrapItems: true },
  // R30 is /subscriptions/{id}/actions/refresh: BFF resolves name->UUID.
  subscription_refresh:    { method: "POST",   path: "/api/v1/subscriptions", query: { name: "refresh" } },

  // --- Nodes (R36 list / R38+R39 probe / R56 pool snapshot) ---
  // list_nodes hard-codes limit=500 in resin_client.rs; headless must match or
  // the node table silently renders a partial view (v1.2 pagination contract).
  node_list:               { method: "GET",    path: "/api/v1/nodes", queryConst: { limit: "500" } },
  node_pool_snapshot:      { method: "GET",    path: "/api/v1/metrics/snapshots/node-pool" },
  // kind is "egress" | "latency"; it selects the actions/{verb} segment.
  node_probe:              { method: "POST",   path: "/api/v1/nodes/{nodeHash}/actions/probe-{kind}" },

  // --- Leases + metrics (R49 / R47 / R53) ---
  lease_map:               { method: "GET",    path: "/api/v1/metrics/realtime/leases", unwrapItems: true },
  metrics_realtime_throughput: { method: "GET", path: "/api/v1/metrics/realtime/throughput" },
  metrics_probe_history:   { method: "GET",    path: "/api/v1/metrics/history/probes", query: { from: "from", to: "to" } },

  // --- Request logs (R44 tail / R45 detail / R46 payloads) ---
  // R44 carries 8 filter params + cursor; `limit` is the one the SPA sends today.
  request_log_tail:        { method: "GET",    path: "/api/v1/request-logs", query: { limit: "limit" }, unwrapItems: true },
  request_log_detail:      { method: "GET",    path: "/api/v1/request-logs/{logId}" },
  request_log_payloads:    { method: "GET",    path: "/api/v1/request-logs/{logId}/payloads" },

  // --- Account header rules (R32-R35) ---
  list_account_header_rules: { method: "GET",  path: "/api/v1/account-header-rules", query: { keyword: "keyword" } },
  // R33/R35 address the rule by url_prefix in the PATH; the prefix may contain
  // "/" (and %2F must survive), so the SPA sends it in the body and the BFF
  // performs the path rewrite with its own segment encoder.
  put_account_header_rules:  { method: "PUT",  path: "/api/v1/account-header-rules", bodyKeys: { urlPrefix: "url_prefix" } },
  resolve_account_header_rule: { method: "POST", path: "/api/v1/account-header-rules:resolve" },
  delete_account_header_rule:  { method: "DELETE", path: "/api/v1/account-header-rules", bodyKeys: { urlPrefix: "url_prefix" } },

  // --- System config (R03 read / R06 patch) ---
  system_config_get:       { method: "GET",    path: "/api/v1/system/config" },
  system_config_patch:     { method: "PATCH",  path: "/api/v1/system/config", bodyArg: "body" },

  // --- Entry ports: BFF-owned L2 desired state (option C). ---
  // Desktop keeps the L2 whitebox as truth; headless initialises the SAME
  // resin-core stores, so these routes are BFF-native (Resin has no /ports
  // resource: its listener face is /api/v1/endpoints, driven as a side effect).
  port_list:               { method: "GET",    path: "/api/v1/ports" },
  port_suggest:            { method: "GET",    path: "/api/v1/ports/suggest" },
  port_upsert:             { method: "PUT",    path: "/api/v1/ports/{port}", bodyKeys: {
    platformName: "platform_name", authRequired: "auth_required",
  } },
  port_remove:             { method: "DELETE", path: "/api/v1/ports/{port}" },
  port_toggle:             { method: "PATCH",  path: "/api/v1/ports/{port}" },
  port_bind_platform:      { method: "PATCH",  path: "/api/v1/ports/{port}/platform", bodyKeys: { platformName: "platform_name" } },
  port_running:            { method: "GET",    path: "/api/v1/ports/running" },
  port_auth_info:          { method: "GET",    path: "/api/v1/ports/{port}/auth" },
  port_health_check:       { method: "GET",    path: "/api/v1/ports/{port}/health", query: { protocol: "protocol" } },
};

/** Why a command has no headless surface. Drives the UI disabled state and
 *  the i18n reason key (`ipc.disabled.<reason>`); never a runtime surprise. */
export type CommandDisabledReason =
  | "deprecated_noop"
  | "shell_local_l1_prefs"
  | "shell_local_l2_whitebox"
  | "shell_local_snapshot"
  | "shell_local_process_route"
  | "shell_local_config_transfer"
  | "shell_local_backup"
  | "shell_local_sidecar"
  | "desktop_only_tray"
  | "desktop_only_os"
  | "desktop_only_local_path"

/** 43 commands with no reachable HTTP semantics in headless mode (
 *  see .scratch/architecture-recovery/reports/02-headless-adapter-report.md).
 *  Grouped by WHY, so a future change of circumstance has one place to edit. */
const DISABLED_COMMANDS: Record<string, CommandDisabledReason> = {
  // A-020: echo commands kept per AGENTS 7.6; removal condition not triggered.
  account_add: "deprecated_noop",
  account_bind_ip: "deprecated_noop",
  // L1 GUI preferences (tauri-plugin-store).
  lightweight_get: "shell_local_l1_prefs",
  lightweight_set: "shell_local_l1_prefs",
  get_diag_poll_interval: "shell_local_l1_prefs",
  set_diag_poll_interval: "shell_local_l1_prefs",
  set_log_level: "shell_local_l1_prefs",
  get_log_level: "shell_local_l1_prefs",
  ip_reputation_snapshot: "shell_local_l1_prefs",
  // L2 whitebox files (ports + strategy) - present in headless only for the
  // port CRUD subset exposed above; the raw file surface stays desktop-only.
  whitebox_get: "shell_local_l2_whitebox",
  whitebox_path: "shell_local_l2_whitebox",
  whitebox_reload: "shell_local_l2_whitebox",
  whitebox_save_network: "shell_local_l2_whitebox",
  whitebox_backup_list: "shell_local_l2_whitebox",
  whitebox_rollback: "shell_local_l2_whitebox",
  strategy_config_get: "shell_local_l2_whitebox",
  strategy_config_put: "shell_local_l2_whitebox",
  strategy_platform_regions_set: "shell_local_l2_whitebox",
  strategy_backup_list: "shell_local_l2_whitebox",
  strategy_rollback: "shell_local_l2_whitebox",
  // Cross-store snapshot / reconcile (L2 + egressapikey.db + L3 merge).
  strategy_verify: "shell_local_snapshot",
  strategy_apply: "shell_local_snapshot",
  authoritative_snapshot: "shell_local_snapshot",
  reconcile_now: "shell_local_snapshot",
  // Process routes live in egressapikey-ports.json (ADR-0055).
  process_route_add: "shell_local_process_route",
  process_route_remove: "shell_local_process_route",
  process_route_list: "shell_local_process_route",
  // ADR-0061: whitebox-source transfer, no headless HTTP surface.
  config_export: "shell_local_config_transfer",
  config_import: "shell_local_config_transfer",
  // Local zip + WebDAV backup (L1 credentials).
  backup_create: "shell_local_backup",
  backup_upload: "shell_local_backup",
  backup_list: "shell_local_backup",
  // Desktop shell sidecar lifecycle.
  get_sidecar_status: "shell_local_sidecar",
  get_sidecar_logs: "shell_local_sidecar",
  close_all_connections: "shell_local_sidecar",
  reset_kernel: "shell_local_sidecar",
  // Tray / OS / streaming surfaces.
  tray_refresh_labels: "desktop_only_tray",
  check_firewall_status: "desktop_only_os",
  probe_exit_ip: "desktop_only_os",
  watch_port_health: "desktop_only_tray",
  // Local filesystem paths / exports.
  get_config_dir: "desktop_only_local_path",
  get_log_dir: "desktop_only_local_path",
  export_audit_log: "desktop_only_local_path",
};

/** Thrown when a headless-mode call targets a command with no HTTP surface.
 *  The UI checks `ipcCommandAvailability` BEFORE calling, so reaching this is a
 *  programming error - but it stays typed so no raw string ever surfaces. */
export class IpcUnavailableError extends Error {
  readonly command: string;
  readonly reason: CommandDisabledReason | "unknown";
  readonly i18nKey: string;
  constructor(command: string, reason: CommandDisabledReason | "unknown") {
    super(`[ipc] ${command} is unavailable in headless mode (${reason})`);
    this.name = "IpcUnavailableError";
    this.command = command;
    this.reason = reason;
    this.i18nKey = `ipc.disabled.${reason}`;
  }
}

/** Headless availability probe for one command. The UI uses this to render an
 *  explicit disabled state (no more runtime Tauri-only throw).
 *  In Tauri mode every command is available. */
export function ipcCommandAvailability(
  cmd: string,
): { available: true } | { available: false; reason: CommandDisabledReason | "unknown" } {
  if (isTauri()) return { available: true };
  if (CMD_TO_HTTP[cmd]) return { available: true };
  return { available: false, reason: DISABLED_COMMANDS[cmd] ?? "unknown" };
}

/** True when the SPA is running in the headless browser control plane. */
export function ipcIsHeadless(): boolean {
  return !isTauri();
}

/** Every command with no headless HTTP surface, paired with its typed reason
 * Exposed so the UI capability panel can render the
 *  COMPLETE disabled set: a command must not be invisible just because no
 *  view happens to call it, and the user must be able to see WHY it is gone
 *  rather than discovering it at click time. */
export function ipcDisabledCommands(): { command: string; reason: CommandDisabledReason }[] {
  return Object.entries(DISABLED_COMMANDS).map(([command, reason]) => ({ command, reason }));
}

/** Fill `{arg}` placeholders from the Tauri args as URL-encoded segments. */
function buildPath(route: HttpRoute, args?: Record<string, unknown>): string {
  return route.path.replace(/\{([A-Za-z0-9_]+)\}/g, (_m, key: string) => {
    const v = args?.[key];
    if (v === undefined || v === null) {
      throw new Error(`[ipc] missing path arg "${key}" for ${route.method} ${route.path}`);
    }
    return encodeURIComponent(String(v));
  });
}

/** GET query string: constant params first, then the declared arg passthrough. */
function buildQuery(route: HttpRoute, args?: Record<string, unknown>): string {
  const qs = new URLSearchParams();
  for (const [k, v] of Object.entries(route.queryConst ?? {})) qs.set(k, v);
  for (const [argKey, param] of Object.entries(route.query ?? {})) {
    const v = args?.[argKey];
    if (v === undefined || v === null) continue;
    qs.set(param, String(v));
  }
  const s = qs.toString();
  return s ? `?${s}` : "";
}

/** Non-GET body: unwrap the Tauri arg envelope, rename camelCase keys, and
 *  drop nulls (the "only provided fields" contract Resin and the BFF share). */
function buildBody(route: HttpRoute, args?: Record<string, unknown>): string | undefined {
  if (route.method === "GET" || !args) return undefined;
  let payload: Record<string, unknown> = args;
  if (route.bodyArg) {
    const inner = args[route.bodyArg];
    if (inner === undefined || inner === null) return undefined;
    if (typeof inner !== "object" || Array.isArray(inner)) {
      throw new Error(`[ipc] ${route.bodyArg} must be a JSON object`);
    }
    payload = inner as Record<string, unknown>;
  }
  if (route.bodyKeys) {
    const renamed: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(payload)) {
      if (v === null || v === undefined) continue;
      renamed[route.bodyKeys[k] ?? k] = v;
    }
    payload = renamed;
  }
  return JSON.stringify(payload);
}

async function invokeHttp<T>(route: HttpRoute, args?: Record<string, unknown>): Promise<T> {
  const init: RequestInit = {
    method: route.method,
    headers: { "Content-Type": "application/json" },
  };
  const body = buildBody(route, args);
  if (body !== undefined) init.body = body;
  const url = `${buildPath(route, args)}${buildQuery(route, args)}`;
  const r = await fetch(url, init);
  if (!r.ok) {
    const text = await r.text().catch(() => "");
    throw new Error(`IPC ${route.method} ${url} -> ${r.status}: ${text.slice(0, 256)}`);
  }
  if (r.status === 204) return undefined as T;
  const json = await r.json();
  // T22: Resin wraps list reads as { items: [...], total, limit, offset }.
  // Unwrap ONLY where the caller expects a bare array - an object-returning
  // read (metrics_probe_history carries bucket_seconds beside items) must keep
  // the wrapper, otherwise the sibling fields are silently dropped.
  if (route.unwrapItems && json && typeof json === "object" && Array.isArray((json as { items?: unknown }).items)) {
    const items = (json as { items: unknown[] }).items;
    if (route.project) {
      return items.map((it) => (it as Record<string, unknown>)[route.project as string]) as T;
    }
    return items as T;
  }
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

/** Single dispatch point. Tauri webview -> native invoke(); plain browser
 *  (headless) -> fetch() against the declarative route table above. A command
 *  with neither a route nor a DISABLED_COMMANDS entry raises a typed
 *  IpcUnavailableError instead of the former opaque Tauri-only string. */
async function invoke<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const traceId = genTraceId();
  if (typeof console !== "undefined" && console.debug) {
    console.debug(`[trace_id=${traceId}] ipc.${cmd}`);
  }
  if (!isTauri()) {
    const route = CMD_TO_HTTP[cmd];
    if (!route) {
      throw new IpcUnavailableError(cmd, DISABLED_COMMANDS[cmd] ?? "unknown");
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

/// Re-export from strategy.ts — shell 6-option is the sole UI source of truth.
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
  // translate shell StrategyId → Resin enum before sending to backend.
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
/// accepts shell StrategyId, translates to Resin enum internally.
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

/**
 * @deprecated since ADR-0050: account semantics are owned by the Resin
 * sidecar; the shell command only validates input and echoes. Kept for IPC
 * contract compatibility — no in-repo caller (see AGENTS.md §7.6 echo list).
 */
export async function ipcAccountBindIp(platform: string, account: string, ip: string): Promise<boolean> {
  const meta = import.meta as { env?: { DEV?: boolean } };
  if (meta.env?.DEV) {
    console.warn("[ipc] account_bind_ip is deprecated since ADR-0050 (Resin owns account semantics)");
  }
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

/**
 * pipeline="establish" opts into the backend
 * five-step cascade (resolve -> whitebox platform -> strategy apply).
 * Absent/undefined = the legacy import-only POST (no cascade).
 */
export async function ipcSubscriptionAdd(
  name: string,
  url: string,
  updateInterval?: string,
  pipeline?: "establish",
  defaultPort?: number,
): Promise<void> {
  assertShortName(name, "subscription");
  if (!url || url.length > 4096) throw new Error("subscription url invalid");
  if (!/^https?:\/\//.test(url)) throw new Error("subscription url must start with http:// or https://");
  if (pipeline !== undefined && pipeline !== "establish") {
    throw new Error("pipeline must be \"establish\" when provided");
  }
  // optional explicit binding target for the
  // cascade's default-port tail. Provided => the backend skips the suggest
  // probe; same range discipline as the port wrappers (§7.5 mirrored).
  if (defaultPort !== undefined) assertPort(defaultPort);
  await invoke("subscription_add", {
    name,
    url,
    updateInterval: updateInterval ?? null,
    pipeline: pipeline ?? null,
    defaultPort: defaultPort ?? null,
  });
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

/// refresh a subscription. Resin POST /actions/refresh is
/// synchronously blocking but its response body is empty, so the shell
/// re-queries /subscriptions up to 5 x 500ms and returns the actual
/// post-refresh node_count plus a changed flag derived from node_count /
/// node_version drift. The UI uses changed to distinguish "no upstream
/// diff" from a stale closure read.
export interface SubscriptionRefreshResult {
  node_count: number;
  changed: boolean;
}
export async function ipcSubscriptionRefresh(name: string): Promise<SubscriptionRefreshResult> {
  assertShortName(name, "subscription");
  return invoke<SubscriptionRefreshResult>("subscription_refresh", { name });
}

/// on-demand per-node probe. kind is "egress" or "latency". Returns
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


/// Backup: create the configuration archive and return its path.
/// `passphrase` (non-empty) wraps the archive in the AEAD envelope, so a
/// restore needs the same passphrase to open it.
export async function ipcBackupCreate(passphrase?: string): Promise<string> {
  return invoke<string>("backup_create", { passphrase: passphrase && passphrase.length > 0 ? passphrase : null });
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

/** Restore summary returned by the backend. The backend verifies the manifest
 *  and every member hash BEFORE it writes, so a failure here means nothing was
 *  applied. `evidence` lists the L3 / audit members that were extracted as
 *  read-only evidence instead of being written back. */
export interface BackupRestoreSummary {
  configRestored: boolean;
  settingsRestored: boolean;
  platforms: number;
  ports: number;
  portsRestored: number;
  evidence: string[];
  evidenceDir: string;
  errors: string[];
}

/** Backup: download a listed archive from WebDAV and restore it through the
 *  authoritative write entries (the same path config_import uses). */
export async function ipcBackupRestore(
  url: string,
  username: string,
  password: string,
  zipName: string,
  passphrase?: string,
): Promise<BackupRestoreSummary> {
  if (!url || url.length > 2048) throw new Error("webdav url invalid");
  if (!/^https?:\/\//.test(url)) throw new Error("webdav url must start with http:// or https://");
  if (!zipName || zipName.includes("/") || !zipName.endsWith(".zip")) throw new Error("backup name invalid");
  return invoke<BackupRestoreSummary>("backup_restore", {
    url,
    username,
    password,
    zipName,
    passphrase: passphrase && passphrase.length > 0 ? passphrase : null,
  });
}

// ---- Process routing (ADR-0055: L2 whitebox family) ----
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

// ── Account header rules (ADR-0063): Resin-side HTTP-header
// routing family, R32-R35. Coexists with the process_route_* family —
// see docs/architecture/PROCESS_ROUTE_VS_HEADER_RULES.md.

/** §7.5 shared TS gate: DNS-host style cap (253) + reject control chars. */
function assertRuleString(v: string, field: string): void {
  if (!v || v.trim().length === 0 || v.length > 253 || /[\u0000-\u001f\u007f]/.test(v)) {
    throw new Error(`${field} invalid (1..253 chars, no control chars)`);
  }
}

/** GET /api/v1/account-header-rules (R32) — raw items-wrapper passthrough. */
export async function ipcListAccountHeaderRules(
  keyword?: string,
): Promise<unknown> {
  if (keyword !== undefined) {
    if (keyword.length > 253 || /[\u0000-\u001f\u007f]/.test(keyword)) {
      throw new Error("keyword invalid (<=253 chars, no control chars)");
    }
  }
  return invoke("list_account_header_rules", { keyword });
}

/** PUT /api/v1/account-header-rules/{prefix} (R33) — upsert one rule. */
export async function ipcPutAccountHeaderRules(
  urlPrefix: string,
  headers: string[],
): Promise<unknown> {
  assertRuleString(urlPrefix, "url_prefix");
  if (!Array.isArray(headers) || headers.length === 0 || headers.length > 64) {
    throw new Error("headers must be a non-empty array (1..64)");
  }
  for (const h of headers) {
    assertRuleString(h, "header");
  }
  return invoke("put_account_header_rules", { urlPrefix, headers });
}

/** POST /api/v1/account-header-rules:resolve (R34) — matcher debug aid. */
export async function ipcResolveAccountHeaderRule(url: string): Promise<unknown> {
  if (!url || url.length > 2048 || /[\u0000-\u001f\u007f]/.test(url)) {
    throw new Error("url invalid (1..2048 chars, no control chars)");
  }
  // §7.6 URL convention: absolute http(s) only (same as subscription_add).
  if (!/^https?:\/\//i.test(url)) {
    throw new Error("url must be an absolute http(s) URL");
  }
  return invoke("resolve_account_header_rule", { url });
}

/** DELETE /api/v1/account-header-rules/{prefix} (R35) — remove one rule. */
export async function ipcDeleteAccountHeaderRule(urlPrefix: string): Promise<unknown> {
  assertRuleString(urlPrefix, "url_prefix");
  return invoke("delete_account_header_rule", { urlPrefix });
}

/// ADR-0061: export the L2 whitebox config (strategy + ports).
export async function ipcConfigExport(): Promise<unknown> {
  return invoke("config_export");
}

/// ADR-0061: import a whitebox config document. The backend
/// validates schema + version and writes through the whitebox stores (never
/// a direct Resin PATCH), then triggers reconcile. Returns a summary.
export async function ipcConfigImport(config: unknown): Promise<{
  platforms_created: number;
  platforms_skipped: number;
  subscriptions_created: number;
  subscriptions_skipped: number;
  ports_created: number;
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
  // the closed three-value set, `mixed` the default.
  const protocol = (m.protocol || "mixed").toLowerCase();
  if (protocol !== "socks5" && protocol !== "http" && protocol !== "mixed") {
    throw new Error("protocol must be socks5, http or mixed");
  }
  // platform_name empty = unbound port (ADR-0029). Allow empty.
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

/// (ADR-0042): Toggle enabled flag on an entry-port without
/// touching any other field. Patches the Resin endpoint `{enabled: bool}`
/// and persists the flag into the shell whitebox. The entry-port's other
/// fields (protocol, platform_name, account, auth_required) are preserved.
export async function ipcPortToggle(port: number, enabled: boolean): Promise<PortMapping> {
  assertPort(port);
  return invoke<PortMapping>("port_toggle", { port, enabled });
}

/// (ADR-0029): Bind a port to a platform without touching auth_required.
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

/// ADR-0026 Q9: protocol-aware health probe. For socks5 and mixed ports the
/// Rust side sends a SOCKS5 greeting (a mixed listener answers it - verified
/// live, / ADR-0068 D4 gate); for http ports it sends an
/// HTTP CONNECT probe. Defaults to "mixed", the default port protocol.
export async function ipcPortHealthCheck(port: number, protocol?: string): Promise<PortHealthCheck> {
  if (port < USER_PORT_MIN || port > USER_PORT_MAX) throw new Error(`port ${port} out of range (${USER_PORT_MIN}..${USER_PORT_MAX})`);
  const proto = (protocol ?? "mixed").toLowerCase();
  if (proto !== "socks5" && proto !== "http" && proto !== "mixed") throw new Error("protocol must be socks5, http or mixed");
  return invoke<PortHealthCheck>("port_health_check", { port, protocol: proto });
}

// Phase 1: streaming port health. One Tauri Channel<PortHealthSnapshot>
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
  // In non-Tauri (headless browser) mode, Channel constructor and
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

// Exit IP probe — routes a request to 1.1.1.1/cdn-cgi/trace through the
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
  if (proto !== "socks5" && proto !== "http" && proto !== "mixed") throw new Error("protocol must be socks5, http or mixed");
  return invoke<ExitIpProbe>("probe_exit_ip", { port, protocol: proto });
}

// Firewall status check (Windows-only, read-only).
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
  /** Resin row UUID — key for the detail drawer; "" on old wire shapes. */
  id: string;
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

// single request-log entry + captured payloads for the
// DiagnosticsView detail drawer (RESIN_API_COVERAGE #R45/#R46). The row id
// rides request_log_tail rows as of this ticket; §7.5 mirrors the Rust
// validate_log_id (1..=64 chars, UUID hex+hyphen charset) and the payload
// wire face is untrusted — coerced before render. Upstream returns
// base64-encoded bodies ({req_headers_b64, req_body_b64, resp_headers_b64,
// resp_body_b64, truncated{req_headers,req_body,resp_headers,resp_body}},
// handler_requestlog.go:355-368) and 200-with-empty-strings when payload
// logging is off.

/** §7.5: mirror of the Rust validate_log_id — same UUID charset + 64 cap. */
export function assertLogId(logId: string): void {
  if (typeof logId !== "string" || logId.length === 0 || logId.length > 64) {
    throw new Error("log_id must be a non-empty string of at most 64 chars");
  }
  if (!/^[0-9a-fA-F-]+$/.test(logId)) {
    throw new Error("log_id must be a UUID (hex digits and hyphens only)");
  }
}

export interface RequestLogDetail {
  id: string;
  ts: string;
  proxy_type: number;
  client_ip: string;
  platform_id: string;
  platform_name: string;
  account: string;
  target_host: string;
  target_url: string;
  node_hash: string;
  node_tag: string;
  egress_ip: string;
  duration_ms: number;
  first_byte_duration_ms: number;
  net_ok: boolean;
  http_method: string;
  http_status: number;
  resin_error: string;
  ingress_bytes: number;
  egress_bytes: number;
  payload_present: boolean;
  req_body_len: number;
  resp_body_len: number;
}

export interface RequestLogPayloads {
  req_headers_b64: string;
  req_body_b64: string;
  resp_headers_b64: string;
  resp_body_b64: string;
  truncated: {
    req_headers: boolean;
    req_body: boolean;
    resp_headers: boolean;
    resp_body: boolean;
  };
}

const b64 = (v: unknown): string => (typeof v === "string" ? v : "");
const bl = (v: unknown): boolean => v === true;
const num = (v: unknown): number => (typeof v === "number" && Number.isFinite(v) ? v : 0);
const str = (v: unknown): string => (typeof v === "string" ? v : "");

export async function ipcRequestLogDetail(logId: string): Promise<RequestLogDetail> {
  assertLogId(logId);
  const raw = (await invoke<unknown>("request_log_detail", { logId })) as {
    [k: string]: unknown;
  } | null;
  const r = raw && typeof raw === "object" ? raw : {};
  // Coerce the untrusted wire face; unknown fields are dropped.
  return {
    id: str(r.id),
    ts: str(r.ts),
    proxy_type: num(r.proxy_type),
    client_ip: str(r.client_ip),
    platform_id: str(r.platform_id),
    platform_name: str(r.platform_name),
    account: str(r.account),
    target_host: str(r.target_host),
    target_url: str(r.target_url),
    node_hash: str(r.node_hash),
    node_tag: str(r.node_tag),
    egress_ip: str(r.egress_ip),
    duration_ms: num(r.duration_ms),
    first_byte_duration_ms: num(r.first_byte_duration_ms),
    net_ok: bl(r.net_ok),
    http_method: str(r.http_method),
    http_status: num(r.http_status),
    resin_error: str(r.resin_error),
    ingress_bytes: num(r.ingress_bytes),
    egress_bytes: num(r.egress_bytes),
    payload_present: bl(r.payload_present),
    req_body_len: num(r.req_body_len),
    resp_body_len: num(r.resp_body_len),
  };
}

/** 1 MB display cap for one decoded payload part (issue F2). */
export const PAYLOAD_DISPLAY_CAP_BYTES = 1024 * 1024;

/** Decode one upstream base64 payload part to UTF-8 text for display.
 *  Invalid/empty input decodes to "" (payload logging off upstream);
 *  parts over the cap are sliced BEFORE decode so a multi-MB body never
 *  reaches the DOM — the flag drives the truncation label. */
export function decodePayloadPart(
  b64: unknown,
  capBytes: number = PAYLOAD_DISPLAY_CAP_BYTES,
): { text: string; bytes: number; displayTruncated: boolean } {
  if (typeof b64 !== "string" || b64.length === 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(b64)) {
    return { text: "", bytes: 0, displayTruncated: false };
  }
  let bytes: Uint8Array;
  try {
    const bin = atob(b64);
    bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
  } catch {
    return { text: "", bytes: 0, displayTruncated: false };
  }
  if (bytes.byteLength > capBytes) {
    return {
      text: new TextDecoder("utf-8", { fatal: false }).decode(bytes.slice(0, capBytes)),
      bytes: bytes.byteLength,
      displayTruncated: true,
    };
  }
  return {
    text: new TextDecoder("utf-8", { fatal: false }).decode(bytes),
    bytes: bytes.byteLength,
    displayTruncated: false,
  };
}

export async function ipcRequestLogPayloads(logId: string): Promise<RequestLogPayloads> {
  assertLogId(logId);
  const raw = (await invoke<unknown>("request_log_payloads", { logId })) as {
    [k: string]: unknown;
  } | null;
  const r = raw && typeof raw === "object" ? raw : {};
  const t = r.truncated && typeof r.truncated === "object"
    ? (r.truncated as { [k: string]: unknown })
    : {};
  return {
    req_headers_b64: b64(r.req_headers_b64),
    req_body_b64: b64(r.req_body_b64),
    resp_headers_b64: b64(r.resp_headers_b64),
    resp_body_b64: b64(r.resp_body_b64),
    truncated: {
      req_headers: bl(t.req_headers),
      req_body: bl(t.req_body),
      resp_headers: bl(t.resp_headers),
      resp_body: bl(t.resp_body),
    },
  };
}

// (ADR-0064): Resin metrics minimal set — realtime throughput
// (#R47) + probe history (#R53). Pull model (no WebSocket push). from/to are
// RFC3339 strings at this boundary (upstream handler_metrics.go:15 parses
// time.RFC3339Nano); validated here first, then re-validated at the Rust
// command boundary (§7.5 dual cover). The Rust response is untrusted wire
// data: items are coerced to an array before use.
export interface MetricsThroughputPoint {
  ts: string;
  ingress_bps: number;
  egress_bps: number;
}

export interface MetricsThroughput {
  step_seconds?: number;
  items: MetricsThroughputPoint[];
}

export interface MetricsProbeBucket {
  bucket_start: string;
  bucket_end: string;
  total_count: number;
}

export interface MetricsProbeHistory {
  bucket_seconds?: number;
  items: MetricsProbeBucket[];
}

const METRICS_RFC3339_RE =
  /^\d{4}-\d{2}-\d{2}[Tt]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:?\d{2})$/;
const METRICS_MAX_WINDOW_MS = 7 * 24 * 3600 * 1000;
const METRICS_FUTURE_SKEW_MS = 5 * 60 * 1000;

function assertMetricsTimestamp(v: string, field: string): void {
  if (v.length > 64) throw new Error(`metrics '${field}' exceeds 64 chars`);
  if (!METRICS_RFC3339_RE.test(v)) {
    throw new Error(`metrics '${field}' must be an RFC3339 timestamp`);
  }
}

export async function ipcMetricsRealtimeThroughput(): Promise<MetricsThroughput> {
  const res = await invoke<MetricsThroughput>("metrics_realtime_throughput");
  const items = (res as { items?: unknown } | null)?.items;
  return {
    step_seconds: (res as { step_seconds?: number } | null)?.step_seconds,
    items: Array.isArray(items) ? (items as MetricsThroughputPoint[]) : [],
  };
}

export async function ipcMetricsProbeHistory(
  from?: string,
  to?: string,
): Promise<MetricsProbeHistory> {
  if (from !== undefined) assertMetricsTimestamp(from, "from");
  if (to !== undefined) assertMetricsTimestamp(to, "to");
  if (from !== undefined && to !== undefined) {
    const f = Date.parse(from);
    const t = Date.parse(to);
    if (!(f < t)) throw new Error("metrics 'from' must be before 'to'");
    if (t - f > METRICS_MAX_WINDOW_MS) {
      throw new Error("metrics window exceeds 7 days");
    }
  } else if (from !== undefined) {
    if (Date.now() - Date.parse(from) > METRICS_MAX_WINDOW_MS) {
      throw new Error("metrics window exceeds 7 days");
    }
  }
  if (to !== undefined && Date.parse(to) - Date.now() > METRICS_FUTURE_SKEW_MS) {
    throw new Error("metrics 'to' is in the future");
  }
  const args: { from?: string; to?: string } = {};
  if (from !== undefined) args.from = from;
  if (to !== undefined) args.to = to;
  const res = await invoke<MetricsProbeHistory>("metrics_probe_history", args);
  const items = (res as { items?: unknown } | null)?.items;
  return {
    bucket_seconds: (res as { bucket_seconds?: number } | null)?.bucket_seconds,
    items: Array.isArray(items) ? (items as MetricsProbeBucket[]) : [],
  };
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
  /** (ADR-0054 §D): optional exemption list (decimal port numbers). */
  acknowledged?: string[];
}

export async function ipcWhiteboxPath(): Promise<string> {
  return invoke<string>("whitebox_path");
}

/** sanitize a whitebox acknowledged exemption array (§7.6). */
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

/** Save network-layer config (DNS + idle + probe + bypass) to whitebox JSON. */
export function ipcWhiteboxSaveNetwork(network: NetworkConfig): Promise<number> {
  return invoke<number>("whitebox_save_network", { network });
}

/**
 * (ADR-0054 section B): one versioned backup copy of a whitebox
 * file. file_name is a server-generated `<original>.<unixts>[-N].bak` name - it
 * is NEVER interpolated into any URL or passed raw to another command; the
 * rollback wrappers validate the shape before invoking (section 7.6).
 */
export interface WhiteboxBackupEntry {
  file_name: string;
  unix_ts: number;
  size_bytes: number;
}

/** (section 7.6): a backup name must look like <name>.<digits>[-N].bak. */
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

/** list the versioned backups of the ports whitebox (newest first). */
export async function ipcWhiteboxBackupList(): Promise<WhiteboxBackupEntry[]> {
  return snapBackupEntries(await invoke("whitebox_backup_list"));
}

/**
 * roll the ports whitebox back to a listed backup. The backend
 * re-enters the validate-before-swap -> apply chain and reconciles L3; the
 * response is the restored entry-port count.
 */
export async function ipcWhiteboxRollback(backupName: string): Promise<number> {
  assertBackupName(backupName);
  const n = await invoke<number>("whitebox_rollback", { backupName });
  return Number(n) || 0;
}

/** list the versioned backups of the strategy whitebox (newest first). */
export async function ipcStrategyBackupList(): Promise<WhiteboxBackupEntry[]> {
  return snapBackupEntries(await invoke("strategy_backup_list"));
}

/** roll the strategy whitebox back to a listed backup (backend re-validates + re-applies). */
export async function ipcStrategyRollback(backupName: string): Promise<unknown> {
  assertBackupName(backupName);
  return invoke("strategy_rollback", { backupName });
}


export interface SidecarStatus {
  api_port: number;
  api_base: string;
  mode: string;
  /** sidecar process PID (0 if not running). */
  pid: number;
  /** RFC3339 timestamp of the last successful /healthz probe. */
  healthz_last_check: string;
  /** round-trip latency of the get_sidecar_status IPC call (microseconds). */
  ipc_latency_us: number;
}

export async function ipcGetSidecarStatus(): Promise<SidecarStatus> {
  return invoke<SidecarStatus>("get_sidecar_status");
}


// ---------------------------------------------------------------------------
// Strategy Engine (ADR-0022) — whitebox per-platform strategy config.
// ---------------------------------------------------------------------------

/// the former `BClassParams` interface
/// mirrored the Rust struct of the same name — four display-only B-class
/// "parameters" that no backend ever read. Both are withdrawn: Resin accepts
/// exactly one B-class knob (`allocation_policy`). A legacy document that still
/// carries `b_class_params` keeps loading; the key is simply ignored.

export interface PlatformStrategy {
  platform_name: string;
  a_class: "manual" | "region" | "quality" | "subscription";
  /// Desired Resin `allocation_policy`, stored verbatim
  /// (BALANCED | PREFER_LOW_LATENCY | PREFER_IDLE_IP). Legacy six-option shell
  /// tokens still deserialize on the Rust side and are rewritten to this
  /// spelling by the one-time migration.
  b_class: string;
  manual_nodes?: string[];
  regions?: string[];
  subscriptions?: string[];
  top_n?: number;
}

export async function ipcGetConfigDir(): Promise<string> {
  return invoke<string>("get_config_dir");
}

export interface AuditExportResult {
  exported_to: string;
  bytes: number;
  rows: number;
}

/**
 * ADR-0059: export the append-only audit log to the path
 * returned by the native save dialog (Settings > Storage "Export audit log").
 * Validates the path at the TS boundary (non-empty, 1..4096 chars, no control
 * chars) before invoking — AGENTS §7.5 validate-then-invoke contract.
 */
export async function ipcExportAuditLog(targetPath: string): Promise<AuditExportResult> {
  if (targetPath.length === 0 || targetPath.length > 4096) {
    throw new Error("target_path must be 1..4096 chars");
  }
  if (/[\u0000-\u001f\u007f]/.test(targetPath)) {
    throw new Error("target_path must not contain control characters");
  }
  return invoke<AuditExportResult>("export_audit_log", { targetPath });
}

export interface StrategyConfig {
  version: number;
  platforms: PlatformStrategy[];
  /** (ADR-0054 §D): optional exemption list (platform names). */
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
  // (§7.6): exemption list is optional; when present it must be a
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

/// (ADR-0052): deep edit — set one platform's region list through
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
// authoritative effective-config snapshot
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
  /** (ADR-0054 §D): read-side exemption flag; never influences the merge. */
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
  /** (ADR-0054 §C): Unix seconds of first in-process drift; undefined while fresh. */
  divergent_since?: number;
  /** (ADR-0054 §D): read-side exemption flag. */
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
  /** (ADR-0054 §C): Unix seconds of first in-process drift. */
  divergent_since?: number;
  /** (ADR-0054 §D): read-side exemption flag. */
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
  /** (ADR-0054 §D): read-side exemption flag. */
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
  /** (ADR-0054 §C): Unix seconds of first in-process drift. */
  divergent_since?: number;
  /** (ADR-0054 §D): read-side exemption flag. */
  acknowledged: boolean;
}
export type PortSnapshot = PortSnapshotConsistent | PortSnapshotMissingOnResin;

/** (ADR-0055 D3): per-route three-state. The live side of a route
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
  /** Unix seconds of first in-process drift; undefined while fresh. */
  divergent_since?: number;
  acknowledged: boolean;
}
export type ProcessRouteSnapshot = ProcessRouteSnapshotConsistent | ProcessRouteSnapshotMissingOnResin;

/** (F4): per-subscription reverse lookup — the Gateway
 *  API attachedRoutes analog. consumed_by lists the whitebox platforms whose
 *  `subscriptions` array names this subscription; empty = unbound. A row
 *  with resolvable=false means the whitebox references a subscription Resin
 *  no longer reports (dangling). */
export interface SubscriptionReverseRow {
  name: string;
  node_count: number;
  healthy_node_count: number;
  consumed_by: string[];
  resolvable: boolean;
}

/** establish-cascade phase of ONE subscription, as
 *  recorded in the strategy whitebox (STATUS, not spec). Wire tags mirror
 *  the Rust enum's PascalCase variant serialization (ConvergePhase style). */
export type SubscriptionPhase =
  | "Never"
  | "Importing"
  | "Establishing"
  | "Converged"
  | "Failed"
  | "NeedsApproval";

/** Cascade sub-step tag (Rust `EstablishStep`; snake_case on the wire).
 *  import/resolve appear only on Failed rows; port is reserved for. */
export type SubscriptionStage = "import" | "resolve" | "platform" | "bind" | "port" | "apply";

const SUBSCRIPTION_PHASES: readonly SubscriptionPhase[] = [
  "Never",
  "Importing",
  "Establishing",
  "Converged",
  "Failed",
  "NeedsApproval",
];
const SUBSCRIPTION_STAGES: readonly SubscriptionStage[] = [
  "import",
  "resolve",
  "platform",
  "bind",
  "port",
  "apply",
];

/** one subscription's persisted cascade-failure
 *  compensation record (Rust `CascadeError`; schema LOCKED to
 *  `{ stage, reason, rollback_actions[] }` — one ordered marking entry per
 *  cascade step, never a dynamic object). */
export interface CascadeErrorInfo {
  stage: SubscriptionStage;
  reason: string;
  rollback_actions?: string[];
}

/** one whitebox phase status row (snake_case row fields, the
 *  per-variant wire convention; the parent field is camelCase). */
export interface SubscriptionPhaseRow {
  name: string;
  phase: SubscriptionPhase;
  stage?: SubscriptionStage;
  phase_error?: string;
  last_cascade_error?: CascadeErrorInfo;
}

/** sanitize one cascade-failure record (untrusted). Any
 *  malformed member drops the WHOLE record — a half-valid failure marking
 *  would read as data that was never persisted. */
function snapCascadeError(v: unknown): CascadeErrorInfo | undefined {
  if (!v || typeof v !== "object" || Array.isArray(v)) return undefined;
  const r = v as Record<string, unknown>;
  const stageTag = typeof r.stage === "string" ? r.stage : "";
  if (!(SUBSCRIPTION_STAGES as readonly string[]).includes(stageTag)) return undefined;
  const reason = typeof r.reason === "string" ? r.reason.trim().slice(0, 1024) : "";
  if (!reason) return undefined;
  let actions: string[] | undefined;
  if (Array.isArray(r.rollback_actions)) {
    const cleaned = r.rollback_actions
      .filter((a): a is string => typeof a === "string")
      .map((a) => a.replace(/[\u0000-\u001f\u007f]/g, " ").trim().slice(0, 256));
    actions = cleaned.length > 0 ? cleaned : undefined;
  }
  return {
    stage: stageTag as SubscriptionStage,
    reason,
    ...(actions !== undefined ? { rollback_actions: actions } : {}),
  };
}

/** sanitize one phase row (untrusted). A malformed phase
 *  degrades to "Never" — the state machine's identity element, honest
 *  "nothing recorded" — never to Converged/Failed (no fake green/red). */
function snapSubscriptionPhaseRow(v: unknown): SubscriptionPhaseRow | null {
  if (!v || typeof v !== "object") return null;
  const r = v as Record<string, unknown>;
  const name = snapStr(r.name);
  if (!name) return null;
  const phaseTag = typeof r.phase === "string" ? r.phase : "";
  const phase = (SUBSCRIPTION_PHASES as readonly string[]).includes(phaseTag)
    ? (phaseTag as SubscriptionPhase)
    : "Never";
  const stageTag = typeof r.stage === "string" ? r.stage : "";
  const stage = (SUBSCRIPTION_STAGES as readonly string[]).includes(stageTag)
    ? (stageTag as SubscriptionStage)
    : undefined;
  const phaseError =
    r.phase_error === undefined || r.phase_error === null ? undefined : snapStr(r.phase_error) || undefined;
  const cascadeError =
    r.last_cascade_error === undefined || r.last_cascade_error === null
      ? undefined
      : snapCascadeError(r.last_cascade_error);
  if (phase === "Never" && stageTag === "" && phaseError === undefined && cascadeError === undefined) {
    return { name, phase };
  }
  return {
    name,
    phase,
    ...(stage !== undefined ? { stage } : {}),
    ...(phaseError !== undefined ? { phase_error: phaseError } : {}),
    ...(cascadeError !== undefined ? { last_cascade_error: cascadeError } : {}),
  };
}

export interface AuthoritativeSnapshot {
  strategyVersion: number;
  platforms: StrategySnapshot[];
  ports: PortSnapshot[];
  /** (ADR-0055 D3): route family; empty when the whitebox has none. */
  routes: ProcessRouteSnapshot[];
  /** (F4): subscription reverse lookup; empty when Resin is
   *  unreachable and the whitebox references nothing. */
  subscriptions: SubscriptionReverseRow[];
  /** per-subscription establish-phase STATUS rows (whitebox
   *  projections); empty when no cascade has ever recorded a phase. */
  subscriptionPhases?: SubscriptionPhaseRow[];
  resinReachable: boolean;
  /** (ADR-0054 §C): Unix seconds when this snapshot was generated. */
  lastCheckedAt: number;
  /** (ADR-0058): whitebox write-authority generation + observed
   *  (last green apply) generation and the derived convergence phase. */
  strategyGeneration: number;
  strategyAppliedGeneration: number;
  convergePhase: ConvergePhase;
  /** Unix seconds of the last green apply pass; undefined = never green. */
  lastApplyAt?: number;
  /** Reason of the last not-green apply pass; undefined when green/never. */
  lastApplyError?: string;
}

/** (ADR-0058): top-level convergence phase. The Rust enum
 *  serializes in its PascalCase variant form over the wire; mirrored here. */
export type ConvergePhase =
  | "NeverApplied"
  | "PendingApply"
  | "ApplyFailed"
  | "Converged"
  | "Drifted"
  | "Unknown";

const CONVERGE_PHASES: readonly ConvergePhase[] = [
  "NeverApplied",
  "PendingApply",
  "ApplyFailed",
  "Converged",
  "Drifted",
  "Unknown",
];

const MAX_SNAPSHOT_ENTRIES = 4096;
const SNAPSHOT_STR_MAX = 512;
/** cap for the per-entity divergent_since / lastCheckedAt timestamps. */
const SNAPSHOT_TS_MAX = 4_102_444_800; // 2100-01-01 UTC, sane ceiling per AGENTS 7.5

function snapStr(v: unknown): string {
  const s = typeof v === "string" ? v : "";
  return s.length > SNAPSHOT_STR_MAX ? s.slice(0, SNAPSHOT_STR_MAX) : s;
}

function snapStrArr(v: unknown): string[] {
  if (!Array.isArray(v)) return [];
  return v.slice(0, 64).map((x) => snapStr(x)).filter((x) => x.length > 0);
}

/** sanitize an optional Unix-seconds timestamp (undefined passthrough). */
function snapTs(v: unknown): number | undefined {
  if (v === undefined || v === null) return undefined;
  const n = Number(v);
  if (!Number.isFinite(n) || n < 0 || n > SNAPSHOT_TS_MAX) return undefined;
  return n;
}

/** sanitize the read-side acknowledged flag (strict boolean). */
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

/** sanitize one route snapshot variant (untrusted response). */
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

/** (F4): sanitize one subscription reverse-lookup row (untrusted). */
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
    subscriptionPhases: (Array.isArray(r.subscriptionPhases) ? r.subscriptionPhases : [])
      .slice(0, MAX_SNAPSHOT_ENTRIES)
      .map(snapSubscriptionPhaseRow)
      .filter((x): x is SubscriptionPhaseRow => x !== null),
    resinReachable: r.resinReachable === true,
    // untrusted timestamp sanitized to a bounded Unix-seconds
    // number; a malformed value degrades to 0 instead of leaking junk.
    lastCheckedAt: snapTs(r.lastCheckedAt) ?? 0,
    // (ADR-0058): untrusted generation counters + phase. A
    // malformed counter degrades to 0; a malformed phase degrades to
    // "Unknown" (honest: we do not know what the wire said).
    strategyGeneration: snapCounter(r.strategyGeneration),
    strategyAppliedGeneration: snapCounter(r.strategyAppliedGeneration),
    convergePhase: ((): ConvergePhase => {
      const s = typeof r.convergePhase === "string" ? r.convergePhase : "";
      return (CONVERGE_PHASES as readonly string[]).includes(s)
        ? (s as ConvergePhase)
        : "Unknown";
    })(),
    lastApplyAt: snapTs(r.lastApplyAt),
    lastApplyError: r.lastApplyError === undefined || r.lastApplyError === null ? undefined : snapStr(r.lastApplyError) || undefined,
  };
}

/** bounded non-negative integer counter (generation fields). */
function snapCounter(v: unknown): number {
  const n = Number(v);
  return Number.isFinite(n) && n >= 0 && n <= Number.MAX_SAFE_INTEGER ? n : 0;
}

/// Read back the merged effective configuration in one call. No inputs to
/// validate (read-only, no args); the response is sanitized as untrusted.
export async function ipcAuthoritativeSnapshot(): Promise<AuthoritativeSnapshot> {
  const raw = await invoke("authoritative_snapshot");
  return snapSnapshot(raw);
}

// ---------------------------------------------------------------------------
// ADR-0054 §A: one-way reconcile. The
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

/** narrow an untrusted reconcile preview into a bounded plan. */
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
   /** name→UUID lookup miss (error.notFound locale key). */
   | { kind: "NotFound"; data: { msg: string; i18n_key: string } }
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
       case "InvalidInput":
         return {
           kind: "InvalidInput",
           data: {
             msg: String(data?.msg ?? ""),
             i18n_key: String(data?.i18n_key ?? ""),
           },
         };
       case "NotFound":
         // name→UUID lookup miss keeps its i18n_key so the GUI
         // renders the locale "Not found" line instead of the Internal text.
         return {
           kind: "NotFound",
           data: {
             msg: String(data?.msg ?? ""),
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
/// Close all in-flight connections (kill + restart sidecar).
export async function ipcCloseAllConnections(): Promise<void> {
  return invoke("close_all_connections");
}

/// Reset kernel (kill + restart sidecar — same impl, different semantic).
export async function ipcResetKernel(): Promise<void> {
  return invoke("reset_kernel");
}

/// Strategy verification — probe N requests, collect exit IP + latency.
export async function ipcStrategyVerify(platformName: string, sampleCount: number): Promise<unknown> {
  assertShortName(platformName, "platform");
  if (typeof sampleCount !== "number" || sampleCount < 3 || sampleCount > 50) {
    throw new Error("sampleCount must be between 3 and 50");
  }
  return invoke("strategy_verify", { platformName, sampleCount });
}

/// get lightweight mode config {enabled, delay_minutes}.
export async function ipcLightweightGet(): Promise<{ enabled: boolean; delay_minutes: number }> {
  try {
    const r = await invoke("lightweight_get") as { enabled: boolean; delay_minutes: number };
    return r;
  } catch {
    // Outside Tauri (vitest) — return defaults
    return { enabled: true, delay_minutes: 10 };
  }
}

/// set lightweight mode config (enabled + delay_minutes).
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
