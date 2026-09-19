import { describe, it, expect, beforeEach, afterEach } from "vitest";

/// R11-04: the capability registry is now a standalone module (boundary
/// split out of lib/ipc.ts). Covers the isTauri() mode detector, the
/// DISABLED_COMMANDS registry, the availability probe the UI calls before
/// rendering controls, and the typed error the dispatch guard throws.

import {
  isTauri,
  ipcCommandAvailability,
  ipcDisabledCommands,
  ipcIsHeadless,
  IpcUnavailableError,
  DISABLED_COMMANDS,
  type CommandDisabledReason,
} from "./headless-availability";
import { CMD_TO_HTTP } from "./headless-routes";

const REASONS: readonly CommandDisabledReason[] = [
  "deprecated_noop",
  "shell_local_l1_prefs",
  "shell_local_l2_whitebox",
  "shell_local_snapshot",
  "shell_local_process_route",
  "shell_local_config_transfer",
  "shell_local_backup",
  "shell_local_sidecar",
  "desktop_only_tray",
  "desktop_only_os",
  "desktop_only_local_path",
];

// setup.ts stubs BOTH injection flags so isTauri() is true by default.
// These helpers clear/restore them on globalThis AND window (the existing
// ipc.test.ts dual-mode block does the same — jsdom object identity has
// shifted across vitest versions, so write to both defensively).
type FlagBag = { __TAURI_INTERNALS__?: unknown; isTauri?: unknown };
const bags = (): FlagBag[] => {
  const out: FlagBag[] = [globalThis as unknown as FlagBag];
  if (typeof window !== "undefined" && (window as unknown) !== globalThis) {
    out.push(window as unknown as FlagBag);
  }
  return out;
};

let saved: { internals: unknown; flag: unknown }[] = [];

function enterHeadless(): void {
  saved = bags().map((b) => ({ internals: b.__TAURI_INTERNALS__, flag: b.isTauri }));
  for (const b of bags()) {
    b.__TAURI_INTERNALS__ = undefined;
    b.isTauri = undefined;
  }
}

function restoreFlags(): void {
  const bs = bags();
  for (let i = 0; i < bs.length; i++) {
    bs[i].__TAURI_INTERNALS__ = saved[i]?.internals;
    bs[i].isTauri = saved[i]?.flag;
  }
}

describe("headless-availability: isTauri / ipcIsHeadless", () => {
  beforeEach(enterHeadless);
  afterEach(restoreFlags);

  it("reports headless mode when neither injection flag exists", () => {
    expect(isTauri()).toBe(false);
    expect(ipcIsHeadless()).toBe(true);
  });

  it("reports Tauri mode when __TAURI_INTERNALS__ is present", () => {
    for (const b of bags()) b.__TAURI_INTERNALS__ = {};
    expect(isTauri()).toBe(true);
    expect(ipcIsHeadless()).toBe(false);
  });

  it("reports Tauri mode when only window.isTauri is present", () => {
    for (const b of bags()) b.isTauri = true;
    expect(isTauri()).toBe(true);
    expect(ipcIsHeadless()).toBe(false);
  });
});

describe("headless-availability: ipcCommandAvailability", () => {
  afterEach(restoreFlags);

  it("headless: a routed command is available", () => {
    enterHeadless();
    expect(ipcCommandAvailability("platform_list")).toEqual({ available: true });
  });

  it("headless: a disabled command returns its typed reason", () => {
    enterHeadless();
    expect(ipcCommandAvailability("strategy_apply")).toEqual({
      available: false,
      reason: "shell_local_snapshot",
    });
    expect(ipcCommandAvailability("whitebox_get")).toEqual({
      available: false,
      reason: "shell_local_l2_whitebox",
    });
  });

  it("headless: an unlisted command degrades to reason \"unknown\"", () => {
    enterHeadless();
    expect(ipcCommandAvailability("totally_made_up_cmd")).toEqual({
      available: false,
      reason: "unknown",
    });
  });

  it("Tauri mode: EVERY command is available (even disabled-listed ones)", () => {
    // Keep the setup.ts Tauri stubs in place for this test.
    for (const b of bags()) {
      b.__TAURI_INTERNALS__ = {};
      b.isTauri = true;
    }
    expect(ipcCommandAvailability("strategy_apply")).toEqual({ available: true });
    expect(ipcCommandAvailability("totally_made_up_cmd")).toEqual({ available: true });
  });
});

describe("headless-availability: DISABLED_COMMANDS registry", () => {
  it("every reason is a member of the CommandDisabledReason union", () => {
    for (const [cmd, reason] of Object.entries(DISABLED_COMMANDS)) {
      expect(REASONS.includes(reason), `${cmd} reason`).toBe(true);
    }
  });

  it("no command is both routed (CMD_TO_HTTP) and disabled — the partition is disjoint", () => {
    for (const cmd of Object.keys(CMD_TO_HTTP)) {
      expect(DISABLED_COMMANDS[cmd], `${cmd} in both tables`).toBeUndefined();
    }
  });

  it("ipcDisabledCommands returns the COMPLETE disabled set with typed reasons", () => {
    const rows = ipcDisabledCommands();
    expect(rows).toHaveLength(Object.keys(DISABLED_COMMANDS).length);
    expect(rows.map((r) => r.command).sort()).toEqual(
      Object.keys(DISABLED_COMMANDS).sort(),
    );
    for (const row of rows) {
      expect(row.reason).toBe(DISABLED_COMMANDS[row.command]);
    }
  });
});

describe("headless-availability: IpcUnavailableError", () => {
  it("carries command, typed reason, and the i18n key", () => {
    const err = new IpcUnavailableError("strategy_apply", "shell_local_snapshot");
    expect(err).toBeInstanceOf(Error);
    expect(err.name).toBe("IpcUnavailableError");
    expect(err.command).toBe("strategy_apply");
    expect(err.reason).toBe("shell_local_snapshot");
    expect(err.i18nKey).toBe("ipc.disabled.shell_local_snapshot");
    expect(err.message).toContain("strategy_apply");
    expect(err.message).toContain("shell_local_snapshot");
  });

  it("accepts the \"unknown\" reason for unlisted commands", () => {
    const err = new IpcUnavailableError("mystery_cmd", "unknown");
    expect(err.reason).toBe("unknown");
    expect(err.i18nKey).toBe("ipc.disabled.unknown");
  });
});
