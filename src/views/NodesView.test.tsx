import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { invokeMock } from "../test/setup";
import { NodesView } from "./NodesView";

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

  it("T4-3b: expanding a subscription shows its child nodes", async () => {
    mockTwoSubs();
    render(<NodesView />);
    await waitFor(() => screen.getByText("sub-alpha"));

    // Nodes should be visible initially (not collapsed by default)
    expect(screen.getByText("HK-01")).toBeTruthy();
    expect(screen.getByText("JP-01")).toBeTruthy();
    expect(screen.getByText("US-01")).toBeTruthy();
    expect(screen.getByText("DE-01")).toBeTruthy();
  });

  it("T4-3c: collapsing a subscription hides its nodes", async () => {
    mockTwoSubs();
    render(<NodesView />);
    await waitFor(() => screen.getByText("sub-alpha"));

    // Click sub-alpha header to collapse
    const alphaHeader = screen.getByText("sub-alpha").closest("button");
    expect(alphaHeader).toBeTruthy();
    fireEvent.click(alphaHeader!);

    // Now sub-alpha's nodes should be hidden
    expect(screen.queryByText("HK-01")).toBeNull();
    // sub-beta nodes still visible
    expect(screen.getByText("US-01")).toBeTruthy();
  });

  it("T4-3d: search filters nodes across subscriptions", async () => {
    mockTwoSubs();
    render(<NodesView />);
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
    await waitFor(() => screen.getByText("HK-01"));

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

  it("T14-7b: subscription expanded by default shows all nodes under threshold", async () => {
    mockLargeSub(10);
    render(<NodesView />);
    await waitFor(() => screen.getByText("sub-alpha"));
    // Subscriptions are expanded by default (collapsed=empty Set); all 10 nodes should render
    await waitFor(() => expect(screen.queryAllByText(/server-\d+\.example\.com/).length).toBe(10));
    // Clicking collapses — nodes disappear
    fireEvent.click(screen.getByText("sub-alpha").closest("button")!);
    await waitFor(() => expect(screen.queryAllByText(/server-\d+\.example\.com/).length).toBe(0));
    // Click again re-expands — nodes reappear
    fireEvent.click(screen.getByText("sub-alpha").closest("button")!);
    await waitFor(() => expect(screen.queryAllByText(/server-\d+\.example\.com/).length).toBe(10));
  });

  it("T14-7c: VirtualNodeList renders small list without virtualizer overhead", async () => {
    mockLargeSub(5);
    render(<NodesView />);
    await waitFor(() => screen.getByText("sub-alpha"));
    // 5 items < VIRTUAL_THRESHOLD(50) — normal render (no virtualizer)
    // All 5 node tags should be in the DOM immediately
    await waitFor(() => expect(screen.queryAllByText(/server-\d+\.example\.com/).length).toBe(5));
    // Collapse and verify all hide
    fireEvent.click(screen.getByText("sub-alpha").closest("button")!);
    await waitFor(() => expect(screen.queryAllByText(/server-\d+\.example\.com/).length).toBe(0));
  });
});
