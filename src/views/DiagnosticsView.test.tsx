import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { DiagnosticsView } from "./DiagnosticsView";

// Mock @tauri-apps/api/core
const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (..._a: unknown[]) => invokeMock(..._a),
}));
// Mock @tauri-apps/plugin-opener
vi.mock("@tauri-apps/plugin-opener", () => ({
  openPath: vi.fn().mockResolvedValue(undefined),
}));

describe("DiagnosticsView closed-loop tests", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string, _args?: Record<string, unknown>) => {
      if (cmd === "get_sidecar_status") return Promise.resolve({
        api_port: 12345, api_base: "http://127.0.0.1:12345", mode: "Running",
        pid: 9999, healthz_last_check: "2026-01-01T00:00:00Z", ipc_latency_us: 500,
      });
      if (cmd === "check_firewall_status") return Promise.resolve({
        platform: "windows", firewall_on: true, inbound_blocked: true,
        detail: "Windows Firewall is ON.",
      });
      if (cmd === "request_log_tail") return Promise.resolve([
        { ts: "2026-01-01 00:00:00", platform_name: "Default", account: "port-1790",
          target_host: "api.openai.com:443", egress_ip: "1.2.3.4",
          http_method: "POST", http_status: 200, duration_ms: 150, resin_error: "" },
      ]);
      if (cmd === "get_sidecar_logs") return Promise.resolve(["line1", "line2"]);
      if (cmd === "get_store_value") return Promise.resolve(5000);
      if (cmd === "set_store_value") return Promise.resolve(undefined);
      if (cmd === "probe_exit_ip") return Promise.resolve({
        port: 1790, protocol: "http", exit_ip: "1.2.3.4", latency_ms: 50, status: 200,
      });
      if (cmd === "port_health_check") return Promise.resolve({
        port: 1790, reachable: true, socks5_ok: true, protocol_mismatch: false,
        latency_ms: 10, reason: "ok",
      });
      return Promise.resolve(null);
    });
  });

  it("renders sidecar status card with port + mode + PID", async () => {
    render(<DiagnosticsView />);
    await waitFor(() => {
      expect(screen.getByTestId("diag-sidecar-port")).toHaveTextContent("12345");
    });
    expect(screen.getByTestId("diag-sidecar-mode")).toHaveTextContent("Running");
    expect(screen.getByTestId("diag-sidecar-pid")).toHaveTextContent("9999");
    expect(screen.getByTestId("diag-ipc-latency")).toHaveTextContent("500");
  });

  it("renders firewall status card", async () => {
    render(<DiagnosticsView />);
    await waitFor(() => {
      expect(screen.getByTestId("diag-firewall-status")).toBeInTheDocument();
    });
  });

  it("renders request log table with entries from IPC", async () => {
    render(<DiagnosticsView />);
    await waitFor(() => {
      expect(screen.getByTestId("diag-log-table")).toBeInTheDocument();
    });
    expect(screen.getByText("api.openai.com:443")).toBeInTheDocument();
  });

  it("renders sidecar log buffer", async () => {
    render(<DiagnosticsView />);
    await waitFor(() => {
      expect(screen.getByTestId("diag-sidecar-logs")).toBeInTheDocument();
    });
    expect(screen.getByTestId("diag-sidecar-logs")).toHaveTextContent("line1");
    expect(screen.getByTestId("diag-sidecar-logs")).toHaveTextContent("line2");
  });

  it("manual refresh button triggers IPC calls", async () => {
    render(<DiagnosticsView />);
    await waitFor(() => {
      expect(screen.getByTestId("diag-refresh-btn")).toBeInTheDocument();
    });
    // Clear mock to count new calls
    invokeMock.mockClear();
    fireEvent.click(screen.getByTestId("diag-refresh-btn"));
    await waitFor(() => {
      const calls = invokeMock.mock.calls.map((c: unknown[]) => c[0]);
      expect(calls).toContain("get_sidecar_status");
      expect(calls).toContain("check_firewall_status");
      expect(calls).toContain("request_log_tail");
      expect(calls).toContain("get_sidecar_logs");
    });
  });

  it("exit IP probe button triggers probe IPC", async () => {
    render(<DiagnosticsView />);
    await waitFor(() => {
      expect(screen.getByTestId("diag-probe-btn")).toBeInTheDocument();
    });
    invokeMock.mockClear();
    fireEvent.click(screen.getByTestId("diag-probe-btn"));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("probe_exit_ip", expect.objectContaining({ port: 1790 }));
    });
  });
});

describe("DiagnosticsView negative tests", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(() => Promise.reject(new Error("not in tauri")));
  });

  it("renders gracefully when all IPC calls fail", async () => {
    render(<DiagnosticsView />);
    await waitFor(() => {
      expect(screen.getByTestId("diag-no-logs")).toBeInTheDocument();
    });
  });
});
