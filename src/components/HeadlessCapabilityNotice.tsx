import { useTranslation } from "react-i18next";

import {
  allDisabledCommands,
  commandUnavailableReason,
  disabledReasonKey,
  headlessMode,
  unavailableCommands,
} from "../lib/headlessCapability";

// A-006: the SPA must render an EXPLICIT disabled state for the
// commands the headless server cannot serve, instead of letting the call throw
// IpcUnavailableError when the user clicks. These three exports cover the three
// shapes that need it: a view-level notice, the complete capability list, and a
// per-control guard.

const NOTICE_CLS =
  "rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-xs text-amber-900 " +
  "dark:border-amber-700 dark:bg-amber-900/30 dark:text-amber-200";

/** True when the command has no surface in this mode. Use it to disable a
 *  control without needing a hook (availability is static per page load). */
export function commandBlocked(command: string): boolean {
  return commandUnavailableReason(command) !== null;
}

/** Button gating for one command: `{ disabled, title }` ready to spread onto a
 *  <button>. The title carries the reason so a hover explains the block. */
export function useCommandGuard(command: string): {
  disabled: boolean;
  title: string | undefined;
} {
  const { t } = useTranslation();
  const reason = commandUnavailableReason(command);
  return {
    disabled: reason !== null,
    title: reason === null ? undefined : t(disabledReasonKey(reason)),
  };
}

/** View-level notice: lists the headless-unavailable commands this view uses.
 *  Renders nothing in Tauri mode (where every command is reachable) and
 *  nothing when the view only calls mapped commands. */
export function HeadlessCapabilityNotice({
  commands,
}: {
  commands: readonly string[];
}) {
  const { t } = useTranslation();
  const blocked = unavailableCommands(commands);
  if (blocked.length === 0) return null;
  const reasons: string[] = [];
  for (const entry of blocked) {
    if (!reasons.includes(entry.reason)) reasons.push(entry.reason);
  }
  return (
    <div
      role="status"
      data-testid="headless-capability-notice"
      data-blocked-commands={blocked.map((entry) => entry.command).join(" ")}
      data-blocked-count={String(blocked.length)}
      className={NOTICE_CLS}
    >
      <p className="font-medium">{t("ipc.headlessNotice.title")}</p>
      <p className="mt-0.5">{t("ipc.headlessNotice.body")}</p>
      <ul className="mt-1 list-disc space-y-0.5 pl-4">
        {reasons.map((reason) => (
          <li key={reason} data-reason={reason}>
            {t(disabledReasonKey(reason as never))}
          </li>
        ))}
      </ul>
    </div>
  );
}

/** The COMPLETE disabled set, grouped by reason. Mounted once so the disabled
 *  commands are visible even when no view calls them. */
export function HeadlessCapabilityPanel() {
  const { t } = useTranslation();
  const disabled = allDisabledCommands();
  if (!headlessMode() || disabled.length === 0) return null;
  return (
    <div
      data-testid="headless-capability-panel"
      data-command-count={String(disabled.length)}
      className={NOTICE_CLS}
    >
      <p className="font-medium">{t("ipc.headlessNotice.panelTitle")}</p>
      <p className="mt-0.5">{t("ipc.headlessNotice.panelBody")}</p>
      <ul className="mt-1 space-y-0.5">
        {disabled.map((entry) => (
          <li key={entry.command} data-command={entry.command} data-reason={entry.reason}>
            <code className="font-mono">{entry.command}</code>
            <span className="mx-1">-</span>
            <span>{t(disabledReasonKey(entry.reason))}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
