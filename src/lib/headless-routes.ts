/**
 * Headless BFF route table.
 *
 * Maps Tauri command names to the HTTP route the headless reverse-proxy
 * exposes. This module is pure data: the capability registry lives in
 * ./headless-availability.ts and the request guard + fetch dispatch in
 * ./headless-dispatch.ts, so adding a headless route never touches the
 * invoke-wrapper body in lib/ipc.ts.
 */

// --- dual-mode: cmd -> REST route map (ADR-0043 Q2=A) ---
// In the Tauri webview ipc.ts uses the native invoke(). In a plain browser
// (headless npm server) the dispatch falls back to fetch("/api/v1/...")
// which the headless axum reverse-proxy forwards to the local Resin sidecar.
//
// Every route is DECLARATIVE so the three historical defect classes cannot
// recur:
//   - wrong resource path        -> `path` is the real Resin route (R-numbers
//      cite docs/architecture/RESIN_API_COVERAGE.md)
//   - GET args silently dropped  -> `query` / `queryConst`
//   - body contract drift        -> `bodyArg` (unwrap the Tauri arg envelope)
//      and `bodyKeys` (camelCase -> Resin snake_case)
// Commands that have NO reachable HTTP semantics in headless mode are NOT
// absent-by-accident: they are listed in DISABLED_COMMANDS
// (./headless-availability.ts) with a typed reason, and the UI renders an
// explicit disabled state instead of the old runtime `Tauri-only` throw.
export type HttpMethod = "GET" | "POST" | "PATCH" | "PUT" | "DELETE";

export interface HttpRoute {
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
  /** Non-GET: constant fields merged into the body (e.g. two commands share
   *  one restart route and differ only by a `reason` constant). */
  bodyConst?: Record<string, unknown>;
  /** Resin wraps list reads as {items:[...],total,limit,offset}. Set only when
   *  the caller expects a bare array (mirrors the Rust items_arr call sites);
   *  object-returning reads keep the wrapper. */
  unwrapItems?: boolean;
  /** After unwrapping, project each row to this field (platform_list returns
   *  string[] in Tauri mode, so headless must match). */
  project?: string;
}

export const CMD_TO_HTTP: Record<string, HttpRoute> = {
  // --- Platforms (R07 list / R08 create / R11 patch / R12 delete / R20 leases) ---
  platform_add: { method: "POST", path: "/api/v1/platforms" },
  // DELETE + PATCH on the collection: the BFF resolves name->UUID server-side
  // (browser never holds Resin UUIDs; resolve+mutate stay in one process).
  platform_remove: { method: "DELETE", path: "/api/v1/platforms" },
  platform_list: { method: "GET", path: "/api/v1/platforms", unwrapItems: true, project: "name" },
  platform_list_full: { method: "GET", path: "/api/v1/platforms" },
  // PATCH body stays in the IPC (camelCase) shape: the BFF owns the Resin
  // snake_case translation for this route (rewrite_patch_body_snake_case), so
  // the SPA must NOT pre-rename - one place owns the Resin shape. The
  // contract test locks this split: adding bodyKeys here would break it.
  platform_update: { method: "PATCH", path: "/api/v1/platforms" },
  // R08 full-schema create. The Tauri arg is already named `body`; unwrap it.
  platform_create_with_fields: { method: "POST", path: "/api/v1/platforms", bodyArg: "body" },
  // R20: the real resource is /platforms/{id}/leases. The BFF resolves the
  // business name carried in `leases_for` to a UUID (wrong-route fix).
  platform_leases: { method: "GET", path: "/api/v1/platforms", query: { name: "leases_for" }, unwrapItems: true },

  // --- Subscriptions (R25 list / R26 create / R29 delete / R30 refresh) ---
  subscription_add: {
    method: "POST", path: "/api/v1/subscriptions", bodyKeys: {
      updateInterval: "update_interval", defaultPort: "default_port",
    }
  },
  subscription_remove: { method: "DELETE", path: "/api/v1/subscriptions" },
  subscription_list: { method: "GET", path: "/api/v1/subscriptions", unwrapItems: true },
  // R30 is /subscriptions/{id}/actions/refresh: BFF resolves name->UUID.
  subscription_refresh: { method: "POST", path: "/api/v1/subscriptions", query: { name: "refresh" } },

  // --- Nodes (R36 list / R38+R39 probe / R56 pool snapshot) ---
  // list_nodes hard-codes limit=500 in resin_client.rs; headless must match or
  // the node table silently renders a partial view (v1.2 pagination contract).
  node_list: { method: "GET", path: "/api/v1/nodes", queryConst: { limit: "500" } },
  node_pool_snapshot: { method: "GET", path: "/api/v1/metrics/snapshots/node-pool" },
  // kind is "egress" | "latency"; it selects the actions/{verb} segment.
  node_probe: { method: "POST", path: "/api/v1/nodes/{nodeHash}/actions/probe-{kind}" },

  // --- Leases + metrics (R49 / R47 / R53) ---
  lease_map: { method: "GET", path: "/api/v1/metrics/realtime/leases", unwrapItems: true },
  metrics_realtime_throughput: { method: "GET", path: "/api/v1/metrics/realtime/throughput" },
  metrics_probe_history: { method: "GET", path: "/api/v1/metrics/history/probes", query: { from: "from", to: "to" } },

  // --- Request logs (R44 tail / R45 detail / R46 payloads) ---
  // R44 carries 8 filter params + cursor; `limit` is the one the SPA sends today.
  request_log_tail: { method: "GET", path: "/api/v1/request-logs", query: { limit: "limit" }, unwrapItems: true },
  request_log_detail: { method: "GET", path: "/api/v1/request-logs/{logId}" },
  request_log_payloads: { method: "GET", path: "/api/v1/request-logs/{logId}/payloads" },

  // --- Account header rules (R32-R35) ---
  list_account_header_rules: { method: "GET", path: "/api/v1/account-header-rules", query: { keyword: "keyword" } },
  // R33/R35 address the rule by url_prefix in the PATH; the prefix may contain
  // "/" (and %2F must survive), so the SPA sends it in the body and the BFF
  // performs the path rewrite with its own segment encoder.
  put_account_header_rules: { method: "PUT", path: "/api/v1/account-header-rules", bodyKeys: { urlPrefix: "url_prefix" } },
  resolve_account_header_rule: { method: "POST", path: "/api/v1/account-header-rules:resolve" },
  delete_account_header_rule: { method: "DELETE", path: "/api/v1/account-header-rules", bodyKeys: { urlPrefix: "url_prefix" } },

  // --- System config (R03 read / R06 patch) ---
  system_config_get: { method: "GET", path: "/api/v1/system/config" },
  system_config_patch: { method: "PATCH", path: "/api/v1/system/config", bodyArg: "body" },

  // --- Entry ports: BFF-owned L2 desired state (option C). ---
  // Desktop keeps the L2 whitebox as truth; headless initialises the SAME
  // resin-core stores, so these routes are BFF-native (Resin has no /ports
  // resource: its listener face is /api/v1/endpoints, driven as a side effect).
  port_list: { method: "GET", path: "/api/v1/ports" },
  port_suggest: { method: "GET", path: "/api/v1/ports/suggest" },
  port_upsert: {
    method: "PUT", path: "/api/v1/ports/{port}", bodyKeys: {
      platformName: "platform_name", authRequired: "auth_required",
    }
  },
  port_remove: { method: "DELETE", path: "/api/v1/ports/{port}" },
  port_toggle: { method: "PATCH", path: "/api/v1/ports/{port}" },
  port_bind_platform: { method: "PATCH", path: "/api/v1/ports/{port}/platform", bodyKeys: { platformName: "platform_name" } },
  port_running: { method: "GET", path: "/api/v1/ports/running" },
  port_auth_info: { method: "GET", path: "/api/v1/ports/{port}/auth" },
  port_health_check: { method: "GET", path: "/api/v1/ports/{port}/health", query: { protocol: "protocol" } },

  // --- BFF-native shell routes (R11-03, /api/v1/shell/*): commands whose
  // desktop dependence was only the storage root. The headless server opens
  // the SAME resin-core stores at --state-root and calls the same *_impl
  // bodies the Tauri commands call — a native implementation, not a proxy.
  account_add: { method: "POST", path: "/api/v1/shell/accounts" },
  account_bind_ip: { method: "POST", path: "/api/v1/shell/accounts/bind-ip" },
  authoritative_snapshot: { method: "GET", path: "/api/v1/shell/snapshot" },
  backup_create: { method: "POST", path: "/api/v1/shell/backups" },
  // backup_list is a WebDAV PROPFIND — credentials ride in the body, not the
  // L1 store (the command signature takes them inline).
  backup_list: { method: "POST", path: "/api/v1/shell/backups/list" },
  backup_restore: { method: "POST", path: "/api/v1/shell/backups/restore", bodyKeys: { zipName: "zip_name" } },
  backup_upload: { method: "POST", path: "/api/v1/shell/backups/upload", bodyKeys: { zipPath: "zip_path" } },
  check_firewall_status: { method: "GET", path: "/api/v1/shell/firewall" },
  close_all_connections: { method: "POST", path: "/api/v1/shell/sidecar/restart", bodyConst: { reason: "close_all" } },
  config_export: { method: "GET", path: "/api/v1/shell/config/export" },
  config_import: { method: "POST", path: "/api/v1/shell/config/import", bodyArg: "config" },
  get_diag_poll_interval: { method: "GET", path: "/api/v1/shell/settings/diag-poll-interval" },
  get_log_level: { method: "GET", path: "/api/v1/shell/log-level" },
  get_sidecar_logs: { method: "GET", path: "/api/v1/shell/sidecar/logs" },
  get_sidecar_status: { method: "GET", path: "/api/v1/shell/sidecar/status" },
  ip_reputation_snapshot: { method: "GET", path: "/api/v1/shell/ip-reputation" },
  probe_exit_ip: { method: "POST", path: "/api/v1/shell/probe-exit-ip" },
  process_route_add: { method: "POST", path: "/api/v1/shell/process-routes", bodyKeys: { targetPort: "target_port" } },
  process_route_list: { method: "GET", path: "/api/v1/shell/process-routes" },
  process_route_remove: { method: "DELETE", path: "/api/v1/shell/process-routes" },
  reconcile_now: { method: "POST", path: "/api/v1/shell/reconcile" },
  reset_kernel: { method: "POST", path: "/api/v1/shell/sidecar/restart", bodyConst: { reason: "reset_kernel" } },
  set_diag_poll_interval: { method: "PUT", path: "/api/v1/shell/settings/diag-poll-interval", bodyKeys: { intervalMs: "interval_ms" } },
  set_log_level: { method: "PUT", path: "/api/v1/shell/log-level" },
  strategy_apply: { method: "POST", path: "/api/v1/shell/strategy/apply" },
  strategy_backup_list: { method: "GET", path: "/api/v1/shell/strategy/backups" },
  strategy_config_get: { method: "GET", path: "/api/v1/shell/strategy/config" },
  strategy_config_put: { method: "PUT", path: "/api/v1/shell/strategy/config", bodyArg: "config" },
  strategy_platform_regions_set: { method: "PATCH", path: "/api/v1/shell/strategy/regions", bodyKeys: { platformName: "platform_name" } },
  strategy_rollback: { method: "POST", path: "/api/v1/shell/strategy/rollback", bodyKeys: { backupName: "backup_name" } },
  strategy_verify: { method: "POST", path: "/api/v1/shell/strategy/verify", bodyKeys: { platformName: "platform_name", sampleCount: "sample_count" } },
  whitebox_backup_list: { method: "GET", path: "/api/v1/shell/whitebox/backups" },
  whitebox_get: { method: "GET", path: "/api/v1/shell/whitebox" },
  whitebox_reload: { method: "POST", path: "/api/v1/shell/whitebox/reload" },
  whitebox_rollback: { method: "POST", path: "/api/v1/shell/whitebox/rollback", bodyKeys: { backupName: "backup_name" } },
  whitebox_save_network: { method: "PATCH", path: "/api/v1/shell/whitebox/network", bodyArg: "network" },
};
