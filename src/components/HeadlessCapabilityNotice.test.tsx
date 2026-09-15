import { describe, it, expect, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";

import {
  HeadlessCapabilityNotice,
  HeadlessCapabilityPanel,
  commandBlocked,
} from "./HeadlessCapabilityNotice";
import { allDisabledCommands } from "../lib/headlessCapability";

// A-006. The shared setup (src/test/setup.ts) stubs the Tauri
// internals, so these specs start in TAURI mode and opt into headless per test.
// That default is deliberate: the notice must be INVISIBLE in the desktop app,
// and "renders nothing" is the assertion that proves it.
function enterHeadless() {
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  delete (window as unknown as { isTauri?: unknown }).isTauri;
}
function enterTauri() {
  (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
  (window as unknown as { isTauri?: unknown }).isTauri = true;
}

describe("headless disabled state (ticket 02 / A-006)", () => {
  afterEach(() => enterTauri());

  it("the notice is invisible in Tauri mode, where every command is reachable", () => {
    render(<HeadlessCapabilityNotice commands={["strategy_apply", "backup_create"]} />);
    expect(screen.queryByTestId("headless-capability-notice")).toBeNull();
  });

  it("names exactly the blocked commands, with one row per distinct reason", () => {
    enterHeadless();
    render(
      <HeadlessCapabilityNotice
        commands={["strategy_apply", "platform_list", "backup_create"]}
      />,
    );
    const notice = screen.getByTestId("headless-capability-notice");
    // platform_list IS mapped, so it must NOT appear as blocked.
    expect(notice.getAttribute("data-blocked-commands")).toBe("strategy_apply backup_create");
    expect(notice.getAttribute("data-blocked-count")).toBe("2");
    const reasons = Array.from(notice.querySelectorAll("li")).map((li) =>
      li.getAttribute("data-reason"),
    );
    expect(reasons).toContain("shell_local_snapshot");
    expect(reasons).toContain("shell_local_backup");
    expect(reasons.length).toBe(2);
  });

  it("renders nothing when a view only calls mapped commands", () => {
    enterHeadless();
    render(<HeadlessCapabilityNotice commands={["platform_list", "node_list"]} />);
    expect(screen.queryByTestId("headless-capability-notice")).toBeNull();
  });

  it("the panel enumerates the COMPLETE disabled set, one row per command", () => {
    enterHeadless();
    const expected = allDisabledCommands();
    // 44 = the round-2 classification (35 mapped + 44 disabled = 79 manifest).
    // If a ticket adds or removes a command, this fails on purpose.
    expect(expected.length).toBe(44);
    render(<HeadlessCapabilityPanel />);
    const panel = screen.getByTestId("headless-capability-panel");
    expect(panel.getAttribute("data-command-count")).toBe(String(44));
    const rows = Array.from(panel.querySelectorAll("li[data-command]"));
    expect(rows.length).toBe(44);
    expect(rows.every((r) => (r.getAttribute("data-reason") ?? "").length > 0)).toBe(true);
    const got = rows.map((r) => r.getAttribute("data-command")).sort();
    expect(got).toEqual(expected.map((entry) => entry.command).sort());
  });

  it("the panel is invisible in Tauri mode", () => {
    render(<HeadlessCapabilityPanel />);
    expect(screen.queryByTestId("headless-capability-panel")).toBeNull();
  });

  it("commandBlocked drives a control disabled state and flips with the mode", () => {
    enterHeadless();
    expect(commandBlocked("strategy_apply")).toBe(true);
    expect(commandBlocked("platform_list")).toBe(false);
    enterTauri();
    expect(commandBlocked("strategy_apply")).toBe(false);
  });
});
