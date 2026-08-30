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

  it("surfaces snapshot read failures", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "authoritative_snapshot") return Promise.reject(new Error("error.resinUnreachable"));
      return Promise.resolve(null);
    });
    render(<EffectiveConfigView />);
    await screen.findByTestId("ec-error");
  });
});
