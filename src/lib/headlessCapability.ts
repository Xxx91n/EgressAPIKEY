// Headless capability surface.
//
// The ipc layer already knows which commands have no headless HTTP surface
// (`DISABLED_COMMANDS` + `ipcCommandAvailability`). This module is the thin
// UI-facing face of that knowledge, so a view can ask "is this action
// available here?" and render an EXPLICIT disabled state instead of letting
// the call throw IpcUnavailableError at click time.

import {
  ipcCommandAvailability,
  ipcDisabledCommands,
  ipcIsHeadless,
  type CommandDisabledReason,
} from "./ipc";

/** A command plus the typed reason it has no headless equivalent. */
export interface UnavailableCommand {
  command: string;
  reason: CommandDisabledReason | "unknown";
}

/** null when the command is reachable in the current mode. In Tauri mode every
 *  command is reachable, so this is always null there. */
export function commandUnavailableReason(
  command: string,
): CommandDisabledReason | "unknown" | null {
  const availability = ipcCommandAvailability(command);
  return availability.available ? null : availability.reason;
}

export function isCommandAvailable(command: string): boolean {
  return commandUnavailableReason(command) === null;
}

/** Filter a view's command list down to the ones this mode cannot serve. */
export function unavailableCommands(commands: readonly string[]): UnavailableCommand[] {
  const blocked: UnavailableCommand[] = [];
  for (const command of commands) {
    const reason = commandUnavailableReason(command);
    if (reason !== null) blocked.push({ command, reason });
  }
  return blocked;
}

/** i18n key for a disabled reason; the catalogs carry one per reason. */
export function disabledReasonKey(reason: CommandDisabledReason | "unknown"): string {
  return "ipc.disabled." + reason;
}

/** True when this SPA is the headless (non-Tauri) control plane. */
export function headlessMode(): boolean {
  return ipcIsHeadless();
}

/** The COMPLETE disabled set, independent of which view calls what. */
export function allDisabledCommands(): UnavailableCommand[] {
  return ipcDisabledCommands();
}
