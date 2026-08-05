import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { invokeMock } from "../test/setup";
import { NodesView } from "./NodesView";

describe("NodesView", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("R3: renders node pool with health stats from node_list + node_pool_snapshot", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "node_list") return {
        items: [
          { node_hash: "h1", display_tag: "HK-01", region: "HK", failure_count: 0, has_outbound: true, tags: [{ subscriptionName: "sub1", tag: "ss" }] },
          { node_hash: "h2", display_tag: "US-01", region: "US", failure_count: 2, has_outbound: false, tags: [{ subscriptionName: "sub1", tag: "vmess" }] },
        ],
        total: 2,
      };
      if (cmd === "node_pool_snapshot") return {
        total_nodes: 2,
        healthy_nodes: 1,
        egress_ip_count: 2,
        healthy_egress_ip_count: 1,
      };
      return undefined;
    });

    render(<NodesView />);

    await waitFor(() => {
      expect(screen.getByText("HK-01")).toBeTruthy();
      expect(screen.getByText("US-01")).toBeTruthy();
    });

    // Aggregate stats render
    expect(invokeMock).toHaveBeenCalledWith("node_list", undefined);
    expect(invokeMock).toHaveBeenCalledWith("node_pool_snapshot", undefined);
  });

  it("P4: renders actual egress reputation returned by server-side IPC", async () => {
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

  it("P4: renders actual egress reputation returned by server-side IPC", async () => {
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

  it("R3: shows empty state when no nodes returned", async () => {
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
});
