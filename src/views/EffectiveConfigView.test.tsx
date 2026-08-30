import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import { EffectiveConfigView } from "./EffectiveConfigView";

// Mock @tauri-apps/api/core (same pattern as DiagnosticsView.test.tsx; the
// test-file vi.mock overrides the setup.ts default mock for this file).
const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...(args as [string])),
}));

const TS = 1700000000;

function baseSnap(overrides: Record<string, unknown> = {}) {
  return {
    strategyVersion: 1,
    resinReachable: true,
    lastCheckedAt: TS,
    platforms: [],
    ports: [],
    ...overrides,
  };
}

describe("EffectiveConfigView (ticket 13, read-only)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(null);
  });

  it("renders three-state badges and the desired|live comparison", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") {
        return Promise.resolve(baseSnap({
          platforms: [
            { state: "consistent", platform_name: "alpha", platform_id: "a", regions: ["r1"], resin_allocation_policy: "p2c", b_class: "", a_class: "", manual_nodes: [], subscriptions: [], acknowledged: false },
            { state: "divergent", platform_name: "beta", platform_id: "b", whitebox_regions: ["r1", "r2"], resin_regions: ["r1"], resin_allocation_policy: "p2c", b_class: "", a_class: "", manual_nodes: [], subscriptions: [], divergent_since: TS, acknowledged: false },
          ],
          ports: [
            { state: "consistent", port: 1790, platform_name: "alpha", protocol: "http", account: "acc1", label: "l", enabled: true, auth_required: false, acknowledged: false },
          ],
        }));
      }
      return Promise.resolve(null);
    });
    render(<EffectiveConfigView />);
    const alpha = await screen.findByTestId("ec-platform-alpha");
    expect(within(alpha).getByTestId("ec-badge-consistent")).toHaveAttribute("data-state", "consistent");
    const beta = screen.getByTestId("ec-platform-beta");
    expect(within(beta).getByTestId("ec-badge-divergent")).toBeInTheDocument();
    expect(within(beta).getByTestId("ec-divergent-since-beta")).toHaveTextContent(
      new Date(TS * 1000).toLocaleString()
    );
    expect(within(beta).getByText(/r1, r2/)).toBeInTheDocument(); // desired half
    expect(within(beta).getByText(/r1(?!, r2)/)).toBeInTheDocument(); // live half
    const portRow = screen.getByTestId("ec-port-1790");
    expect(within(portRow).getByTestId("ec-badge-consistent")).toBeInTheDocument();
  });

  it("degrades acknowledged entries to the grey known badge", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") {
        return Promise.resolve(baseSnap({
          platforms: [
            { state: "missingOnResin", platform_name: "gamma", platform_id: "g", regions: ["r9"], a_class: "", b_class: "", manual_nodes: [], subscriptions: [], divergent_since: TS, acknowledged: true },
          ],
        }));
      }
      return Promise.resolve(null);
    });
    render(<EffectiveConfigView />);
    const row = await screen.findByTestId("ec-platform-gamma");
    expect(within(row).getByTestId("ec-badge-known")).toHaveAttribute("data-state", "missingOnResin");
    expect(within(row).queryByTestId("ec-badge-missing")).toBeNull();
    // acknowledged rows do not surface the drift timer
    expect(within(row).queryByTestId("ec-divergent-since-gamma")).toBeNull();
  });

  it("shows lastCheckedAt and the sidecar-down notice with a readable whitebox half", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") {
        return Promise.resolve(baseSnap({
          resinReachable: false,
          platforms: [
            { state: "missingOnResin", platform_name: "delta", platform_id: "d", regions: ["r3"], a_class: "", b_class: "", manual_nodes: [], subscriptions: [], acknowledged: false },
          ],
        }));
      }
      return Promise.resolve(null);
    });
    render(<EffectiveConfigView />);
    await screen.findByTestId("ec-platform-delta");
    expect(screen.getByTestId("ec-last-checked")).toHaveTextContent(new Date(TS * 1000).toLocaleString());
    expect(screen.getByTestId("ec-resin-down")).toBeInTheDocument();
    const row = screen.getByTestId("ec-platform-delta");
    // ADR-0051: sidecar-down absence is neutral, not drift.
    expect(within(row).getByTestId("ec-badge-missing")).toHaveAttribute("data-neutral", "true");
    expect(within(row).getByText(/r3/)).toBeInTheDocument();
  });

  it("fetches once on open and re-checks via the manual button", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") return Promise.resolve(baseSnap());
      return Promise.resolve(null);
    });
    render(<EffectiveConfigView />);
    await screen.findByTestId("ec-view");
    const count = () => invokeMock.mock.calls.filter((c: unknown[]) => c[0] === "authoritative_snapshot").length;
    expect(count()).toBe(1);
    fireEvent.click(screen.getByTestId("ec-refresh"));
    await waitFor(() => expect(count()).toBe(2));
  });

  it("shows the empty notice when nothing is configured", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") return Promise.resolve(baseSnap());
      return Promise.resolve(null);
    });
    render(<EffectiveConfigView />);
    await screen.findByTestId("ec-no-entries");
  });

  it("ticket 15: lists whitebox history and opens a confirm dialog with the target timestamp", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") return Promise.resolve(baseSnap());
      if (cmd === "strategy_backup_list") return Promise.resolve([{ file_name: "egressapikey-strategy.json.100.bak", unix_ts: TS, size_bytes: 55 }]);
      if (cmd === "whitebox_backup_list") return Promise.resolve([]);
      return Promise.resolve(null);
    });
    render(<EffectiveConfigView />);
    const row = await screen.findByTestId("ec-history-strategy-" + TS);
    expect(within(row).getByText(new Date(TS * 1000).toLocaleString())).toBeInTheDocument();
    // No dialog before the button is pressed; cancel path closes it.
    expect(screen.queryByTestId("ec-confirm-dialog")).toBeNull();
    fireEvent.click(within(row).getByTestId("ec-rollback-strategy-" + TS));
    const dialog = screen.getByTestId("ec-confirm-dialog");
    expect(within(dialog).getByTestId("ec-confirm-time")).toHaveTextContent(new Date(TS * 1000).toLocaleString());
    fireEvent.click(within(dialog).getByTestId("ec-confirm-cancel"));
    expect(screen.queryByTestId("ec-confirm-dialog")).toBeNull();
    expect(invokeMock).not.toHaveBeenCalledWith("strategy_rollback", expect.anything());
  });

  it("ticket 15: rollback only fires after confirm, then refreshes snapshot and history", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") return Promise.resolve(baseSnap());
      if (cmd === "strategy_backup_list") return Promise.resolve([{ file_name: "egressapikey-strategy.json.100.bak", unix_ts: TS, size_bytes: 55 }]);
      if (cmd === "whitebox_backup_list") return Promise.resolve([]);
      return Promise.resolve(null);
    });
    render(<EffectiveConfigView />);
    const row = await screen.findByTestId("ec-history-strategy-" + TS);
    fireEvent.click(within(row).getByTestId("ec-rollback-strategy-" + TS));
    fireEvent.click(screen.getByTestId("ec-confirm-rollback"));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("strategy_rollback", expect.objectContaining({ backupName: "egressapikey-strategy.json.100.bak" })));
    // Post-rollback re-check: snapshot + history re-pulled (2nd+ fetch).
    await waitFor(() => {
      const snaps = invokeMock.mock.calls.filter((c: unknown[]) => c[0] === "authoritative_snapshot").length;
      expect(snaps).toBeGreaterThanOrEqual(2);
    });
    expect(screen.queryByTestId("ec-confirm-dialog")).toBeNull();
  });

  it("ticket 15: rollback failure surfaces in the view error slot", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") return Promise.resolve(baseSnap());
      if (cmd === "strategy_backup_list") return Promise.resolve([{ file_name: "egressapikey-strategy.json.100.bak", unix_ts: TS, size_bytes: 55 }]);
      if (cmd === "whitebox_backup_list") return Promise.resolve([]);
      if (cmd === "strategy_rollback") return Promise.reject(new Error("error.strategyWriteFail"));
      return Promise.resolve(null);
    });
    render(<EffectiveConfigView />);
    const row = await screen.findByTestId("ec-history-strategy-" + TS);
    fireEvent.click(within(row).getByTestId("ec-rollback-strategy-" + TS));
    fireEvent.click(screen.getByTestId("ec-confirm-rollback"));
    await screen.findByTestId("ec-error");
  });
  it("surfaces snapshot read failures", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") return Promise.reject(new Error("error.resinUnreachable"));
      return Promise.resolve(null);
    });
    render(<EffectiveConfigView />);
    await screen.findByTestId("ec-error");
  });
});

// Ticket 14 / ADR-0054 §A: reconcile preview dialog — confirm / cancel /
// failure paths, re-entry guard, auto re-verify. Preview data comes from
// the snapshot in memory (no extra request besides the re-verify pull).
describe("EffectiveConfigView reconcile (ticket 14)", () => {
  const driftedSnap = () =>
    baseSnap({
      platforms: [
        {
          state: "divergent",
          platform_name: "beta",
          platform_id: "b",
          whitebox_regions: ["hk"],
          resin_regions: ["us"],
          resin_allocation_policy: "p2c",
          b_class: "random",
          a_class: "region",
          manual_nodes: [],
          subscriptions: [],
          acknowledged: false,
        },
      ],
      ports: [
        {
          state: "missingOnResin",
          port: 17990,
          platform_name: "beta",
          protocol: "socks5",
          account: "",
          label: "",
          auth_required: false,
          acknowledged: false,
        },
      ],
    });

  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") return Promise.resolve(driftedSnap());
      // ticket-15 backup list mocks default to empty
      return Promise.resolve([]);
    });
  });

  it("preview dialog shows the snapshot-derived plan; confirm runs reconcile_now then re-verifies", async () => {
    render(<EffectiveConfigView />);
    await screen.findByTestId("ec-platform-beta");
    fireEvent.click(screen.getByTestId("ec-reconcile"));
    const dialog = await screen.findByTestId("ec-preview-dialog");
    expect(dialog).toBeInTheDocument();
    expect(screen.getByTestId("ec-preview-platform-beta")).toHaveTextContent("beta");
    expect(screen.getByTestId("ec-preview-port-17990")).toBeInTheDocument();

    invokeMock.mockClear();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "reconcile_now") {
        return Promise.resolve({ strategy: { platforms: [] }, portsRestored: [17990], portsSkipped: 0 });
      }
      if (cmd === "authoritative_snapshot") return Promise.resolve(driftedSnap());
      return Promise.resolve([]);
    });
    fireEvent.click(screen.getByTestId("ec-preview-confirm"));
    await waitFor(() => expect(invokeMock.mock.calls.some((c) => c[0] === "reconcile_now")).toBe(true));
    // auto re-verify: the snapshot is re-pulled after success
    await waitFor(() => {
      const pulls = invokeMock.mock.calls.filter((c: unknown[]) => c[0] === "authoritative_snapshot").length;
      expect(pulls).toBeGreaterThanOrEqual(1);
    });
    // dialog closes after success
    await waitFor(() => expect(screen.queryByTestId("ec-preview-dialog")).toBeNull());
  });

  it("cancel closes the dialog with zero IPC side effects", async () => {
    render(<EffectiveConfigView />);
    await screen.findByTestId("ec-platform-beta");
    fireEvent.click(screen.getByTestId("ec-reconcile"));
    await screen.findByTestId("ec-preview-dialog");
    invokeMock.mockClear();
    fireEvent.click(screen.getByTestId("ec-preview-cancel"));
    await waitFor(() => expect(screen.queryByTestId("ec-preview-dialog")).toBeNull());
    // zero IPC of ANY kind during cancel
    expect(invokeMock.mock.calls).toHaveLength(0);
  });

  it("failure surfaces the i18n-mapped error and keeps the dialog open", async () => {
    render(<EffectiveConfigView />);
    await screen.findByTestId("ec-platform-beta");
    fireEvent.click(screen.getByTestId("ec-reconcile"));
    await screen.findByTestId("ec-preview-dialog");
    invokeMock.mockClear();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "reconcile_now") {
        return Promise.reject({ kind: "Internal", data: { msg: "boom", i18n_key: "error.internal" } });
      }
      if (cmd === "authoritative_snapshot") return Promise.resolve(driftedSnap());
      return Promise.resolve([]);
    });
    fireEvent.click(screen.getByTestId("ec-preview-confirm"));
    await screen.findByTestId("ec-error");
    // failure keeps the dialog open for retry
    expect(screen.getByTestId("ec-preview-dialog")).toBeInTheDocument();
    // and the snapshot was still re-pulled (auto re-verify shows latest state)
    await waitFor(() => expect(invokeMock.mock.calls.some((c: unknown[]) => c[0] === "authoritative_snapshot")).toBe(true));
  });

  it("disables the confirm button while in flight (no re-entry)", async () => {
    let resolveReconcile: (v: unknown) => void = () => {};
    render(<EffectiveConfigView />);
    await screen.findByTestId("ec-platform-beta");
    fireEvent.click(screen.getByTestId("ec-reconcile"));
    await screen.findByTestId("ec-preview-dialog");
    invokeMock.mockClear();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "reconcile_now") {
        return new Promise((res) => { resolveReconcile = res; });
      }
      if (cmd === "authoritative_snapshot") return Promise.resolve(driftedSnap());
      return Promise.resolve([]);
    });
    fireEvent.click(screen.getByTestId("ec-preview-confirm"));
    // while the first reconcile is pending, the confirm button is disabled
    await waitFor(() => expect(screen.getByTestId("ec-preview-confirm")).toBeDisabled());
    resolveReconcile({ strategy: { platforms: [] }, portsRestored: [], portsSkipped: 0 });
    await waitFor(() => expect(screen.queryByTestId("ec-preview-dialog")).toBeNull());
  });

  it("shows nothing-to-do state and disables the button for an empty plan", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") return Promise.resolve(baseSnap());
      return Promise.resolve([]);
    });
    render(<EffectiveConfigView />);
    await screen.findByTestId("ec-view");
    const btn = screen.getByTestId("ec-reconcile");
    expect(btn).toBeDisabled();
    fireEvent.click(btn);
    // dialog cannot open from a disabled plan-less state
    expect(screen.queryByTestId("ec-preview-dialog")).toBeNull();
  });
});
