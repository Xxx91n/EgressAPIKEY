import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

// Mock the IPC module so we control the platform/node data the canvas sees.
const invokeMock = vi.fn<(cmd: string, args?: Record<string, unknown>) => Promise<unknown>>();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(args[0] as string, args[1] as Record<string, unknown> | undefined),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));

import { TopologyView } from "./TopologyView";

describe("TopologyView (Phase R2 three-column canvas, closed-loop)", () => {
  beforeEach(() => { invokeMock.mockReset(); });

  it("renders entry port + platforms + node-groups from live Resin data", async () => {
    // platform_list_full returns the items-wrapper; node_list returns items-wrapper.
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "platform_list_full") return Promise.resolve({
        items: [
          { id: "u1", name: "OpenAI", regex_filters: ["api.openai.com"], region_filters: ["hk"], allocation_policy: "BALANCED", routable_node_count: 3, sticky_ttl: "168h0m0s" },
          { id: "u2", name: "Anthropic", regex_filters: null, region_filters: null, allocation_policy: "PREFER_LOW_LATENCY", routable_node_count: 0, sticky_ttl: "168h0m0s" },
        ],
        total: 2, limit: 50, offset: 0,
      });
      if (cmd === "node_list") return Promise.resolve({
        items: [
          { name: "hk-01", display_tag: "HK-01", has_outbound: true, failure_count: 0, tags: [{ subscriptionName: "sub", tag: "HK" }] },
          { name: "hk-02", display_tag: "HK-02", has_outbound: true, failure_count: 0, tags: [{ subscriptionName: "sub", tag: "HK" }] },
          { name: "us-01", display_tag: "US-01", has_outbound: true, failure_count: 1, tags: [{ subscriptionName: "sub", tag: "US" }] },
        ],
        total: 3, limit: 50, offset: 0,
      });
      return Promise.resolve(undefined);
    });

    render(<TopologyView />);
    // The canvas renders platform names and node-group regions.
    await waitFor(() => {
      expect(screen.getByText("OpenAI")).toBeInTheDocument();
      expect(screen.getByText("Anthropic")).toBeInTheDocument();
    });
    // Entry port label present.
    expect(screen.getByText(/forward proxy/i)).toBeInTheDocument();
  });

  it("shows noPlatforms / noNodes hints when Resin returns empty", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "platform_list_full") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
      if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
      return Promise.resolve(undefined);
    });

    render(<TopologyView />);
    await waitFor(() => {
      // i18n test setup uses en; the key is topology.noPlatforms.
      expect(screen.getByText(/No platforms|没有平台/i)).toBeInTheDocument();
      expect(screen.getByText(/No nodes|没有加载节点/i)).toBeInTheDocument();
    });
  });
});
