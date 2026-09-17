import { describe, it, expect, beforeEach, beforeAll, vi } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import { EffectiveConfigView } from "./EffectiveConfigView";
import i18next from "i18next";
import enCommon from "../locales/en/common.json";

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
    // round9 ticket 02 (A-002): the Consistent row also surfaces the live policy value.
    expect(within(alpha).getByTestId("ec-live-policy-alpha")).toHaveTextContent("p2c");
    const beta = screen.getByTestId("ec-platform-beta");
    expect(within(beta).getByTestId("ec-badge-divergent")).toBeInTheDocument();
    expect(within(beta).getByTestId("ec-divergent-since-beta")).toHaveTextContent(
      new Date(TS * 1000).toLocaleString()
    );
    expect(within(beta).getByText(/r1, r2/)).toBeInTheDocument(); // desired half
    expect(within(beta).getByText(/r1(?!, r2)/)).toBeInTheDocument(); // live half
    // ticket 02 scope is the Consistent row: the Divergent row keeps the region-only layout.
    expect(within(beta).queryByTestId("ec-live-policy-beta")).not.toBeInTheDocument();
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

// ADR-0054 §A: reconcile preview dialog — confirm / cancel /
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
          b_class: "BALANCED",
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
      // backup list mocks default to empty
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

// preview coverage completion — the ports-domain will-change rows
// locked beside the strategy domain (snapshot-derived, zero extra requests)
// and the explicit desired-only wording for missing-on-resin platforms
// (instead of being folded into the divergent patch). The REAL en catalog is
// wired into the test i18n so the assertions lock the shipped wording values,
// not key fallbacks (src/test/setup.ts ships no effectiveConfig.* keys).
describe("EffectiveConfigView reconcile preview coverage (ticket 19)", () => {
  beforeAll(() => {
    i18next.addResourceBundle("en", "translation", { effectiveConfig: enCommon.effectiveConfig }, true, true);
  });

  const coveredSnap = () =>
    baseSnap({
      platforms: [
        {
          state: "divergent",
          platform_name: "beta",
          platform_id: "b",
          whitebox_regions: ["hk"],
          resin_regions: ["us"],
          resin_allocation_policy: "p2c",
          b_class: "BALANCED",
          a_class: "region",
          manual_nodes: [],
          subscriptions: [],
          acknowledged: false,
        },
        {
          state: "missingOnResin",
          platform_name: "gamma",
          platform_id: "",
          regions: ["hk"],
          a_class: "region",
          b_class: "BALANCED",
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
      if (cmd === "authoritative_snapshot") return Promise.resolve(coveredSnap());
      return Promise.resolve([]);
    });
  });

  it("preview lists ports-domain will-change rows beside the strategy domain with zero extra requests", async () => {
    render(<EffectiveConfigView />);
    await screen.findByTestId("ec-platform-beta");
    const snapsBefore = invokeMock.mock.calls.filter((c: unknown[]) => c[0] === "authoritative_snapshot").length;
    fireEvent.click(screen.getByTestId("ec-reconcile"));
    const dialog = await screen.findByTestId("ec-preview-dialog");
    expect(dialog).toBeInTheDocument();
    // Both domains render side by side inside the same dialog.
    expect(screen.getByTestId("ec-preview-platforms")).toBeInTheDocument();
    expect(screen.getByTestId("ec-preview-ports")).toBeInTheDocument();
    const row = screen.getByTestId("ec-preview-port-17990");
    expect(row).toHaveTextContent("17990");
    expect(row).toHaveTextContent(enCommon.effectiveConfig.reconcileCreateEndpoint);
    // Snapshot-derived: opening the preview issued NO new snapshot fetch.
    const snapsAfter = invokeMock.mock.calls.filter((c: unknown[]) => c[0] === "authoritative_snapshot").length;
    expect(snapsAfter).toBe(snapsBefore);
  });

  it("missing-on-resin platform renders the explicit desired-only wording, not the divergent patch", async () => {
    render(<EffectiveConfigView />);
    await screen.findByTestId("ec-platform-gamma");
    fireEvent.click(screen.getByTestId("ec-reconcile"));
    await screen.findByTestId("ec-preview-dialog");
    const gamma = screen.getByTestId("ec-preview-platform-gamma");
    expect(gamma).toHaveTextContent(enCommon.effectiveConfig.reconcileCreatePlatform);
    // Not folded into the divergent patch template (which interpolates "live → desired").
    expect(gamma.textContent).not.toContain("→");
    const beta = screen.getByTestId("ec-preview-platform-beta");
    expect(beta).toHaveTextContent("→");
  });
});

// ADR-0058: the top-level convergence chip. The REAL en
// catalog is wired in so the assertions lock the shipped wording values.
describe("EffectiveConfigView converge chip (round5 T09)", () => {
  beforeAll(() => {
    i18next.addResourceBundle("en", "translation", { effectiveConfig: enCommon.effectiveConfig }, true, true);
  });

  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") return Promise.resolve(baseSnap());
      return Promise.resolve([]);
    });
  });

  it("Converged renders the green chip with apply time and rev", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") {
        return Promise.resolve(baseSnap({
          strategyGeneration: 3,
          strategyAppliedGeneration: 3,
          convergePhase: "Converged",
          lastApplyAt: TS,
        }));
      }
      return Promise.resolve([]);
    });
    render(<EffectiveConfigView />);
    const chip = await screen.findByTestId("ec-converge-chip");
    expect(chip).toHaveAttribute("data-phase", "Converged");
    expect(chip).toHaveTextContent(enCommon.effectiveConfig.convergeConverged
      .replace("{{time}}", new Date(TS * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }))
      .replace("{{rev}}", "3"));
  });

  it("ApplyFailed renders the red chip with the recorded reason", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") {
        return Promise.resolve(baseSnap({
          strategyGeneration: 3,
          strategyAppliedGeneration: 2,
          convergePhase: "ApplyFailed",
          lastApplyError: "PATCH failed: 500",
        }));
      }
      return Promise.resolve([]);
    });
    render(<EffectiveConfigView />);
    const chip = await screen.findByTestId("ec-converge-chip");
    expect(chip).toHaveAttribute("data-phase", "ApplyFailed");
    expect(chip).toHaveTextContent("ApplyFailed · PATCH failed: 500");
  });

  it("PendingApply renders an amber clickable chip that opens the reconcile preview", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") {
        return Promise.resolve(baseSnap({
          strategyGeneration: 3,
          strategyAppliedGeneration: 2,
          convergePhase: "PendingApply",
          platforms: [
            { state: "divergent", platform_name: "beta", platform_id: "b", whitebox_regions: ["hk"], resin_regions: ["us"], resin_allocation_policy: "p2c", b_class: "BALANCED", a_class: "region", manual_nodes: [], subscriptions: [], acknowledged: false },
          ],
        }));
      }
      return Promise.resolve([]);
    });
    render(<EffectiveConfigView />);
    const chip = await screen.findByTestId("ec-converge-chip");
    expect(chip).toHaveAttribute("data-phase", "PendingApply");
    expect(chip.tagName).toBe("BUTTON");
    fireEvent.click(chip);
    expect(await screen.findByTestId("ec-preview-dialog")).toBeInTheDocument();
  });

  it("NeverApplied and Unknown render their honest labels, not the three main states", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") {
        return Promise.resolve(baseSnap({ strategyGeneration: 0, strategyAppliedGeneration: 0, convergePhase: "NeverApplied" }));
      }
      return Promise.resolve([]);
    });
    const { unmount } = render(<EffectiveConfigView />);
    let chip = await screen.findByTestId("ec-converge-chip");
    expect(chip).toHaveAttribute("data-phase", "NeverApplied");
    expect(chip).toHaveTextContent(enCommon.effectiveConfig.convergeNever);
    unmount();

    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") {
        return Promise.resolve(baseSnap({ strategyGeneration: 2, strategyAppliedGeneration: 2, convergePhase: "Unknown", resinReachable: false }));
      }
      return Promise.resolve([]);
    });
    render(<EffectiveConfigView />);
    chip = await screen.findByTestId("ec-converge-chip");
    expect(chip).toHaveAttribute("data-phase", "Unknown");
    expect(chip).toHaveTextContent(enCommon.effectiveConfig.convergeUnknown);
  });

  it("Drifted renders the amber edge-state chip beside a per-entry divergent row", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") {
        return Promise.resolve(baseSnap({
          strategyGeneration: 3,
          strategyAppliedGeneration: 3,
          convergePhase: "Drifted",
          platforms: [
            { state: "divergent", platform_name: "beta", platform_id: "b", whitebox_regions: ["hk"], resin_regions: ["us"], resin_allocation_policy: "p2c", b_class: "BALANCED", a_class: "region", manual_nodes: [], subscriptions: [], divergent_since: TS, acknowledged: false },
          ],
        }));
      }
      return Promise.resolve([]);
    });
    render(<EffectiveConfigView />);
    const chip = await screen.findByTestId("ec-converge-chip");
    expect(chip).toHaveAttribute("data-phase", "Drifted");
    expect(chip).toHaveTextContent(enCommon.effectiveConfig.convergeDrifted);
  });

  it("no snapshot yet => no chip", async () => {
    invokeMock.mockImplementation(() => new Promise(() => {})); // never resolves
    render(<EffectiveConfigView />);
    await screen.findByTestId("ec-view");
    expect(screen.queryByTestId("ec-converge-chip")).toBeNull();
  });
});

// round9 ticket 02 (A-002): the Consistent row surfaces the live
// allocation_policy Resin reported for the platform — the visible half of the
// D-002 two-axis reconcile. The REAL en catalog is wired in so the assertions
// lock the shipped wording values, not key fallbacks.
describe("EffectiveConfigView live policy render (round9 ticket 02)", () => {
  beforeAll(() => {
    i18next.addResourceBundle("en", "translation", { effectiveConfig: enCommon.effectiveConfig }, true, true);
  });

  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") {
        return Promise.resolve(baseSnap({
          platforms: [
            { state: "consistent", platform_name: "alpha", platform_id: "a", regions: ["r1"], resin_allocation_policy: "PREFER_IDLE_IP", b_class: "", a_class: "", manual_nodes: [], subscriptions: [], acknowledged: false },
            { state: "consistent", platform_name: "omega", platform_id: "o", regions: ["r2"], resin_allocation_policy: "", b_class: "", a_class: "", manual_nodes: [], subscriptions: [], acknowledged: false },
            { state: "divergent", platform_name: "beta", platform_id: "b", whitebox_regions: ["r1", "r2"], resin_regions: ["r1"], resin_allocation_policy: "BALANCED", b_class: "", a_class: "", manual_nodes: [], subscriptions: [], divergent_since: TS, acknowledged: false },
          ],
        }));
      }
      return Promise.resolve([]);
    });
  });

  it("renders the live allocation_policy on the Consistent row only", async () => {
    render(<EffectiveConfigView />);
    const alpha = await screen.findByTestId("ec-live-policy-alpha");
    expect(alpha).toHaveTextContent(enCommon.effectiveConfig.livePolicy);
    expect(alpha).toHaveTextContent("PREFER_IDLE_IP");
    // an empty live policy degrades to the shared "not recorded" wording
    const omega = screen.getByTestId("ec-live-policy-omega");
    expect(omega).toHaveTextContent(enCommon.effectiveConfig.notRecorded);
    // the Divergent row keeps the region-only layout (scope: Consistent rows)
    expect(screen.queryByTestId("ec-live-policy-beta")).not.toBeInTheDocument();
  });
});
