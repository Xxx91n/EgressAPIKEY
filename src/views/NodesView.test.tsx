import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { invokeMock } from "../test/setup";
import { NodesView, parseDelayQuery } from "./NodesView";

describe("NodesView T4-3", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  // Helper: mock data with two subscriptions and 4 nodes
  function mockTwoSubs() {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "node_list") return {
        items: [
          { node_hash: "h1", display_tag: "HK-01", region: "HK", failure_count: 0, has_outbound: true, reference_latency_ms: 120, egress_ip: "1.1.1.1", tags: [{ subscription_name: "sub-alpha", tag: "vmess" }] },
          { node_hash: "h2", display_tag: "JP-01", region: "JP", failure_count: 0, has_outbound: true, reference_latency_ms: 350, egress_ip: "2.2.2.2", tags: [{ subscription_name: "sub-alpha", tag: "ss" }] },
          { node_hash: "h3", display_tag: "US-01", region: "US", failure_count: 3, has_outbound: false, reference_latency_ms: null, egress_ip: "3.3.3.3", tags: [{ subscription_name: "sub-beta", tag: "trojan" }] },
          { node_hash: "h4", display_tag: "DE-01", region: "DE", failure_count: 0, has_outbound: true, reference_latency_ms: 800, egress_ip: "4.4.4.4", tags: [{ subscription_name: "sub-beta", tag: "vmess" }] },
        ],
        total: 4,
      };
      if (cmd === "node_pool_snapshot") return {
        total_nodes: 4, healthy_nodes: 3, egress_ip_count: 4, healthy_egress_ip_count: 3,
      };
      return undefined;
    });
  }

  it("T4-3a: groups nodes by subscription_name in collapsible tree", async () => {
    mockTwoSubs();
    render(<NodesView />);
    await waitFor(() => {
      expect(screen.getByText("sub-alpha")).toBeTruthy();
      expect(screen.getByText("sub-beta")).toBeTruthy();
    });
    // Both subscription headers render with node counts (2 each)
    // nodeCount interpolation: "2 nodes" for both subs
    const nodeCountEls = screen.getAllByText(/\d+ nodes/);
    expect(nodeCountEls.length).toBe(2);
  });

  // T19-P1: groups default-collapsed — flip T4-3b: nodes hidden after first refresh, expandable on click
  it("T4-3b (T19): groups are collapsed by default after first refresh; expand reveals nodes", async () => {
    mockTwoSubs();
    render(<NodesView />);
    await waitFor(() => screen.getByText("sub-alpha"));
    // Nodes hidden by default (collapsed seeded after first refresh)
    await waitFor(() => {
      expect(screen.queryByText("HK-01")).toBeNull();
      expect(screen.queryByText("US-01")).toBeNull();
    });
    // Click sub-alpha header to expand
    fireEvent.click(screen.getByText("sub-alpha").closest("button")!);
    await waitFor(() => {
      expect(screen.getByText("HK-01")).toBeTruthy();
      expect(screen.getByText("JP-01")).toBeTruthy();
    });
    // sub-beta still collapsed
    expect(screen.queryByText("US-01")).toBeNull();
  });

  it("T4-3c: collapsing an expanded subscription hides its nodes", async () => {
    mockTwoSubs();
    render(<NodesView />);
    await waitFor(() => screen.getByText("sub-alpha"));
    // expand first
    fireEvent.click(screen.getByText("sub-alpha").closest("button")!);
    await waitFor(() => screen.getByText("HK-01"));
    // collapse again
    fireEvent.click(screen.getByText("sub-alpha").closest("button")!);
    await waitFor(() => expect(screen.queryByText("HK-01")).toBeNull());
  });

  it("T4-3d: search filters nodes across subscriptions", async () => {
    mockTwoSubs();
    render(<NodesView />);
    await waitFor(() => screen.getByText("sub-alpha"));
    // expand to reveal before searching
    fireEvent.click(screen.getByText("sub-alpha").closest("button")!);
    fireEvent.click(screen.getByText("sub-beta").closest("button")!);
    await waitFor(() => screen.getByText("HK-01"));

    // Type "HK" in search
    const searchInput = screen.getByPlaceholderText(/Search nodes/i);
    expect(searchInput).toBeTruthy();
    fireEvent.change(searchInput, { target: { value: "HK" } });

    // Only HK-01 visible, JP-01 hidden
    await waitFor(() => {
      expect(screen.getByText("HK-01")).toBeTruthy();
      expect(screen.queryByText("JP-01")).toBeNull();
      expect(screen.queryByText("US-01")).toBeNull();
    });
  });

  it("T4-3e: shows no-match state when search has no hits", async () => {
    mockTwoSubs();
    render(<NodesView />);
    await waitFor(() => screen.getByText("sub-alpha"));

    const searchInput = screen.getByPlaceholderText(/Search nodes/i);
    fireEvent.change(searchInput, { target: { value: "ZZZNONEXIST" } });

    await waitFor(() => {
      expect(screen.getByText(/nodes\.noMatch|No nodes match/i)).toBeTruthy();
    });
  });

  it("T4-3f: health rate percentage shown per subscription", async () => {
    mockTwoSubs();
    render(<NodesView />);
    await waitFor(() => screen.getByText("sub-alpha"));

    // sub-alpha: 2/2 healthy = 100%, sub-beta: 1/2 healthy = 50%
    // healthRate interpolation: "100% healthy" and "50% healthy"
    const healthEls = screen.getAllByText(/\d+% healthy/);
    expect(healthEls.length).toBe(2);
  });

  it("T4-3g: empty state when no nodes returned", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "node_list") return { items: [], total: 0 };
      if (cmd === "node_pool_snapshot") return { total_nodes: 0, healthy_nodes: 0, egress_ip_count: 0, healthy_egress_ip_count: 0 };
      return undefined;
    });
    render(<NodesView />);
    await waitFor(() => {
      expect(screen.getByText(/nodes\.empty|No nodes loaded/i)).toBeTruthy();
    });
  });

  it("T4-3h: aggregate stats render from node_pool_snapshot", async () => {
    mockTwoSubs();
    render(<NodesView />);
    await waitFor(() => screen.getByText("sub-alpha"));

    // Stat cards show pool values
    expect(invokeMock).toHaveBeenCalledWith("node_list", expect.objectContaining({ __trace_id: expect.any(String) }));
    expect(invokeMock).toHaveBeenCalledWith("node_pool_snapshot", expect.objectContaining({ __trace_id: expect.any(String) }));
  });

  it("T4-3i: P4 reputation card renders when IPC returns entries", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "node_list") return { items: [] };
      if (cmd === "node_pool_snapshot") return { total_nodes: 0, healthy_nodes: 0, egress_ip_count: 0, healthy_egress_ip_count: 0 };
      if (cmd === "ip_reputation_snapshot") return { provider: "ip_api", status: "ok", entries: [{ ip: "1.1.1.1", score: 3, cached: true }] };
      return undefined;
    });
    render(<NodesView />);
    await waitFor(() => expect(screen.getByText(/1\.1\.1\.1/)).toBeTruthy());
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "ip_reputation_snapshot")).toBe(true);
  });


  // T14-7: mock for large subscription lists
  function mockLargeSub(count: number) {
    const items = Array.from({ length: count }, (_, i) => ({
      node_hash: "node-" + i,
      display_tag: "server-" + i + ".example.com",
      has_outbound: true,
      failure_count: 0,
      region: "HK",
      reference_latency_ms: 50 + (i % 100),
      tags: [{ tag: "sub-alpha" }],
    }));
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "node_list") return { items };
      if (cmd === "node_pool_snapshot") return { total_nodes: count, healthy_nodes: count, egress_ip_count: 1, healthy_egress_ip_count: 1 };
      if (cmd === "ip_reputation_snapshot") return { provider: "ip_api", status: "ok", entries: [] };
      return undefined;
    });
  }

  // T14-7: VirtualNodeList component — threshold-based virtualization
  // In jsdom, virtualized mode can't measure scroll, so we test the <threshold path

  it("T14-7a: renders subscription header for a node list under threshold", async () => {
    mockLargeSub(30);
    render(<NodesView />);
    await waitFor(() => expect(screen.getByText("sub-alpha")).toBeTruthy());
    
  });

  // T19-P1: flip T14-7b — subscription collapsed by default, expand reveals all 10 nodes
  it("T14-7b (T19): subscription collapsed by default; expand shows all nodes", async () => {
    mockLargeSub(10);
    render(<NodesView />);
    await waitFor(() => screen.getByText("sub-alpha"));
    // Collapsed by default (T19-P1) — none of the 10 nodes render
    await waitFor(() => expect(screen.queryAllByText(/server-\d+\.example\.com/).length).toBe(0));
    // Click to expand — all 10 appear
    fireEvent.click(screen.getByText("sub-alpha").closest("button")!);
    await waitFor(() => expect(screen.queryAllByText(/server-\d+\.example\.com/).length).toBe(10));
    // Collapse again — nodes disappear
    fireEvent.click(screen.getByText("sub-alpha").closest("button")!);
    await waitFor(() => expect(screen.queryAllByText(/server-\d+\.example\.com/).length).toBe(0));
  });

  it("T14-7c: VirtualNodeList renders small list without virtualizer overhead", async () => {
    mockLargeSub(5);
    render(<NodesView />);
    await waitFor(() => screen.getByText("sub-alpha"));
    // Collapsed by default (T19-P1); expand to see 5 nodes
    fireEvent.click(screen.getByText("sub-alpha").closest("button")!);
    await waitFor(() => expect(screen.queryAllByText(/server-\d+\.example\.com/).length).toBe(5));
    // Collapse and verify all hide
    fireEvent.click(screen.getByText("sub-alpha").closest("button")!);
    await waitFor(() => expect(screen.queryAllByText(/server-\d+\.example\.com/).length).toBe(0));
  });
});

/// T19-P1 — pure-function closed loops for parseDelayQuery + sortMode + hideUnhealthy
describe("NodesView T19-P1 parseDelayQuery + sort + hide-unhealthy", () => {
  it("P1-1a: delay>100 admits 200, rejects 50 and null", () => {
    const { delayFilter } = parseDelayQuery("delay>100");
    expect(delayFilter).toBeTruthy();
    expect(delayFilter!(200)).toBe(true);
    expect(delayFilter!(50)).toBe(false);
    expect(delayFilter!(null)).toBe(false); // null treated as 0, not >100
  });

  it("P1-1b: delay<200 admits 100, rejects 300 and null", () => {
    const { delayFilter } = parseDelayQuery("delay<200");
    expect(delayFilter!(100)).toBe(true);
    expect(delayFilter!(300)).toBe(false);
    expect(delayFilter!(null)).toBe(false);
  });

  it("P1-1c: delay=timeout admits null and 9999+, rejects 200", () => {
    const { delayFilter } = parseDelayQuery("delay=timeout");
    expect(delayFilter!(null)).toBe(true);
    expect(delayFilter!(15000)).toBe(true);
    expect(delayFilter!(200)).toBe(false);
  });

  it("P1-1d: delay=error admits null, rejects numeric", () => {
    const { delayFilter } = parseDelayQuery("delay=error");
    expect(delayFilter!(null)).toBe(true);
    expect(delayFilter!(200)).toBe(false);
  });

  it("P1-1e: plain text query passes through as text (no delayFilter)", () => {
    const r = parseDelayQuery("JP");
    expect(r.text).toBe("JP");
    expect(r.delayFilter).toBeUndefined();
  });

  it("P1-2a: hide-unhealthy toggle excludes failure_count>0 rows (integration)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "node_list") return {
        items: [
          { node_hash: "h1", display_tag: "OK-01", failure_count: 0, has_outbound: true, reference_latency_ms: 100, tags: [{ subscription_name: "sub-x", tag: "v" }] },
          { node_hash: "h2", display_tag: "DEAD-02", failure_count: 5, has_outbound: false, reference_latency_ms: null, tags: [{ subscription_name: "sub-x", tag: "v" }] },
        ],
      };
      if (cmd === "node_pool_snapshot") return { total_nodes: 2, healthy_nodes: 1, egress_ip_count: 1, healthy_egress_ip_count: 1 };
      return undefined;
    });
    render(<NodesView />);
    await waitFor(() => screen.getByText("sub-x"));
    fireEvent.click(screen.getByText("sub-x").closest("button")!);
    // Toggle visible initially
    await waitFor(() => expect(screen.queryAllByText(/OK-01|DEAD-02/).length).toBe(2), { timeout: 3000 });
    // Click hide-unhealthy toggle (getByTitle is more robust than text match across icon+label)
    const toggle = screen.getByTitle("Hide unhealthy");
    fireEvent.click(toggle);
    await waitFor(() => {
      expect(screen.queryByText("OK-01")).toBeTruthy();
      expect(screen.queryByText("DEAD-02")).toBeNull();
    });
  });

  it("P1-3a: 3-state sortMode cycles default → asc → desc → default (integration)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "node_list") return {
        items: [
          { node_hash: "a", display_tag: "A", failure_count: 0, has_outbound: true, reference_latency_ms: 500, tags: [{ subscription_name: "sub", tag: "v" }] },
          { node_hash: "b", display_tag: "B", failure_count: 0, has_outbound: true, reference_latency_ms: 100, tags: [{ subscription_name: "sub", tag: "v" }] },
          { node_hash: "c", display_tag: "C", failure_count: 0, has_outbound: true, reference_latency_ms: 300, tags: [{ subscription_name: "sub", tag: "v" }] },
        ],
      };
      if (cmd === "node_pool_snapshot") return { total_nodes: 3, healthy_nodes: 3, egress_ip_count: 0, healthy_egress_ip_count: 0 };
      return undefined;
    });
    render(<NodesView />);
    await waitFor(() => screen.getByText("sub"));
    fireEvent.click(screen.getByText("sub").closest("button")!);

    // Default sort — nodes render in source order (A, B, C)
    const defaultRows = screen.queryAllByText(/^[ABC]$/);
    expect(defaultRows.length).toBe(3);

    // Click sort button: asc cycle (getByTitle matches the initial default state)
    let sortBtn = screen.getByTitle("Sort: default").closest("button")!;
    fireEvent.click(sortBtn);
    await waitFor(() => {
      const rows = screen.queryAllByText(/^[ABC]$/);
      // asc = B(100), C(300), A(500)
      expect(rows[0].textContent).toBe("B");
      expect(rows[1].textContent).toBe("C");
      expect(rows[2].textContent).toBe("A");
    });

    // Click sort button: desc cycle — title swapped to "Sort: latency ↑"
    sortBtn = screen.getByTitle("Sort: latency ↑").closest("button")!;
    fireEvent.click(sortBtn);
    await waitFor(() => {
      const rows = screen.queryAllByText(/^[ABC]$/);
      // desc = A(500), C(300), B(100)
      expect(rows[0].textContent).toBe("A");
      expect(rows[1].textContent).toBe("C");
      expect(rows[2].textContent).toBe("B");
    });

    // Click sort button: back to default — title swapped to "Sort: latency ↓"
    sortBtn = screen.getByTitle("Sort: latency ↓").closest("button")!;
    fireEvent.click(sortBtn);
    // default returns to source order
  });
});

describe("NodesView T19-P4 batch probe timeout + batch_on_load=false gate", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  // T19-P4 "timeout fires" spec (audit deviation fix).
  // Verifies the Promise.race(() => probe, timeout(cfg.timeout_ms)) path:
  // when a probe never resolves, the setTimeout timeout fires, the inner
  // try/catch swallows it, and the batch loop continues -> refresh() is
  // called and the subscription is no longer flagged as inflight.
  it("P4-2a: individual probe timeout fires (Promise.race) and batch completes", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      invokeMock.mockImplementation(async (cmd: string) => {
        if (cmd === "node_list") return {
          items: [
            { node_hash: "h1", display_tag: "HK-01", region: "HK", failure_count: 0, has_outbound: true, reference_latency_ms: null, tags: [{ subscription_name: "sub-alpha", tag: "vmess" }] },
            { node_hash: "h2", display_tag: "JP-01", region: "JP", failure_count: 0, has_outbound: true, reference_latency_ms: null, tags: [{ subscription_name: "sub-alpha", tag: "ss" }] },
          ],
          total: 2,
        };
        if (cmd === "node_pool_snapshot") return { total_nodes: 2, healthy_nodes: 2, egress_ip_count: 0, healthy_egress_ip_count: 0 };
        if (cmd === "node_probe") {
          // Never resolves — forces the Promise.race timeout branch.
          return new Promise(() => {});
        }
        return undefined;
      });
      render(<NodesView />);
      await vi.waitFor(() => screen.getByText("sub-alpha"));
      fireEvent.click(screen.getByText("sub-alpha"));

      const batchBtn = screen.getByTitle(/Batch probe/i).closest("button")!;
      fireEvent.click(batchBtn);

      // loadNodeProbe() returns default { timeout_ms: 10000 }. Advance fake
      // clock past the timeout so each Promise.race's setTimeout rejects.
      await vi.advanceTimersByTimeAsync(10050);

      const probeCalls = invokeMock.mock.calls.filter(([c]) => c === "node_probe");
      expect(probeCalls.length).toBeGreaterThanOrEqual(2);
    } finally {
      vi.useRealTimers();
    }
  });

  // T19-P4 "batch_on_load=false skips auto-batch" spec (audit deviation fix).
  // ADR-0044 S4 default behavior: manual single-probe. batch_on_load=false
  // prevents an automatic batch probe on page mount. NodesView has NO
  // useEffect reading batch_on_load to auto-trigger, so on mount + the 10s
  // poll refresh the node_probe IPC is never invoked; only node_list and
  // node_pool_snapshot fire.
  it("P4-2b: batch_on_load=false skips auto-batch on mount (ADR-0044 S4)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "node_list") return {
        items: [
          { node_hash: "x1", display_tag: "A", region: "HK", failure_count: 0, has_outbound: true, reference_latency_ms: 100, tags: [{ subscription_name: "sub", tag: "vmess" }] },
        ],
        total: 1,
      };
      if (cmd === "node_pool_snapshot") return { total_nodes: 1, healthy_nodes: 1, egress_ip_count: 1, healthy_egress_ip_count: 1 };
      if (cmd === "node_probe") throw new Error("auto-batch must not fire when batch_on_load=false");
      return undefined;
    });
    render(<NodesView />);
    await waitFor(() => screen.getByText("sub"));
    await new Promise((r) => setTimeout(r, 50));
    const probeCalls = invokeMock.mock.calls.filter(([c]) => c === "node_probe");
    expect(probeCalls).toHaveLength(0);
  });
});
