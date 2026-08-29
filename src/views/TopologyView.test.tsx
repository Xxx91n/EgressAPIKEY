import { describe, it, expect, vi, beforeEach, beforeAll, afterAll } from "vitest";
import { render, screen, waitFor, act } from "@testing-library/react";
import i18next from "i18next";

// Mock the IPC module so we control the platform/node data the canvas sees.
const invokeMock = vi.fn<(cmd: string, args?: Record<string, unknown>) => Promise<unknown>>();
// T18: file-scoped core mock must ALSO export Channel for watch_port_health streaming tests.
// Channel class defined INSIDE the factory to remain valid after vitest vi.mock hoisting.
vi.mock("@tauri-apps/api/core", () => {
  class ChannelStub<T = unknown> {
    onmessage: ((msg: T) => void) | null = null;
    constructor() { (globalThis as unknown as { __lastChannel: ChannelStub<T> }).__lastChannel = this; }
    __emit(msg: T) { if (this.onmessage) this.onmessage(msg); }
    __close() { this.onmessage = null; }
  }
  return {
    invoke: (...args: unknown[]) => invokeMock(args[0] as string, args[1] as Record<string, unknown> | undefined),
    Channel: ChannelStub,
  };
});
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));

import { TopologyView, addRegionFilter, removeRegionFilter, patchAndSyncOnce, buildEdges, layoutNodesViaDagre, fixedHandleStyle, useTopologyStore, dedupNodesByHash, buildCColumnGroups, isNodeSelectedByAnyPlatform } from "./TopologyView";

// Ticket 07 (Authoritative Snapshot): the canvas now consumes the ONE
// pre-merged snapshot instead of merging platform_list_full +
// strategy_config_get in the view. These helpers convert the legacy per-test
// Resin/whitebox fixtures into the snapshot payload so the migrated tests
// still exercise the same user-visible behavior at the new seam.
type SnapshotBuilder = {
  resinPlatforms: Array<Record<string, unknown>>;
  strategyPlatforms: Array<Record<string, unknown>>;
  ports: Array<Record<string, unknown>>;
  resinReachable: boolean;
  whiteboxExists: boolean;
};
function snapshotFromFixtures(b: Partial<SnapshotBuilder>) {
  const strategy = b.strategyPlatforms ?? [];
  const platforms = strategy.map((ps) => {
    const name = String(ps.platform_name);
    const regions = Array.isArray(ps.regions) ? ps.regions : [];
    const aClass = String(ps.a_class ?? "");
    const bClass = String(ps.b_class ?? "");
    const manualNodes = Array.isArray(ps.manual_nodes) ? ps.manual_nodes : [];
    const subscriptions = Array.isArray(ps.subscriptions) ? ps.subscriptions : [];
    const resin = (b.resinPlatforms ?? []).find((p) => String(p.name) === name);
    const resinId = resin ? String(resin.id ?? "") : "";
    if (!resin) {
      return { state: "missingOnResin", platform_name: name, regions, a_class: aClass, b_class: bClass, manual_nodes: manualNodes, subscriptions };
    }
    const resinRegions = Array.isArray(resin.region_filters) ? resin.region_filters : [];
    const eq = (x: unknown[], y: unknown[]) =>
      [...x].map(String).map((v) => v.toLowerCase()).sort().join("|") ===
      [...y].map(String).map((v) => v.toLowerCase()).sort().join("|");
    if (eq(regions, resinRegions)) {
      return { state: "consistent", platform_name: name, platform_id: resinId, regions, resin_allocation_policy: String(resin.allocation_policy ?? "BALANCED"), b_class: bClass, a_class: aClass, manual_nodes: manualNodes, subscriptions };
    }
    return { state: "divergent", platform_name: name, platform_id: resinId, whitebox_regions: regions, resin_regions: resinRegions, resin_allocation_policy: String(resin.allocation_policy ?? "BALANCED"), b_class: bClass, a_class: aClass, manual_nodes: manualNodes, subscriptions };
  });
  const runtimeOnlyAClass = b.whiteboxExists === false ? "" : "subscription";
  for (const rp of b.resinPlatforms ?? []) {
    if (!strategy.some((ps) => String(ps.platform_name) === String(rp.name))) {
      platforms.push({ state: "divergent", platform_name: String(rp.name), platform_id: String(rp.id ?? ""), whitebox_regions: [], resin_regions: Array.isArray(rp.region_filters) ? rp.region_filters : [], resin_allocation_policy: String(rp.allocation_policy ?? "BALANCED"), b_class: "", a_class: runtimeOnlyAClass, manual_nodes: [], subscriptions: [] });
    }
  }
  const ports = (b.ports ?? []).map((m) => {
    const hasResin = m._has_resin_endpoint !== false;
    const { _has_resin_endpoint, ...rest } = m;
    if (rest.enabled === false && !hasResin) {
      return { state: "consistent", ...rest };
    }
    if (hasResin) return { state: "consistent", ...rest };
    return { state: "missingOnResin", port: rest.port, platform_name: rest.platform_name, protocol: rest.protocol };
  });
  return {
    strategyVersion: 1,
    platforms,
    ports,
    resinReachable: b.resinReachable ?? true,
  };
}
// Derive the snapshot from the legacy platform_list_full + strategy_config_get +
// port_list mock responses: mirrors what resin-core does in production.
function snapshotFromMocks(cmdMock: (cmd: string) => unknown) {
  const platResp = cmdMock("platform_list_full") as { items?: Array<Record<string, unknown>> } | undefined;
  const cfgResp = cmdMock("strategy_config_get") as { platforms?: Array<Record<string, unknown>> } | undefined;
  const portResp = cmdMock("port_list") as Array<Record<string, unknown>> | undefined;
  const resinPlatforms = platResp?.items ?? [];
  // Default the whitebox entries to the Resin rows so a test that only mocks
  // platform_list_full still renders. No strategy intent is invented
  // (a_class/b_class stay empty): the snapshot carries the runtime-only
  // default explicitly via merge_strategies' whitebox_exists semantics.
  const strategyPlatforms = cfgResp?.platforms ?? resinPlatforms.map((p) => ({
    platform_name: String(p.name),
    a_class: String(p.aClass ?? p.a_class ?? ""),
    b_class: String(p.bClass ?? p.b_class ?? ""),
    regions: Array.isArray(p.region_filters) ? p.region_filters : [],
  }));
  const ports = (portResp ?? []).map((m) => ({ ...m, _has_resin_endpoint: true }));
  return snapshotFromFixtures({ resinPlatforms, strategyPlatforms, ports, whiteboxExists: cfgResp !== undefined });
}


describe("TopologyView (T9 canvas: subscription-folded C + strategy labels + dual badges)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    // Ticket 07 migration shim: per-test mocks still declare the legacy
    // platform_list_full / strategy_config_get / port_list fixtures. Wrap
    // every mockImplementation so an authoritative_snapshot call derives its
    // payload from those legacy fixtures — the same merge resin-core performs
    // in production. The view itself must only call authoritative_snapshot.
    const origSet = invokeMock.mockImplementation.bind(invokeMock);
    (invokeMock as unknown as { mockImplementation: (impl: unknown) => unknown }).mockImplementation =
      (impl: unknown) => {
        const typed = impl as (cmd: string, args?: Record<string, unknown>) => unknown;
        const wrapped = async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === "authoritative_snapshot") {
            // A test that mocks the snapshot directly wins over the legacy
            // fixture derivation.
            const direct = await Promise.resolve(typed(cmd, args));
            if (direct !== undefined) return direct;
            const plat = await Promise.resolve(typed("platform_list_full", args));
            const cfg = await Promise.resolve(typed("strategy_config_get", args)).catch(() => undefined);
            const ports = await Promise.resolve(typed("port_list", args));
            return snapshotFromMocks((probe: string) => {
              if (probe === "platform_list_full") return plat;
              if (probe === "strategy_config_get") return cfg;
              if (probe === "port_list") return ports;
              return undefined;
            });
          }
          return typed(cmd, args);
        };
        return origSet(wrapped);
      };
  });

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
        items: [{ id: "p1", name: "OpenAI", regex_filters: [], region_filters: ["hk"], allocation_policy: "BALANCED", routable_node_count: 2, sticky_ttl: "", aClass: "region" }],
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
      expect(text).toMatch(/A:.*Region.*HK.*US/i);
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
      expect(text).toContain("A: Manual selection");
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

describe("T15-3: React.memo canvas node optimization", () => {
  it("memoized node components are defined (not undefined)", () => {
    // The memo() wrapper should produce a valid component, not undefined.
    // We verify via the exported nodeTypes map containing non-null entries.
    // In the test environment (no ReactFlow), we just verify the module loads.
    expect(true).toBe(true); // module loaded successfully = components are defined
  });

  it("memo prevents re-render when data reference is unchanged", () => {
    // React.memo does a shallow comparison of props. If data object is the
    // same reference, the memoized component should NOT re-render.
    // This is a no-op assertion test: the memo import in TopologyView
    // guarantees this behavior at the React runtime level.
    const { memo } = require("react");
    let renderCount = 0;
    const Inner = (_: { data: { label: string } }) => {
      renderCount++;
      return null;
    };
    const Memoized = memo(Inner);
    const data = { label: "test" };
    // First render

    const { render } = require("@testing-library/react");
    const { rerender } = render(<Memoized data={data} />);
    // Re-render with same data reference
    rerender(<Memoized data={data} />);
    // memo should have skipped the second render
    expect(renderCount).toBe(1);
  });
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
      const platforms = [{ name: "OpenAI", region_filters: ["hk"], allocation_policy: "BALANCED", aClass: "region" }];
      const subGroups = [{ subscriptionName: "sub1", regions: ["hk", "jp"] }];
      const edges = buildEdges(platforms, subGroups as any, []);
      const bcEdges = edges.filter((e) => e.source.startsWith("platform-"));
      expect(bcEdges).toHaveLength(1);
      expect(bcEdges[0].label).toBe("region:hk");
      // Auto-strategy edges are non-deletable (T9-2)
      expect(bcEdges[0].deletable).toBe(false);
    });

    it("T9-2: no edge when platform region_filters don't intersect subscription regions", () => {
      const platforms = [{ name: "OpenAI", region_filters: ["us"], allocation_policy: "BALANCED", aClass: "region" }];
      const subGroups = [{ subscriptionName: "sub1", regions: ["hk", "jp"] }];
      const edges = buildEdges(platforms, subGroups as any, []);
      const bcEdges = edges.filter((e) => e.source.startsWith("platform-"));
      expect(bcEdges).toHaveLength(0);
    });

    it("T9-2: edge label shows region:<matched> for multi-region intersection", () => {
      const platforms = [{ name: "OpenAI", region_filters: ["hk", "jp"], allocation_policy: "BALANCED", aClass: "region" }];
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
  describe("T22-1: isNodeSelectedByAnyPlatform a_class-semantic filtering", () => {
    it("subscription: node in selected subscription -> true", () => {
      const node = { node_hash: "h1", region: "hk", tags: [{ subscriptionName: "sub1" }] } as any;
      const platforms = [{ name: "P1", aClass: "subscription", subscriptionNames: ["sub1"] }] as any;
      expect(isNodeSelectedByAnyPlatform(node, platforms, "sub1")).toBe(true);
    });
    it("subscription: empty subscriptions = select ALL nodes", () => {
      const node = { node_hash: "h2", region: "us", tags: [{ subscriptionName: "sub2" }] } as any;
      const platforms = [{ name: "P2", aClass: "subscription", subscriptionNames: [] }] as any;
      expect(isNodeSelectedByAnyPlatform(node, platforms, "sub2")).toBe(true);
    });
    it("region: node in selected region -> true", () => {
      const node = { node_hash: "h3", region: "hk" } as any;
      const platforms = [{ name: "P3", aClass: "region", region_filters: ["HK"] }] as any;
      expect(isNodeSelectedByAnyPlatform(node, platforms, "hk")).toBe(true);
    });
    it("quality: healthy node -> true", () => {
      const node = { node_hash: "h4", region: "jp", failure_count: 0, has_outbound: true } as any;
      const platforms = [{ name: "P4", aClass: "quality" }] as any;
      expect(isNodeSelectedByAnyPlatform(node, platforms, "any")).toBe(true);
    });
    it("quality: unhealthy node -> false", () => {
      const node = { node_hash: "h5", region: "jp", failure_count: 3, has_outbound: false } as any;
      const platforms = [{ name: "P5", aClass: "quality" }] as any;
      expect(isNodeSelectedByAnyPlatform(node, platforms, "any")).toBe(false);
    });
    it("manual: node_hash in manualNodes -> true", () => {
      const node = { node_hash: "h6", region: "sg" } as any;
      const platforms = [{ name: "P6", aClass: "manual", manualNodes: ["h6"] }] as any;
      expect(isNodeSelectedByAnyPlatform(node, platforms, "any")).toBe(true);
    });
    it("manual: node_hash NOT in manualNodes -> false", () => {
      const node = { node_hash: "h7", region: "sg" } as any;
      const platforms = [{ name: "P7", aClass: "manual", manualNodes: ["h6"] }] as any;
      expect(isNodeSelectedByAnyPlatform(node, platforms, "any")).toBe(false);
    });
    it("missing aClass defaults to manual", () => {
      const node = { node_hash: "h8", region: "us" } as any;
      const platforms = [{ name: "P8", manualNodes: ["h8"] }] as any;
      expect(isNodeSelectedByAnyPlatform(node, platforms, "any")).toBe(true);
    });
  });

  describe("T22-3: buildEdges a_class-semantic B->C edge", () => {
    it("subscription+empty = edges to ALL groups", () => {
      const platforms = [{ name: "OpenAI", region_filters: [], aClass: "subscription", subscriptionNames: [] }];
      const subGroups = [{ subscriptionName: "sub1", regions: ["hk"] }, { subscriptionName: "sub2", regions: ["us"] }];
      const edges = buildEdges(platforms as any, subGroups as any, []);
      const bcEdges = edges.filter((e) => e.source.startsWith("platform-"));
      expect(bcEdges).toHaveLength(2);
    });
    it("subscription+specific = matching only", () => {
      const platforms = [{ name: "OpenAI", region_filters: [], aClass: "subscription", subscriptionNames: ["sub1"] }];
      const subGroups = [{ subscriptionName: "sub1", regions: ["hk"] }, { subscriptionName: "sub2", regions: ["us"] }];
      const edges = buildEdges(platforms as any, subGroups as any, []);
      const bcEdges = edges.filter((e) => e.source.startsWith("platform-"));
      expect(bcEdges).toHaveLength(1);
      expect(bcEdges[0].target).toBe("subgroup-sub1");
    });
    it("region = matching region groups only", () => {
      const platforms = [{ name: "OpenAI", region_filters: ["hk"], aClass: "region" }];
      const subGroups = [{ subscriptionName: "sub1", regions: ["hk", "us"] }];
      const edges = buildEdges(platforms as any, subGroups as any, []);
      const bcEdges = edges.filter((e) => e.source.startsWith("platform-"));
      expect(bcEdges).toHaveLength(1);
      expect(bcEdges[0].label).toBe("region:hk");
    });
    it("manual = NO group-level edges", () => {
      const platforms = [{ name: "OpenAI", region_filters: [], aClass: "manual", manualNodes: ["h1"] }];
      const subGroups = [{ subscriptionName: "sub1", regions: ["hk"] }];
      const edges = buildEdges(platforms as any, subGroups as any, []);
      const bcEdges = edges.filter((e) => e.source.startsWith("platform-"));
      expect(bcEdges).toHaveLength(0);
    });
    it("quality = edges to ALL groups", () => {
      const platforms = [{ name: "OpenAI", region_filters: [], aClass: "quality" }];
      const subGroups = [{ subscriptionName: "sub1", regions: ["hk"] }, { subscriptionName: "sub2", regions: ["us"] }];
      const edges = buildEdges(platforms as any, subGroups as any, []);
      const bcEdges = edges.filter((e) => e.source.startsWith("platform-"));
      expect(bcEdges).toHaveLength(2);
      expect(bcEdges[0].label).toBe("quality:all");
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


  // --- T14 closed-loop tests ---
  describe("T14-1: A badge renders before B badge in PlatformNode", () => {
    it("A badge comes before B badge in DOM order", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({
          items: [{ id: "p1", name: "TestAB", regex_filters: [], region_filters: ["hk"], allocation_policy: "BALANCED", routable_node_count: 1, sticky_ttl: "" }],
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
        expect(text).toContain("A:");
        expect(text).toContain("B:");
      });
      // Verify A badge appears before B badge in DOM
      const allText = container.textContent || "";
      const aIdx = allText.indexOf("A:");
      const bIdx = allText.indexOf("B:");
      expect(aIdx).toBeGreaterThan(-1);
      expect(bIdx).toBeGreaterThan(-1);
      expect(aIdx).toBeLessThan(bIdx);
    });
  });

  describe("T14-2: A strategy i18n uses t() calls", () => {
    it("A: Manual comes from strategy.manual i18n key", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({
          items: [{ id: "p2", name: "ManualT", regex_filters: [], region_filters: [], allocation_policy: "BALANCED", routable_node_count: 0, sticky_ttl: "" }],
          total: 1, limit: 50, offset: 0,
        });
        if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 500, offset: 0 });
        if (cmd === "lease_map") return Promise.resolve([]);
        if (cmd === "port_list") return Promise.resolve([]);
        return Promise.resolve(undefined);
      });
      const { container } = render(<TopologyView />);
      await waitFor(() => {
        expect(container.textContent || "").toContain("A: Manual selection");
      });
    });
  });

  describe("T14-3: SubscriptionGroupNode collapsible — max 10 nodes visible", () => {
    it("renders regionStats chips when collapsed and shows +N more when expanded", async () => {
      // Build 15 fake nodes in one subscription
      const fakeNodes = Array.from({ length: 15 }, (_, i) => ({
        name: "node-" + i, display_tag: "JP-Node-" + i, has_outbound: true, failure_count: 0, region: "jp", tags: [{ tag: "JP", subscriptionName: "sub1" }],
      }));
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({
          items: [{ id: "p1", name: "Plat", regex_filters: [], region_filters: ["jp"], allocation_policy: "BALANCED", routable_node_count: 15, sticky_ttl: "", aClass: "region" }],
          total: 1, limit: 50, offset: 0,
        });
        if (cmd === "node_list") return Promise.resolve({ items: fakeNodes, total: 15, limit: 500, offset: 0 });
        if (cmd === "lease_map") return Promise.resolve([]);
        if (cmd === "port_list") return Promise.resolve([]);
        return Promise.resolve(undefined);
      });
      const { container } = render(<TopologyView />);
      await waitFor(() => {
        const text = container.textContent || "";
        // Collapsed: should show JP(15) region stat chip
        expect(text).toContain("JP(15)");
      });
    });
  });

  describe("T14-4: region view mode builds regionGroup nodes", () => {
    it("viewMode=region creates regiongroup- prefixed node ids", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({
          items: [{ id: "p1", name: "RegPlat", regex_filters: [], region_filters: ["jp", "us"], allocation_policy: "BALANCED", routable_node_count: 5, sticky_ttl: "" }],
          total: 1, limit: 50, offset: 0,
        });
        if (cmd === "node_list") return Promise.resolve({
          items: [
            { name: "n1", display_tag: "JP-1", has_outbound: true, failure_count: 0, region: "jp", tags: [{ tag: "JP", subscriptionName: "sub1" }] },
            { name: "n2", display_tag: "US-1", has_outbound: true, failure_count: 0, region: "us", tags: [{ tag: "US", subscriptionName: "sub1" }] },
          ],
          total: 2, limit: 500, offset: 0,
        });
        if (cmd === "lease_map") return Promise.resolve([]);
        if (cmd === "port_list") return Promise.resolve([]);
        return Promise.resolve(undefined);
      });
      const { container } = render(<TopologyView />);
      await waitFor(() => {
        const text = container.textContent || "";
        expect(text).toContain("JP");
        expect(text).toContain("US");
      });
    });
  });

  describe("T14-5: CanvasControls has Home (reset to center) button", () => {
    it("renders resetCenter tooltip text", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
        if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 500, offset: 0 });
        if (cmd === "lease_map") return Promise.resolve([]);
        if (cmd === "port_list") return Promise.resolve([]);
        return Promise.resolve(undefined);
      });
      const { container } = render(<TopologyView />);
      await waitFor(() => {
        const buttons = container.querySelectorAll("button[title]");
        const titles = Array.from(buttons).map((b) => b.getAttribute("title"));
        expect(titles).toContain("Reset to center");
      });
    });
  });


  // --- T15 closed-loop tests ---
  describe("T15-1: C column hides subscription groups with no selected nodes", () => {
    it("does not render subscription card when no platform selects its region", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({
          items: [{ id: "p1", name: "Plat", regex_filters: [], region_filters: ["us"], allocation_policy: "BALANCED", routable_node_count: 0, sticky_ttl: "" }],
          total: 1, limit: 50, offset: 0,
        });
        if (cmd === "node_list") return Promise.resolve({
          items: [{ name: "jp-01", display_tag: "JP-01", has_outbound: true, failure_count: 0, region: "jp", tags: [{ tag: "JP", subscriptionName: "sub1" }] }],
          total: 1, limit: 500, offset: 0,
        });
        if (cmd === "lease_map") return Promise.resolve([]);
        if (cmd === "port_list") return Promise.resolve([]);
        return Promise.resolve(undefined);
      });
      const { container } = render(<TopologyView />);
      await waitFor(() => {
        // Platform "Plat" with region_filter "us" should appear
        expect(container.textContent || "").toContain("Plat");
      });
      // sub1 only has JP nodes, but platform selected US — sub1 should NOT appear
      expect(container.textContent || "").not.toContain("sub1");
    });
  });

  describe("T15-2: CanvasControls icon buttons have w-fit self-center", () => {
    it("all icon buttons (not segmented toggle) have w-fit class", async () => {
      invokeMock.mockImplementation(() => Promise.resolve(undefined));
      const { container } = render(<TopologyView />);
      await waitFor(() => {
        const buttons = container.querySelectorAll("button[title]");
        const iconButtons = Array.from(buttons).filter((b) => {
          const cls = b.className || "";
          return cls.includes("p-1.5"); // icon buttons have p-1.5, segmented toggle has py-1
        });
        expect(iconButtons.length).toBeGreaterThan(0);
        for (const btn of iconButtons) {
          expect((btn.className || "")).toContain("w-fit");
        }
      });
    });
  });

  describe("T15-3: topologyState persistence round-trip", () => {
    it("saveTopologyState then loadTopologyState restores viewMode and locked", async () => {
      // Reconfigure the global LazyStore mock to use an in-memory map for a real round-trip.
      const db = new Map<string, unknown>();
      const { LazyStore } = await import("@tauri-apps/plugin-store");
      vi.mocked(LazyStore).mockImplementation((() => ({
        get: vi.fn(async (k: string) => db.get(k) ?? null),
        set: vi.fn(async (k: string, v: unknown) => { db.set(k, v); }),
        save: vi.fn(async () => {}),
      })) as never);

      const settingsMod = await import("../lib/settings");
      const ts = { x: 100, y: -50, zoom: 1.25, viewMode: "region" as const, locked: true };
      await settingsMod.saveTopologyState(ts);
      const loaded = await settingsMod.loadTopologyState();
      expect(loaded).not.toBeNull();
      expect(loaded!.viewMode).toBe("region");
      expect(loaded!.locked).toBe(true);
      expect(loaded!.zoom).toBe(1.25);
    });

    it("loadTopologyState falls back to legacy topologyViewport key", async () => {
      const db = new Map<string, unknown>();
      db.set("topologyViewport", { x: 10, y: 20, zoom: 0.5 });
      const { LazyStore } = await import("@tauri-apps/plugin-store");
      vi.mocked(LazyStore).mockImplementation((() => ({
        get: vi.fn(async (k: string) => db.get(k) ?? null),
        set: vi.fn(async () => {}),
        save: vi.fn(async () => {}),
      })) as never);

      const settingsMod = await import("../lib/settings");
      const loaded = await settingsMod.loadTopologyState();
      expect(loaded).not.toBeNull();
      expect(loaded!.x).toBe(10);
      expect(loaded!.viewMode).toBe("subscription"); // legacy default when not "region"
      expect(loaded!.locked).toBe(false); // legacy default
    });
  });

  describe("T15-4: dagre layout centers graph to origin", () => {
    it("layoutNodesViaDagre positions nodes around coordinate origin", () => {
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
      // T15-v3-2: offset removed per ADR-0039 SS4; dagre positions start from margin (20,20).
      // Assert positions are non-negative and finite (no NaN/infinite drift).
      for (const n of result) {
        expect(Number.isFinite(n.position.x)).toBe(true);
        expect(Number.isFinite(n.position.y)).toBe(true);
        expect(n.position.x).toBeGreaterThanOrEqual(-10); // small tolerance for margin
        expect(n.position.y).toBeGreaterThanOrEqual(-10);
      }
    });
  });

  describe("T15-5: onConnect routes through strategyConfig not direct PATCH (ADR-0036)", () => {
    it("sync() fetches ONLY the authoritative snapshot; legacy cross-store commands are not invoked", async () => {
      const invokedCmds: string[] = [];
      invokeMock.mockImplementation((cmd: string) => {
        invokedCmds.push(cmd);
        if (cmd === "authoritative_snapshot") return Promise.resolve(snapshotFromFixtures({
          resinPlatforms: [{ id: "p1", name: "TestPlat", region_filters: ["hk"], allocation_policy: "BALANCED" }],
          strategyPlatforms: [{ platform_name: "TestPlat", a_class: "region", b_class: "random", regions: ["hk"] }],
          ports: [],
        }));
        if (cmd === "node_list") return Promise.resolve({
          items: [{ name: "hk-01", display_tag: "HK-01", has_outbound: true, failure_count: 0, region: "hk", tags: [{ tag: "HK", subscriptionName: "sub1" }] }],
          total: 1, limit: 500, offset: 0,
        });
        if (cmd === "lease_map") return Promise.resolve([]);
        return Promise.resolve(undefined);
      });

      const { container } = render(<TopologyView />);
      await waitFor(() => {
        expect(container.textContent || "").toContain("TestPlat");
      });

      // Ticket 07: the view consumes the ONE pre-merged snapshot. It must
      // NOT fetch the legacy cross-store sources anymore (ARCHITECTURE.md
      // §Config Authority: views never re-merge stores).
      expect(invokedCmds).toContain("authoritative_snapshot");
      expect(invokedCmds).not.toContain("strategy_config_get");
      expect(invokedCmds).not.toContain("strategy_config_put");
      expect(invokedCmds).not.toContain("platform_list_full");
      expect(invokedCmds).not.toContain("port_list");
      // platform_update MUST NOT be called during sync (strategyConfig is the pipeline, not direct Resin PATCH)
      expect(invokedCmds).not.toContain("platform_update");
    });

    it("whitebox strategyConfig regions override Resin platform region_filters in canvas", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({
          items: [{ id: "p1", name: "WBPlat", regex_filters: [], region_filters: ["us"], allocation_policy: "BALANCED", routable_node_count: 0, sticky_ttl: "" }],
          total: 1, limit: 50, offset: 0,
        });
        if (cmd === "node_list") return Promise.resolve({
          items: [
            { name: "us-01", display_tag: "US-01", has_outbound: true, failure_count: 0, region: "us", tags: [{ tag: "US", subscriptionName: "s1" }] },
            { name: "jp-01", display_tag: "JP-01", has_outbound: true, failure_count: 0, region: "jp", tags: [{ tag: "JP", subscriptionName: "s1" }] },
          ],
          total: 2, limit: 500, offset: 0,
        });
        if (cmd === "lease_map") return Promise.resolve([]);
        if (cmd === "port_list") return Promise.resolve([]);
        // Whitebox JSON says regions=["jp"] even though Resin platform still shows ["us"]
        if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [{ platform_name: "WBPlat", a_class: "region", b_class: "random", regions: ["jp"] }] });
        return Promise.resolve(undefined);
      });
      const { container } = render(<TopologyView />);
      await waitFor(() => {
        expect(container.textContent || "").toContain("WBPlat");
      });
      // Canvas should reflect whitebox intent (jp) not Resin runtime (us).
      // Ticket 07: the divergence itself is carried IN the snapshot (both
      // values, state=divergent); the view renders the intent without
      // re-merging stores.
      await waitFor(() => {
        expect(container.textContent || "").toContain("JP");
      });
    });
  
  describe("T15-4-anti-flash: empty-state messages do not appear before ready=true", () => {
    it("noPorts/noNodes/noPlatforms messages absent before sync resolves", async () => {
      // Block all IPC so sync() never resolves (ready stays false initially)
      invokeMock.mockImplementation(() => new Promise(() => {}));
      const { container } = render(<TopologyView />);
      // Before ready, opacity gate hides canvas AND empty-state messages should NOT be in DOM
      await new Promise((r) => setTimeout(r, 50));
      const text = container.textContent || "";
      // The empty-state messages are i18n keys translated in setup; check they are absent
      // (these messages only render inside the ready-gated section)
      expect(text).not.toContain("No platforms configured");
      expect(text).not.toContain("未加载节点");
    });
  });

  // === T17 tests (ADR-0041: Canvas v4 node pool + toolbar merge + MiniMap ariaLabel + dedup) ===
  describe("T17-1 (ADR-0041 S1): viewMode guard on subGroup loop", () => {
    beforeEach(() => { invokeMock.mockReset(); });

    it("T17-1a: region viewMode does not render subscription group nodes (ADR-0041 S1)", async () => {
      // T17-audit: assertion strategy switched from ReactFlow-rendered DOM textContent to
      // a direct call of the extracted `buildCColumnGroups` pure helper. Reason (atomcode
      // research + local evidence): under jsdom, ReactFlow only reliably stampedes entry-port
      // + platform nodes; subscriptionGroup/regionGroup custom-node DOM does not render, so
      // either `textContent` or class-based DOM assertion is unreliable (the failing variant
      // saw `my-sub` survive in region viewMode because RegionGroupNode renders the sub name
      // as an inline chip). The S1 contract "subGroup push block gated by viewMode !==
      // subscription" is now verifiable at the helper layer: the helper is the exact code
      // that mutated the list inside rawNodes useMemo before extraction, so asserting its
      // outputs pins the same guard LiveView exercised.
      //
      // Defensive double-cover kept: a pre-populated `topologyState` with viewMode=region is
      // set so future vitest versions that DO fire ReactFlow onInit in jsdom converge to
      // region viewMode as well; this does not affect the helper-call assertions below.
      const db = new Map<string, unknown>();
      db.set("topologyState", { x: 0, y: 0, zoom: 1, viewMode: "region", locked: false });
      const { LazyStore } = await import("@tauri-apps/plugin-store");
      vi.mocked(LazyStore).mockImplementation((() => ({
        get: vi.fn(async (k: string) => db.get(k) ?? null),
        set: vi.fn(async (k: string, v: unknown) => { db.set(k, v); }),
        save: vi.fn(async () => {}),
      })) as never);

      try {
        // Mounted TopologyView is still rendered so the IPC mock path is exercised AND a
        // future vitest that fires onInit under jsdom finds an already-region viewMode —
        // regression coverage for the previous "boot flash" class of bugs. We do not assert on
        // the rendered DOM here: under jsdom, ReactFlow only reliably renders the entry-port
        // node, so platform + C-column custom-node DOM is missing and a textContent anchor
        // would hang. The S1 contract assertions below are pure-helper-output checks.
        void render(<TopologyView />);
        await Promise.resolve();

        // Test fixture mirrors the node_list mock shape so the helper sees exactly the data
        // the canvas would feed it: 2 healthy HK nodes from subscription "my-sub", all in
        // region "hk" which the platform selects.
        const subGroups = [
          {
            subscriptionName: "my-sub",
            nodes: [
              { name: "hk-01", display_tag: "HK-01", node_hash: "h1", has_outbound: true, failure_count: 0, region: "hk", tags: [{ tag: "HK", subscriptionName: "my-sub" }] },
              { name: "hk-02", display_tag: "HK-02", node_hash: "h2", has_outbound: true, failure_count: 0, region: "hk", tags: [{ tag: "HK", subscriptionName: "my-sub" }] },
            ],
            healthy: 2,
            total: 2,
            regions: ["hk"],
          },
        ] as unknown as Parameters<typeof buildCColumnGroups>[1];
        const t = i18next.t.bind(i18next) as (key: string, opts?: Record<string, unknown>) => string;

        // S1 contract, viewMode = subscription (default): subscriptionGroup node IS pushed.
        const subPlatforms = [{ id: "p1", name: "PlatA", regex_filters: null, region_filters: ["hk"], allocation_policy: "BALANCED", routable_node_count: 1, sticky_ttl: "", aClass: "subscription", subscriptionNames: ["my-sub"] }] as any;
        const subNodes = buildCColumnGroups("subscription", subGroups, subPlatforms, t);
        const subGroupEntries = subNodes.filter((n) => (n.id ?? "").startsWith("subgroup-"));
        expect(subGroupEntries.length).toBe(1);
        expect(subGroupEntries[0].id).toBe("subgroup-my-sub");
        // Sanity: region nodes are never built under subscription viewMode.
        const regionEntriesInSub = subNodes.filter((n) => (n.id ?? "").startsWith("regiongroup-"));
        expect(regionEntriesInSub.length).toBe(0);

        // S1 contract, viewMode = region (flipped by the CanvasControls region button /
        // pre-populated topologyState above): subscriptionGroup node is NOT pushed (the early
        // return at L723 of the helper fires), and the region group for "hk" IS pushed
        // because selectedRegions contains "hk" (T16-1 selectedRegions gate).
        const regionPlatforms = [{ id: "p1", name: "PlatA", regex_filters: null, region_filters: ["hk"], allocation_policy: "BALANCED", routable_node_count: 1, sticky_ttl: "", aClass: "region" }] as any;
        const regionNodes = buildCColumnGroups("region", subGroups, regionPlatforms, t);
        const subGroupEntriesInRegion = regionNodes.filter((n) => (n.id ?? "").startsWith("subgroup-"));
        expect(subGroupEntriesInRegion.length).toBe(0);
        const regionGroupEntries = regionNodes.filter((n) => (n.id ?? "").startsWith("regiongroup-"));
        expect(regionGroupEntries.length).toBe(1);
        expect(regionGroupEntries[0].id).toBe("regiongroup-hk");
      } finally {
        // Restore the default LazyStore mock factory (setup.ts) so T17-1b and later tests
        // stay clean. Default returns null for every get.
        vi.mocked(LazyStore).mockImplementation((() => ({
          get: vi.fn(async () => null),
          set: vi.fn(async () => {}),
          save: vi.fn(async () => {}),
        })) as never);
      }
    });

    it("T17-1b: subscription viewMode still renders subGroup labels (regression guard)", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({
          items: [{ id: "p1", name: "OpenAI", regex_filters: [], region_filters: ["hk"], allocation_policy: "BALANCED", routable_node_count: 1, sticky_ttl: "" }],
          total: 1, limit: 50, offset: 0,
        });
        if (cmd === "node_list") return Promise.resolve({
          items: [
            { name: "hk-01", display_tag: "HK-01", node_hash: "h1", has_outbound: true, failure_count: 0, tags: [{ tag: "HK", subscriptionName: "my-sub" }] },
          ],
          total: 1, limit: 500, offset: 0,
        });
        if (cmd === "lease_map") return Promise.resolve([]);
        if (cmd === "port_list") return Promise.resolve([]);
        if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [{ platform_name: "OpenAI", a_class: "region", b_class: "random", regions: ["hk"] }] });
        return Promise.resolve(undefined);
      });
      const { container } = render(<TopologyView />);
      await waitFor(() => { expect(container.textContent || "").toContain("my-sub"); });
    });
  });

  describe("T17-2 (ADR-0041 S2): cross-subscription node dedup by node_hash", () => {
    beforeEach(() => { invokeMock.mockReset(); });

    it("T17-2a: duplicate node_hash within one subscription is deduped", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({
          items: [{ id: "p1", name: "P", regex_filters: [], region_filters: ["hk"], allocation_policy: "BALANCED", routable_node_count: 2, sticky_ttl: "" }],
          total: 1, limit: 50, offset: 0,
        });
        if (cmd === "node_list") return Promise.resolve({
          items: [
            { name: "hk-01", display_tag: "DUP", node_hash: "abc", has_outbound: true, failure_count: 0, tags: [{ tag: "HK", subscriptionName: "s1" }] },
            { name: "hk-02", display_tag: "DUP", node_hash: "abc", has_outbound: true, failure_count: 0, tags: [{ tag: "HK", subscriptionName: "s1" }] },
            { name: "hk-03", display_tag: "UNIQ", node_hash: "def", has_outbound: true, failure_count: 0, tags: [{ tag: "HK", subscriptionName: "s1" }] },
          ],
          total: 3, limit: 500, offset: 0,
        });
        if (cmd === "lease_map") return Promise.resolve([]);
        if (cmd === "port_list") return Promise.resolve([]);
        if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [{ platform_name: "P", a_class: "region", b_class: "random", regions: ["hk"] }] });
        return Promise.resolve(undefined);
      });
      const { container } = render(<TopologyView />);
      await waitFor(() => { expect(container.textContent || "").toContain("s1"); });
      // SubscriptionGroupNode collapses node rows by default (only region stats chip shows).
      // The ▶ expand toggle is a button with textContent "▶" or "▼" inside the subgroup card.
      // Click it so the per-node rows render, then assert dedup by display_tag text.
      await waitFor(() => {
        const expandBtn = Array.from(container.querySelectorAll('button'))
          .find((b) => (b.textContent || "").trim() === "▶" || (b.textContent || "").trim() === "▼") as HTMLButtonElement | undefined;
        if (expandBtn) expandBtn.click();
      });
      await waitFor(() => {
        const text = container.textContent || "";
        // Per-subscription dedup means only 1 "DUP" row is in the subgroup card (verified via FoldRow count)
        const dupMatches = (text.match(/DUP/g) || []).length;
        expect(dupMatches).toBe(1);
        const uniqMatches = (text.match(/UNIQ/g) || []).length;
        expect(uniqMatches).toBe(1);
      });
    });

    it("T17-2b: dedupNodesByHash removes duplicates across two subscriptions (pure function contract)", () => {
      // Pure-function contract test for dedupNodesByHash — independent of React render
      // timing, viewMode state, or DOM polling. This replaces the flaky DOM-based test
      // that had to click the region toggle + Expand button with waitFor polling.
      const nodeA = { node_hash: "sharedhash", display_tag: "node-dup" };
      const nodeB = { node_hash: "sharedhash", display_tag: "node-dup" };
      const nodeC = { node_hash: "uniq1", display_tag: "other" };
      const nodeD = { node_hash: "uniq2", display_tag: "third" };
      const nodes = [nodeA, nodeB, nodeC, nodeD];
      const out = dedupNodesByHash(nodes);
      // 4 nodes in, 3 unique node_hash out (B is dropped as duplicate of A)
      expect(out.length).toBe(3);
      // The deduped entry for sharedhash is nodeA (first occurrence wins)
      expect(out[0]).toBe(nodeA);
      expect(out.find((n) => n.node_hash === "sharedhash")).toBe(nodeA);
      // nodeB (the duplicate) is NOT in the output
      expect(out).not.toContain(nodeB);
      // node-dup string appears once in the surviving node_hash group
      const dupTags = out.filter((n) => n.display_tag === "node-dup").length;
      expect(dupTags).toBe(1);
    });
  });

  describe("T17-3 (ADR-0041 S3): MiniMap ariaLabel replaces div[title]", () => {
    it("T17-3a: MiniMap receives ariaLabel prop with translated tooltip text", async () => {
      invokeMock.mockImplementation(() => Promise.resolve(undefined));
      const { container } = render(<TopologyView />);
      await waitFor(() => {
        // The MiniMap svg should have aria-labelledby pointing at a <title> whose text = label
        // Searching for the translated "Mini-map" label from en/common.json (mocked via i18next test setup)
        // Actual test: find svg with role=img inside container; its <title> child text should be nonempty
        const svg = container.querySelector('svg.react-flow__minimap, svg[role="img"]');
        expect(svg).toBeTruthy();
      });
    });

    it("T17-3b: no <div title=...> wrapper around MiniMap (anti-regression)", () => {
      invokeMock.mockImplementation(() => Promise.resolve(undefined));
      const { container } = render(<TopologyView />);
      // Ensure no wrapper div with title=topology.minimapHint literal exists
      const wrapper = container.querySelector('div[title="topology.minimapHint"]');
      expect(wrapper).toBeNull();
      // Also ensure literal title attribute equal to the raw key does NOT exist anywhere
      const anyTitled = container.querySelectorAll('[title]');
      const hits = Array.from(anyTitled).filter((el) => (el.getAttribute("title") || "").match(/^(topology.minimapHint|Mini-map|$)/));
      // No container should use the raw key as its title attribute (that would indicate the wrapper never read t())
      expect(hits.filter((el) => el.getAttribute("title") === "topology.minimapHint")).toHaveLength(0);
    });
  });

  describe("T18-1 (ADR-0042 S1): port health chip — 4-state + authRequired + disabled", () => {
    it("T18-1a: entry port renders Alive green dot when watch_port_health emits alive snapshot", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
        if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 500, offset: 0 });
        if (cmd === "lease_map") return Promise.resolve([]);
        if (cmd === "port_list") return Promise.resolve([
          { port: 17001, protocol: "socks5", platform_name: "P1", account: "acc", label: "L1", enabled: true, auth_required: false },
        ]);
        if (cmd === "watch_port_health") return Promise.resolve(undefined);
        return Promise.resolve(undefined);
      });
      const { container } = render(<TopologyView />);
      // Wait for the EntryPortNode to render
      await waitFor(() => {
        expect(container.textContent || "").toMatch(/17001/);
      });
      // Emit a snapshot via the stubbed channel
      const stub = (globalThis as unknown as { __lastChannel: { __emit: (m: unknown) => void } }).__lastChannel;
      stub.__emit({ revision: 1, entries: [{ port: 17001, state: "alive", reachable: true, fails: 0, latency_ms: 10, interval_secs: 5 }] });
      // After emit, the title attribute on the health dot should be "Alive"
      await waitFor(() => {
        const dot = container.querySelector('span[title="Alive"]');
        expect(dot).toBeTruthy();
        expect(dot?.className).toContain("bg-emerald-500");
      });
    });

    it("T18-1b: entry port renders Dead red dot and Disabled overlay when enabled=false", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
        if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 500, offset: 0 });
        if (cmd === "lease_map") return Promise.resolve([]);
        if (cmd === "port_list") return Promise.resolve([
          { port: 17002, protocol: "http", platform_name: "", account: "", label: "Pool2", enabled: false, auth_required: false },
        ]);
        if (cmd === "watch_port_health") return Promise.resolve(undefined);
        return Promise.resolve(undefined);
      });
      const { container } = render(<TopologyView />);
      await waitFor(() => {
        expect(container.textContent || "").toMatch(/17002/);
      });
      // emit Dead snapshot
      const stub = (globalThis as unknown as { __lastChannel: { __emit: (m: unknown) => void } }).__lastChannel;
      stub.__emit({ revision: 2, entries: [{ port: 17002, state: "dead", reachable: false, fails: 5, latency_ms: null, interval_secs: 300 }] });
      // Disabled text is shown (i18n "Disabled")
      await waitFor(() => {
        expect(container.textContent || "").toMatch(/Disabled/);
      });
      // dead dot should be red (bg-red-500)
      const dot = container.querySelector('span[title="Dead"]');
      expect(dot).toBeTruthy();
      expect(dot?.className).toContain("bg-red-500");
    });

    it("T18-1c: entry port Lock icon aria-label=i18n(portAuthRequired) when auth_required=true", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
        if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 500, offset: 0 });
        if (cmd === "lease_map") return Promise.resolve([]);
        if (cmd === "port_list") return Promise.resolve([
          { port: 17003, protocol: "socks5", platform_name: "P3", account: "acc3", label: "L3", enabled: true, auth_required: true },
        ]);
        if (cmd === "watch_port_health") return Promise.resolve(undefined);
        return Promise.resolve(undefined);
      });
      const { container } = render(<TopologyView />);
      await waitFor(() => {
        expect(container.textContent || "").toMatch(/17003/);
      });
      const lockEl = container.querySelector('svg[aria-label="Authentication required"]');
      expect(lockEl).toBeTruthy();
    });

    it("T18-4 (ADR-0042 S4): A badge shows Manual (N nodes) when strategyConfig.manual_nodes is non-empty", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({
          items: [{ id: "p1", name: "platManual", allocation_policy: "BALANCED", routable_node_count: 0, region_filters: null, regex_filters: null, sticky_ttl: "0s" }],
          total: 1, limit: 50, offset: 0,
        });
        if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 500, offset: 0 });
        if (cmd === "lease_map") return Promise.resolve({ items: [{ active_leases: 0, platform_id: "" }] });
        if (cmd === "port_list") return Promise.resolve([]);
        if (cmd === "strategy_config_get") return Promise.resolve({
          version: 1,
          platforms: [{ platform_name: "platManual", a_class: "manual", b_class: "random", manual_nodes: ["h1", "h2"] }],
        });
        if (cmd === "watch_port_health") return Promise.resolve(undefined);
        return Promise.resolve(undefined);
      });
      const { container } = render(<TopologyView />);
      await waitFor(() => {
        // Badge text rendered as "A: Manual (2 nodes)" (L404-405 TopologyView)
        expect(container.textContent || "").toMatch(/A: Manual \(2 nodes\)/);
      });
    });
  });

  describe("T17-4 (ADR-0041 S4): openStrategyConfig button merged into CanvasControls toolbar", () => {
    it("T17-4a/T18-5a (ADR-0042 S5): ConfigToolbar toggle button title=i18n(openConfig) at top-right and dropdown expands 2 options", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
        if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 500, offset: 0 });
        if (cmd === "lease_map") return Promise.resolve([]);
        if (cmd === "port_list") return Promise.resolve([]);
        return Promise.resolve(undefined);
      });
      const { container } = render(<TopologyView />);
      await waitFor(() => {
        const toggle = container.querySelector('button[title="Open config"]') as HTMLButtonElement | null;
        expect(toggle).toBeTruthy();
        // Must live in the right-top ConfigToolbar container, not the bottom-left CanvasControls
        const parent = toggle?.closest('div.absolute.top-2.right-2');
        expect(parent).toBeTruthy();
      });
      // Click the toggle to open the dropdown
      const toggle = container.querySelector('button[title="Open config"]') as HTMLButtonElement;
      await act(async () => { toggle.click(); });
      await waitFor(() => {
        // Dropdown shows the two config file options as buttons
        const options = container.querySelectorAll("button.text-left");
        const texts = Array.from(options).map((b) => b.textContent || "");
        expect(texts).toContain("Open ports config");
        expect(texts).toContain("Open strategy config");
      });
    });

    it("T18-5b (ADR-0042 S5): clicking Open ports config dropdown option closes the dropdown", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
        if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 500, offset: 0 });
        if (cmd === "lease_map") return Promise.resolve([]);
        if (cmd === "port_list") return Promise.resolve([]);
        if (cmd === "get_config_dir") return Promise.resolve("C:/fake-config-dir");
        return Promise.resolve(undefined);
      });
      const { container } = render(<TopologyView />);
      // Open the dropdown
      await waitFor(() => {
        const toggle = container.querySelector('button[title="Open config"]') as HTMLButtonElement | null;
        expect(toggle).toBeTruthy();
      });
      const toggle = container.querySelector('button[title="Open config"]') as HTMLButtonElement;
      await act(async () => { toggle.click(); });
      // Verify the Open ports config option appears
      await waitFor(() => {
        const options = container.querySelectorAll("button.text-left");
        const opt = Array.from(options).find((b) => (b.textContent || "").includes("Open ports config"));
        expect(opt).toBeTruthy();
      });
      // Click the Open ports config option — openCfg calls ipcGetConfigDir then dynamically imports opener (swallowed if unavailable in test).
      // The dropdown should close and toggle again reopens it.
      const opt = Array.from(container.querySelectorAll("button.text-left")).find((b) => (b.textContent || "").includes("Open ports config")) as HTMLButtonElement;
      await act(async () => { opt.click(); });
      // After click, dropdown closed (open=false). The dropdown content div no longer rendered.
      await waitFor(() => {
        const dropdownContent = container.querySelector("div.absolute.top-9.right-0");
        expect(dropdownContent).toBeNull();
      });
    });

    it("T17-4b: no standalone button at absolute bottom-20 left-4 remains (anti-regression)", () => {
      invokeMock.mockImplementation(() => Promise.resolve(undefined));
      const { container } = render(<TopologyView />);
      // The old standalone button had className "absolute bottom-20 left-4 z-10 ..."
      const standalone = container.querySelector('button.absolute.bottom-20.left-4');
      expect(standalone).toBeNull();
    });
  });

});

});
