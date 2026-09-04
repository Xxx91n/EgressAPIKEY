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
        { id: "0b7fd2a8-1f3e-4c5d-9a6b-7c8d9e0f1a2b", ts: "2026-01-01 00:00:00", platform_name: "Default", account: "port-1790",
          target_host: "api.openai.com:443", egress_ip: "1.2.3.4",
          http_method: "POST", http_status: 200, duration_ms: 150, resin_error: "" },
      ]);
      // T21 (Round 5): detail drawer wire faces
      if (cmd === "request_log_detail") return Promise.resolve({
        id: "0b7fd2a8-1f3e-4c5d-9a6b-7c8d9e0f1a2b", ts: "2026-01-01T00:00:00Z",
        proxy_type: 1, client_ip: "127.0.0.1", platform_id: "p-1", platform_name: "Default",
        account: "port-1790", target_host: "api.openai.com:443", target_url: "https://api.openai.com/v1/chat",
        node_hash: "h1", node_tag: "us-1", egress_ip: "1.2.3.4",
        duration_ms: 150, first_byte_duration_ms: 40, net_ok: true,
        http_method: "POST", http_status: 200, resin_error: "",
        ingress_bytes: 1024, egress_bytes: 2048, payload_present: true,
        req_body_len: 5, resp_body_len: 2,
      });
      if (cmd === "request_log_payloads") return Promise.resolve({
        req_headers_b64: btoa("content-type: application/json"),
        req_body_b64: btoa('{"a":1}'),
        resp_headers_b64: btoa("content-type: text/plain"),
        resp_body_b64: btoa("ok"),
        truncated: { req_headers: false, req_body: false, resp_headers: false, resp_body: false },
      });
      if (cmd === "get_sidecar_logs") return Promise.resolve(["line1", "line2"]);
      // T05: typed L1 command pair replaced the bare get/set_store_value bypass
      if (cmd === "get_diag_poll_interval") return Promise.resolve(5000);
      if (cmd === "set_diag_poll_interval") return Promise.resolve(undefined);
      // T19 (ADR-0064): metrics minimal set
      if (cmd === "metrics_realtime_throughput") return Promise.resolve({
        step_seconds: 10,
        items: [{ ts: "2026-09-04T00:00:00Z", ingress_bps: 1000, egress_bps: 2000 }],
      });
      if (cmd === "metrics_probe_history") return Promise.resolve({
        bucket_seconds: 60,
        items: [{ bucket_start: "2026-09-04T00:00:00Z", bucket_end: "2026-09-04T00:01:00Z", total_count: 3 }],
      });
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

  it("T05: loads poll interval via typed get_diag_poll_interval, never via bare get_store_value", async () => {
    render(<DiagnosticsView />);
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_diag_poll_interval");
    });
    const commands = invokeMock.mock.calls.map((c: unknown[]) => c[0]);
    expect(commands).not.toContain("get_store_value");
    expect(commands).not.toContain("set_store_value");
  });

  it("T05: changing the poll selector persists via typed set_diag_poll_interval", async () => {
    render(<DiagnosticsView />);
    await waitFor(() => {
      expect(screen.getByTestId("diag-poll-select")).toBeInTheDocument();
    });
    invokeMock.mockClear();
    fireEvent.change(screen.getByTestId("diag-poll-select"), { target: { value: "10000" } });
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("set_diag_poll_interval", { intervalMs: 10000 });
    });
  });

  it("T19: renders Resin metrics card with throughput + probe history charts", async () => {
    render(<DiagnosticsView />);
    await waitFor(() => {
      // Both metrics fetches ride the initial poll cycle.
      expect(invokeMock).toHaveBeenCalledWith("metrics_realtime_throughput", expect.anything());
      expect(invokeMock).toHaveBeenCalledWith("metrics_probe_history", expect.anything());
    });
    // Two sparklines render (throughput + probe history) — no empty state.
    expect(screen.getAllByTestId("metrics-sparkline")).toHaveLength(2);
    expect(screen.queryByTestId("metrics-throughput-empty")).not.toBeInTheDocument();
    expect(screen.queryByTestId("metrics-probes-empty")).not.toBeInTheDocument();
  });

  it("T19: probe-history range change refetches with a new from/to window", async () => {
    render(<DiagnosticsView />);
    await waitFor(() => {
      expect(screen.getByTestId("metrics-probe-range")).toBeInTheDocument();
    });
    invokeMock.mockClear();
    fireEvent.change(screen.getByTestId("metrics-probe-range"), { target: { value: "1h" } });
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "metrics_probe_history",
        expect.objectContaining({ from: expect.any(String), to: expect.any(String) }),
      );
    });
    const args = invokeMock.mock.calls
      .filter((c: unknown[]) => c[0] === "metrics_probe_history")
      .map((c: unknown[]) => c[1] as { from: string; to: string })
      .pop();
    expect(args).toBeDefined();
    const from = Date.parse(args!.from);
    const to = Date.parse(args!.to);
    expect(from).toBeLessThan(to);
    // 1h window (± a second of clock skew between the two Date() reads).
    expect(to - from).toBeGreaterThanOrEqual(3600_000 - 1000);
    expect(to - from).toBeLessThanOrEqual(3600_000 + 1000);
  });


  it("T21: clicking a request-log row opens the detail drawer with detail + payloads", async () => {
    render(<DiagnosticsView />);
    await waitFor(() => {
      expect(screen.getByTestId("diag-log-table")).toBeInTheDocument();
    });
    fireEvent.click(screen.getAllByTestId("diag-log-row")[0]);
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("request_log_detail", expect.objectContaining({ logId: "0b7fd2a8-1f3e-4c5d-9a6b-7c8d9e0f1a2b" }));
      expect(invokeMock).toHaveBeenCalledWith("request_log_payloads", expect.objectContaining({ logId: "0b7fd2a8-1f3e-4c5d-9a6b-7c8d9e0f1a2b" }));
    });
    expect(screen.getByTestId("diag-log-detail-grid")).toBeInTheDocument();
    expect(screen.getByTestId("diag-log-detail-http_status")).toHaveTextContent("200");
    expect(screen.getByTestId("diag-log-payloads")).toBeInTheDocument();
    // Payload halves render decoded text inside <details>.
    expect(screen.getByTestId("diag-payload-req-body")).toHaveTextContent('{"a":1}');
  });

  it("T21: drawer close button hides the drawer without extra detail calls", async () => {
    render(<DiagnosticsView />);
    await waitFor(() => {
      expect(screen.getByTestId("diag-log-table")).toBeInTheDocument();
    });
    fireEvent.click(screen.getAllByTestId("diag-log-row")[0]);
    await waitFor(() => {
      expect(screen.getByTestId("diag-log-detail-grid")).toBeInTheDocument();
    });
    const detailCalls = invokeMock.mock.calls.filter((c: unknown[]) => c[0] === "request_log_detail").length;
    fireEvent.click(screen.getByTestId("diag-log-detail-close"));
    expect(screen.queryByTestId("diag-log-detail")).not.toBeInTheDocument();
    expect(invokeMock.mock.calls.filter((c: unknown[]) => c[0] === "request_log_detail").length).toBe(detailCalls);
  });

  it("T21: a row without an id stays inert (no detail IPC)", async () => {
    render(<DiagnosticsView />);
    await waitFor(() => {
      expect(screen.getByTestId("diag-log-table")).toBeInTheDocument();
    });
    // The negative-scenario fixture below has no id; verify via a fresh mock here.
    invokeMock.mockClear();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "request_log_tail") return Promise.resolve([
        { ts: "2026-01-01 00:00:00", platform_name: "NoId", account: "x", target_host: "h",
          egress_ip: "1.1.1.1", http_method: "GET", http_status: 500, duration_ms: 1, resin_error: "boom" },
      ]);
      return Promise.resolve(null);
    });
    fireEvent.click(screen.getAllByTestId("diag-log-row")[0]);
    await new Promise((r) => setTimeout(r, 20));
    expect(invokeMock.mock.calls.map((c: unknown[]) => c[0])).not.toContain("request_log_detail");
    expect(screen.queryByTestId("diag-log-detail")).not.toBeInTheDocument();
  });
});


describe("T15-1: DiagnosticsView uses usePoll (not setInterval)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string, _args?: Record<string, unknown>) => {
      if (cmd === "get_sidecar_status") return Promise.resolve({ api_port: 12345, api_base: "http://127.0.0.1:12345", mode: "Running", pid: 9999, healthz_last_check: "2026-01-01T00:00:00Z", ipc_latency_us: 500 });
      if (cmd === "check_firewall_status") return Promise.resolve({ platform: "windows", firewall_on: true, inbound_blocked: true, detail: "Windows Firewall is ON." });
      if (cmd === "request_log_tail") return Promise.resolve([]);
      if (cmd === "get_sidecar_logs") return Promise.resolve([]);
      // T05: typed L1 command pair replaced the bare get/set_store_value bypass
      if (cmd === "get_diag_poll_interval") return Promise.resolve(5000);
      if (cmd === "set_diag_poll_interval") return Promise.resolve(undefined);
      // T19 (ADR-0064): metrics minimal set (also needed in usePoll block)
      if (cmd === "metrics_realtime_throughput") return Promise.resolve({ step_seconds: 10, items: [] });
      if (cmd === "metrics_probe_history") return Promise.resolve({ bucket_seconds: 60, items: [] });
      return Promise.resolve(null);
    });
  });

  it("pause polling when document hidden, resume when visible (usePoll integration)", async () => {
    // Track invoke calls to count poll cycles
    render(<DiagnosticsView />);
    await waitFor(() => {
      expect(screen.getByTestId("diag-sidecar-port")).toHaveTextContent("12345");
    });

    // fireImmediately fires on mount — count status calls
    const callsBefore = invokeMock.mock.calls.filter((c: unknown[]) => c[0] === "get_sidecar_status").length;
    expect(callsBefore).toBeGreaterThanOrEqual(1);

    // Simulate hidden → usePoll should stop polling
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
    document.dispatchEvent(new Event("visibilitychange"));

    // Wait a tick to ensure no new calls
    await new Promise(r => setTimeout(r, 50));
    const callsAfterHidden = invokeMock.mock.calls.filter((c: unknown[]) => c[0] === "get_sidecar_status").length;

    // Simulate visible → usePoll should resume with immediate fire
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
    document.dispatchEvent(new Event("visibilitychange"));

    await waitFor(() => {
      const callsAfterVisible = invokeMock.mock.calls.filter((c: unknown[]) => c[0] === "get_sidecar_status").length;
      // Should have at least one more call after resume (usePoll fires immediately on resume)
      expect(callsAfterVisible).toBeGreaterThan(callsAfterHidden);
    });

    // Cleanup: restore visible state
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
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

  it("T19: metrics card shows empty state when IPC rejects", async () => {
    render(<DiagnosticsView />);
    await waitFor(() => {
      expect(screen.getByTestId("metrics-throughput-empty")).toBeInTheDocument();
      expect(screen.getByTestId("metrics-probes-empty")).toBeInTheDocument();
    });
  });
});
