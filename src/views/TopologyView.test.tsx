import { describe, it, expect, vi, beforeEach, beforeAll, afterAll } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import i18next from "i18next";

// Mock the IPC module so we control the platform/node data the canvas sees.
const invokeMock = vi.fn<(cmd: string, args?: Record<string, unknown>) => Promise<unknown>>();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(args[0] as string, args[1] as Record<string, unknown> | undefined),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));

import { TopologyView, addRegionFilter, removeRegionFilter, patchAndSyncOnce, buildEdges, getSelectedRegions, layoutNodesViaDagre, fixedHandleStyle, useTopologyStore } from "./TopologyView";

describe("TopologyView (T9 canvas: subscription-folded C + strategy labels + dual badges)", () => {
  beforeEach(() => { invokeMock.mockReset(); });

  it("renders entry port + platforms + subscription groups from live Resin data", async () => {
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
          { name: "hk-01", display_tag: "HK-01", has_outbound: true, failure_count: 0, tags: [{ subscriptionName: "sub1", tag: "HK" }] },
          { name: "hk-02", display_tag: "HK-02", has_outbound: true, failure_count: 0, tags: [{ subscriptionName: "sub1", tag: "HK" }] },
          { name: "us-01", display_tag: "US-01", has_outbound: true, failure_count: 1, tags: [{ subscriptionName: "sub2", tag: "US" }] },
        ],
        total: 3, limit: 50, offset: 0,
      });
      if (cmd === "lease_map") return Promise.resolve([]);
      if (cmd === "port_list") return Promise.resolve([
        { port: 17990, protocol: "socks5", platform_name: "OpenAI", account: "acc1", label: "Pool1", enabled: true },
        { port: 17991, protocol: "http", platform_name: "Anthropic", account: "acc2", label: "Pool2", enabled: true },
      ]);
      return Promise.resolve(undefined);
    });

    render(<TopologyView />);
    await waitFor(() => {
      expect(screen.getByText("OpenAI")).toBeInTheDocument();
      expect(screen.getByText("Anthropic")).toBeInTheDocument();
    });
    expect(screen.getAllByText("17990").length).toBeGreaterThan(0);
    expect(screen.getAllByText("17991").length).toBeGreaterThan(0);
  });

  it("T9-1: C column shows subscription group names (not region labels)", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "platform_list_full") return Promise.resolve({
        items: [{ id: "p1", name: "OpenAI", regex_filters: [], region_filters: ["hk"], allocation_policy: "BALANCED", routable_node_count: 2, sticky_ttl: "" }],
        total: 1, limit: 50, offset: 0,
      });
      if (cmd === "node_list") return Promise.resolve({
        items: [
          { name: "hk-01", display_tag: "HK-01", has_outbound: true, failure_count: 0, tags: [{ subscriptionName: "my-sub", tag: "HK" }] },
          { name: "hk-02", display_tag: "HK-02", has_outbound: true, failure_count: 0, tags: [{ subscriptionName: "my-sub", tag: "HK" }] },
        ],
        total: 2, limit: 500, offset: 0,
      });
      if (cmd === "lease_map") return Promise.resolve([]);
      if (cmd === "port_list") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    render(<TopologyView />);
    await waitFor(() => {
      // The subscription name "my-sub" should appear as a C-column node label
      expect(screen.getByText("my-sub")).toBeInTheDocument();
    });
  });

  it("T9-3: platform node shows B-class strategy badge (blue badge)", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "platform_list_full") return Promise.resolve({
        items: [{ id: "p1", name: "TestPlat", regex_filters: [], region_filters: ["us"], allocation_policy: "PREFER_LOW_LATENCY", routable_node_count: 1, sticky_ttl: "" }],
        total: 1, limit: 50, offset: 0,
      });
      if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 500, offset: 0 });
      if (cmd === "lease_map") return Promise.resolve([]);
      if (cmd === "port_list") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    const { container } = render(<TopologyView />);
    await waitFor(() => {
      // The B: badge should be present (mapped from PREFER_LOW_LATENCY -> latency -> strategy.latency i18n key)
      const text = container.textContent || "";
      expect(text).toMatch(/B:/);
    });
  });

  it("T9-3: platform node shows A-class strategy badge (green badge)", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "platform_list_full") return Promise.resolve({
        items: [{ id: "p1", name: "TestPlat2", regex_filters: [], region_filters: ["hk", "us"], allocation_policy: "BALANCED", routable_node_count: 1, sticky_ttl: "" }],
        total: 1, limit: 50, offset: 0,
      });
      if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 500, offset: 0 });
      if (cmd === "lease_map") return Promise.resolve([]);
      if (cmd === "port_list") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    const { container } = render(<TopologyView />);
    await waitFor(() => {
      const text = container.textContent || "";
      // A-class badge shows region:HK,US when region_filters has hk+us
      expect(text).toMatch(/A:.*region.*HK.*US/i);
    });
  });

  it("T9-3: platform node shows A: manual when no region_filters", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "platform_list_full") return Promise.resolve({
        items: [{ id: "p2", name: "ManualPlat", regex_filters: [], region_filters: [], allocation_policy: "BALANCED", routable_node_count: 0, sticky_ttl: "" }],
        total: 1, limit: 50, offset: 0,
      });
      if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 500, offset: 0 });
      if (cmd === "lease_map") return Promise.resolve([]);
      if (cmd === "port_list") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    const { container } = render(<TopologyView />);
    await waitFor(() => {
      const text = container.textContent || "";
      expect(text).toContain("A: manual");
    });
  });

  it("shows noPlatforms / noNodes hints when Resin returns empty", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "platform_list_full") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
      if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
      if (cmd === "lease_map") return Promise.resolve([]);
      if (cmd === "port_list") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    render(<TopologyView />);
    await waitFor(() => {
      expect(screen.getByText(/No platforms|没有平台/i)).toBeInTheDocument();
      expect(screen.getByText(/No nodes|没有加载节点/i)).toBeInTheDocument();
    });
  });

  // --- Q2-Bug1 closed-loop: idempotent region_filters helpers ---
  describe("Q2-Bug1: region_filters add/remove idempotency", () => {
    it("addRegionFilter is idempotent and returns same ref on no-op", () => {
      const a1 = addRegionFilter(["hk", "us"], "jp");
      expect(a1).toEqual(["hk", "us", "jp"]);
      const a2 = addRegionFilter(a1, "jp");
      expect(a2).toBe(a1);
    });
    it("addRegionFilter handles null -> [region]", () => {
      expect(addRegionFilter(null, "sg")).toEqual(["sg"]);
    });
    it("removeRegionFilter works on null", () => {
      expect(removeRegionFilter(null, "hk")).toEqual([]);
    });
    it("removeRegionFilter -> empty array (never null)", () => {
      const out = removeRegionFilter(["hk"], "hk");
      expect(out).toEqual([]);
      expect(out).not.toBeNull();
    });
  });

  // --- C1-2 closed-loop: patchAndSyncOnce helper ---
  describe("C1-2: patchAndSyncOnce backup->PATCH->sync ordering", () => {
    it("add mode with already-bound region skips PATCH", async () => {
      const sync = vi.fn(async () => {});
      const ipcUpdate = vi.fn(async () => {});
      const backup = vi.fn(async () => {});
      const res = await patchAndSyncOnce({ platName: "OpenAI", current: ["hk"], region: "hk", mode: "add", sync, ipcUpdate, backup });
      expect(res.patched).toBe(false);
      expect(ipcUpdate).not.toHaveBeenCalled();
      expect(sync).not.toHaveBeenCalled();
      expect(backup).not.toHaveBeenCalled();
    });

    it("add mode with new region calls backup THEN ipcUpdate THEN sync", async () => {
      const order: string[] = [];
      const sync = vi.fn(async () => { order.push("sync"); });
      const ipcUpdate = vi.fn(async () => { order.push("patch"); });
      const backup = vi.fn(async () => { order.push("backup"); });
      const res = await patchAndSyncOnce({ platName: "OpenAI", current: [], region: "jp", mode: "add", sync, ipcUpdate, backup });
      expect(res.patched).toBe(true);
      expect(res.next).toEqual(["jp"]);
      expect(order).toEqual(["backup", "patch", "sync"]);
    });

    it("remove mode always PATCHes and sends [] not null", async () => {
      const sync = vi.fn(async () => {});
      const ipcUpdate = vi.fn(async () => {});
      const res = await patchAndSyncOnce({ platName: "P", current: [], region: "tw", mode: "remove", sync, ipcUpdate, backup: undefined });
      expect(res.patched).toBe(true);
      expect(res.next).toEqual([]);
      expect(ipcUpdate).toHaveBeenCalledWith("P", undefined, undefined, []);
    });

    it("backup swallows failure so it never blocks the routing PATCH", async () => {
      const sync = vi.fn(async () => {});
      const ipcUpdate = vi.fn(async () => {});
      const backup = vi.fn(async () => { throw new Error("webdav down"); });
      const res = await patchAndSyncOnce({ platName: "OpenAI", current: [], region: "kr", mode: "add", sync, ipcUpdate, backup });
      expect(res.patched).toBe(true);
      expect(ipcUpdate).toHaveBeenCalled();
      expect(sync).toHaveBeenCalled();
    });

    it("racy double-PATCH: second call is idempotent skip", async () => {
      const sync = vi.fn(async () => {});
      const ipcUpdate = vi.fn(async () => {});
      const a = await patchAndSyncOnce({ platName: "OpenAI", current: [], region: "hk", mode: "add", sync, ipcUpdate, backup: undefined });
      const b = await patchAndSyncOnce({ platName: "OpenAI", current: a.next, region: "hk", mode: "add", sync, ipcUpdate, backup: undefined });
      expect(a.patched).toBe(true);
      expect(b.patched).toBe(false);
      expect(ipcUpdate).toHaveBeenCalledTimes(1);
    });
  });

  // --- C1-3 + T9-2: buildEdges with strategy labels ---
  describe("C1-3 + T9-2: buildEdges B->C edge with strategy labels", () => {
    // Old shape (region groups) for backward compat
    const regionGroups = [{ region: "hk" }, { region: "us" }];
    it("renders B->C edge for each region in region_filters (old shape)", () => {
      const edges = buildEdges([{ name: "OpenAI", region_filters: ["hk", "us"] }], regionGroups as any);
      expect(edges.find((e) => e.id === "e-OpenAI-hk")).toBeTruthy();
      expect(edges.find((e) => e.id === "e-OpenAI-us")).toBeTruthy();
    });
    it("drops edge after region removed (delete semantics)", () => {
      const before = buildEdges([{ name: "OpenAI", region_filters: ["hk"] }], regionGroups as any);
      expect(before.find((e) => e.id === "e-OpenAI-hk")).toBeTruthy();
      const after = buildEdges([{ name: "OpenAI", region_filters: [] }], regionGroups as any);
      expect(after.find((e) => e.id === "e-OpenAI-hk")).toBeFalsy();
    });
    it("ADR-0012: A->B edge connects port to platform", () => {
      const edges = buildEdges(
        [{ name: "OpenAI", region_filters: null }],
        regionGroups as any,
        [{ port: 17990, platform_name: "OpenAI" }],
      );
      expect(edges.find((e) => e.id === "e-port-17990-OpenAI" && e.source === "entry-port-17990" && e.target === "platform-OpenAI")).toBeTruthy();
    });
    it("does not leak edges for unbound groups", () => {
      const edges = buildEdges([{ name: "OpenAI", region_filters: ["hk"] }], regionGroups as any);
      expect(edges.find((e) => e.id === "e-OpenAI-us")).toBeFalsy();
    });

    // T9-2: New shape (subscription groups) with strategy labels
    it("T9-2: buildEdges with subscription groups produces strategy-labeled edges", () => {
      const platforms = [{ name: "OpenAI", region_filters: ["hk"], allocation_policy: "BALANCED" }];
      const subGroups = [{ subscriptionName: "sub1", regions: ["hk", "jp"] }];
      const edges = buildEdges(platforms, subGroups as any, []);
      const bcEdges = edges.filter((e) => e.source.startsWith("platform-"));
      expect(bcEdges).toHaveLength(1);
      expect(bcEdges[0].label).toBe("region:hk");
      // Auto-strategy edges are non-deletable (T9-2)
      expect(bcEdges[0].deletable).toBe(false);
    });

    it("T9-2: no edge when platform region_filters don't intersect subscription regions", () => {
      const platforms = [{ name: "OpenAI", region_filters: ["us"], allocation_policy: "BALANCED" }];
      const subGroups = [{ subscriptionName: "sub1", regions: ["hk", "jp"] }];
      const edges = buildEdges(platforms, subGroups as any, []);
      const bcEdges = edges.filter((e) => e.source.startsWith("platform-"));
      expect(bcEdges).toHaveLength(0);
    });

    it("T9-2: edge label shows region:<matched> for multi-region intersection", () => {
      const platforms = [{ name: "OpenAI", region_filters: ["hk", "jp"], allocation_policy: "BALANCED" }];
      const subGroups = [{ subscriptionName: "sub1", regions: ["hk", "jp", "us"] }];
      const edges = buildEdges(platforms, subGroups as any, []);
      const bcEdges = edges.filter((e) => e.source.startsWith("platform-"));
      expect(bcEdges).toHaveLength(1);
      expect(bcEdges[0].label).toBe("region:hk,jp");
    });
  });

  // --- ADR-0012 lease identity chips ---
  describe("ADR-0012: lease chips show Platform.Account identity", () => {
    it("renders lease account + egress", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({
          items: [{ id: "p1", name: "OpenAI", regex_filters: ["api.openai.com"], region_filters: [], allocation_policy: "BALANCED", routable_node_count: 1, sticky_ttl: "168h0m0s" }],
          total: 1, limit: 50, offset: 0,
        });
        if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
        if (cmd === "lease_map") return Promise.resolve([{ platform_id: "p1", account: "OpenAI.port-17990", egress_ip: "1.2.3.4", node_tag: "hk-01", target_domain: "api.openai.com", ts: "" }]);
        return Promise.resolve(undefined);
      });
      const { container } = render(<TopologyView />);
      await waitFor(() => {
        expect(container.textContent || "").toMatch(/OpenAI\.port-17990|1\.2\.3\.4/);
      });
    });
  });

  describe("C2-7: i18n.isInitialized gate re-renders after locale switch", () => {
    beforeAll(() => {
      i18next.addResourceBundle("zh", "translation", {
        topology: {
          entryPort: "入口代理端口",
          region: "区域: {{region}}",
          healthy: "健康",
          filters: "上游: {{filters}}",
          policy: "策略: {{policy}}",
          routable: "可路由: {{count}}",
          noNodes: "未加载节点。",
          noPlatforms: "没有平台。",
          noLeases: "无活跃租约",
          sidecarUnhealthy: "副作用作车不安全.",
          dragHint: "从平台拖至节点区域绑定路由",
        },
      });
    });
    afterAll(async () => { await i18next.changeLanguage("en"); });

    it("locale=en renders entry-port in English, then zh after changeLanguage('zh')", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({
          items: [{ id: "u1", name: "OpenAI", regex_filters: [], region_filters: ["hk"], allocation_policy: "BALANCED", routable_node_count: 0, sticky_ttl: "" }],
          total: 1, limit: 50, offset: 0,
        });
        if (cmd === "node_list") return Promise.resolve({
          items: [{ name: "hk-01", display_tag: "HK-01", has_outbound: true, failure_count: 0, tags: [{ tag: "HK" }] }],
          total: 1, limit: 500, offset: 0,
        });
        if (cmd === "lease_map") return Promise.resolve([]);
        if (cmd === "port_list") return Promise.resolve([]);
        return Promise.resolve(undefined);
      });
      await i18next.changeLanguage("en");
      render(<TopologyView />);
      expect(await screen.findByText(/Entry proxy port/i)).toBeInTheDocument();
      await i18next.changeLanguage("zh");
      expect(await screen.findByText(/入口代理端口/i)).toBeInTheDocument();
    });
  });

  // --- T13 closed-loop tests ---
  describe("T13-1: getSelectedRegions filters C column by platform region_filters", () => {
    it("returns set of lowercase regions from platform region_filters", () => {
      const platforms = [
        { id: "p1", name: "A", region_filters: ["HK", "JP"], allocation_policy: "BALANCED", routable_node_count: 0, sticky_ttl: "" },
      ];
      const regions = getSelectedRegions(platforms as any);
      expect(regions.size).toBe(2);
      expect(regions.has("hk")).toBe(true);
      expect(regions.has("jp")).toBe(true);
    });

    it("empty when no filters (all manual)", () => {
      const platforms = [
        { id: "p1", name: "A", region_filters: [], allocation_policy: "BALANCED", routable_node_count: 0, sticky_ttl: "" },
      ];
      const regions = getSelectedRegions(platforms as any);
      expect(regions.size).toBe(0);
    });
  });

  describe("T13-2: layoutNodesViaDagre assigns valid x/y positions", () => {
    it("assigns non-zero positions to all nodes when both nodes + edges", () => {
      const nodes: any[] = [
        { id: "a", position: { x: 0, y: 0 }, data: {} },
        { id: "b", position: { x: 0, y: 0 }, data: {} },
        { id: "c", position: { x: 0, y: 0 }, data: {} },
      ];
      const edges: any[] = [
        { id: "e1", source: "a", target: "b" },
        { id: "e2", source: "b", target: "c" },
      ];
      const result = layoutNodesViaDagre(nodes, edges);
      expect(result.length).toBe(3);
      for (const n of result) {
        expect(typeof n.position.x).toBe("number");
        expect(typeof n.position.y).toBe("number");
      }
      // at least one node should have non-zero x (LR layout should stack horizontally)
      const anyNonZero = result.some((n: any) => n.position.x !== 0 || n.position.y !== 0);
      expect(anyNonZero).toBe(true);
    });

    it("returns [] when nodes is empty", () => {
      const result = layoutNodesViaDagre([], []);
      expect(result.length).toBe(0);
    });
  });

  describe("T13-4: empty-state messages inside opacity gate (not flash)", () => {
    it("renders noPorts/noNodes/noPlatforms messages only after ready", async () => {
      invokeMock.mockImplementation(() => Promise.resolve(undefined));
      render(<TopologyView />);
      // before sync resolves (ready=false), opacity is 0 — messages inside gate div
      // after sync, messages visible (rendered into DOM even while opacity-0)
      await waitFor(() => {
        // at least one empty-state message should be in the DOM after ready
        const hints = document.querySelectorAll(".text-zinc-400, .text-zinc-500");
        expect(hints.length).toBeGreaterThan(0);
      });
    });
  });


  describe("T13-3: Handle style is fixed-position (not full-area overlay)", () => {
    it("fixedHandleStyle has bounded pixel dimensions, not 100%", () => {
      expect(typeof fixedHandleStyle.width).toBe("number");
      expect(typeof fixedHandleStyle.height).toBe("number");
      expect(fixedHandleStyle.width).toBeLessThan(50); // not full-area
      expect(fixedHandleStyle.height).toBeLessThan(50);
    });

    it("fixedHandleStyle is visible (no opacity:0)", () => {
      const opacity = (fixedHandleStyle as any).opacity;
      expect(opacity).not.toBe(0);
      expect(fixedHandleStyle.background).toBeDefined();
      expect(fixedHandleStyle.background).not.toBe("transparent");
      expect(fixedHandleStyle.background).not.toBe("");
    });
  });

  describe("T13-5: zustand shallow skip prevents re-render on same data", () => {
    it("setPlatforms with shallow-equal array returns empty update (no re-render)", () => {
      // Import the store directly from TopologyView module
      const store = useTopologyStore.getState();
      const item = { id: "p1", name: "A", region_filters: [], regex_filters: [], allocation_policy: "BALANCED", routable_node_count: 0, sticky_ttl: "" };
      const sameA = [item];
      const sameB = [item]; // same object reference → shallow equal
      // shallow-equal arrays should not cause state change
      store.setPlatforms(sameA);
      const stateAfterFirst = useTopologyStore.getState().platforms;
      store.setPlatforms(sameB);
      const stateAfterSecond = useTopologyStore.getState().platforms;
      // same reference means shallow skip worked
      expect(stateAfterFirst).toBe(stateAfterSecond);
    });
  });

});
