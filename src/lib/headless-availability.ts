/**
 * Headless capability registry.
 *
 * Owns the headless-mode knowledge the UI and the dispatch share:
 *   - isTauri(): which runtime this SPA runs in (Tauri webview with the
 *     native invoke() bridge vs a plain browser pointed at the headless BFF)
 *   - DISABLED_COMMANDS: every command with no reachable headless HTTP
 *     surface, grouped by WHY so a future change of circumstance has one
 *     place to edit
 *   - the availability probe the UI calls to render explicit disabled
 *     states, plus the typed error the dispatch guard throws
 * The cmd -> REST route table lives in ./headless-routes.ts; the request
 * guard + fetch dispatch in ./headless-dispatch.ts. lib/ipc.ts re-exports
 * the public surface so existing callers keep importing from "./ipc".
 */
import { CMD_TO_HTTP } from "./headless-routes";

// --- dual-mode: isTauri detection (ADR-0043 Q2=A) ---
// In the Tauri webview we use the native invoke(). In a plain browser
// (headless npm server) we fall back to fetch("/api/v1/...") which the
// headless axum reverse-proxy forwards to the local Resin sidecar.
export function isTauri(): boolean {
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

/** 44 commands with no reachable HTTP semantics in headless mode
 *  (policy: docs/adr/0071-headless-parity-policy.md).
 *  Grouped by WHY, so a future change of circumstance has one place to edit. */
export const DISABLED_COMMANDS: Record<string, CommandDisabledReason> = {
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
  backup_restore: "shell_local_backup",
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
