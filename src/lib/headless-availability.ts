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
import CAPABILITIES from "./headless-capabilities.json";

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
  // The diagnostic probe is dev-build only (import.meta.env.DEV is statically
  // replaced by Vite, so the warn block is dead-code-eliminated from prod
  // bundles and can never leak UA/location to end-user consoles).
  if (!hasInternals && !hasIsTauri && typeof console !== "undefined" && import.meta.env.DEV) {
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
 *  the i18n reason key (`ipc.disabled.<reason>`); never a runtime surprise.
 *  R11-03 re-triage (ADR-0071): a command may stay disabled ONLY when it
 *  assumes a desktop environment — everything else must be reachable. */
export type CommandDisabledReason =
  | "desktop_only_tray"
  | "desktop_only_os"
  | "desktop_only_local_path"
  | "desktop_only_transport";

/** Shape of one entry in the shared capability registry. */
interface CapabilityEntry {
  status: "enabled" | "disabled";
  method?: string;
  path?: string;
  reason?: CommandDisabledReason;
  note?: string;
}

/** The disabled registry is DERIVED from ./headless-capabilities.json — the
 *  same document GET /api/v1/capabilities serves verbatim — so the UI, the
 *  dispatch guard and the machine-readable endpoint can never disagree.
 *  (R11-03: was 44 hand-maintained entries; the re-triage promoted every
 *  command whose dependence on the desktop was only a storage root.) */
export const DISABLED_COMMANDS: Record<string, CommandDisabledReason> =
  Object.fromEntries(
    Object.entries(
      (CAPABILITIES as { commands: Record<string, CapabilityEntry> }).commands,
    )
      .filter(([, entry]) => entry.status === "disabled")
      .map(([cmd, entry]) => [
        cmd,
        entry.reason ?? "desktop_only_os",
      ]),
  );

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
