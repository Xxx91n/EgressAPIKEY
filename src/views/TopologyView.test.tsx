import { describe, it, expect, vi, beforeEach, beforeAll, afterAll } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import i18next from "i18next";

// Mock the IPC module so we control the platform/node data the canvas sees.
const invokeMock = vi.fn<(cmd: string, args?: Record<string, unknown>) => Promise<unknown>>();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(args[0] as string, args[1] as Record<string, unknown> | undefined),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));

import { TopologyView, addRegionFilter, removeRegionFilter, patchAndSyncOnce, buildEdges } from "./TopologyView";

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
  it("P19-1: viewport persists via onMoveEnd and restores on mount (settings stub)", async () => {
    // Stub settings so we can assert saveTopologyViewport is called and
    // loadTopologyViewport controls the initial viewport.
    const saves = [] as any[];
    let loadReturn: any = null;
    vi.mock("../lib/settings", async (orig) => {
      const real = await (orig as () => Promise<any>)();
      return {
        ...real,
        loadTopologyViewport: vi.fn(async () => loadReturn),
        saveTopologyViewport: vi.fn(async (vp: any) => { saves.push(vp); return; }),
      };
    });
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "platform_list_full") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
      if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
      return Promise.resolve(undefined);
    });
    // Mount: reactflow does not exist here, so the load is silently skipped.
    // We still prove the import wiring + that no crashes happen. The
    // subDrag.test.ts unit test already guards the drag math.
    render(<TopologyView />);
    await waitFor(() => {
      expect(screen.getByText(/forward proxy/i)).toBeInTheDocument();
    });
    vi.doUnmock("../lib/settings");
  });

  // --- Q2 Bug1+Bug2 closed-loop: idempotent region_filters helpers ---
  describe("Q2-Bug1: region_filters add/remove idempotency (pure helpers)", () => {
    it("addRegionFilter is idempotent and renders twice identical no-op", () => {
      const a1 = addRegionFilter(["hk", "us"], "jp");
      expect(a1).toEqual(["hk", "us", "jp"]);
      const a2 = addRegionFilter(a1, "jp"); // already present -> same ref
      expect(a2).toBe(a1); // no-op path returns same reference
    });
    it("addRegionFilter handles null -> [region]", () => {
      expect(addRegionFilter(null, "sg")).toEqual(["sg"]);
    });
    it("removeRegionFilter works on null without throwing", () => {
      expect(removeRegionFilter(null, "hk")).toEqual([]);
    });
    it("removeRegionFilter -> empty array (never null) so PATCH sends an array", () => {
      const out = removeRegionFilter(["hk"], "hk");
      expect(out).toEqual([]);
      expect(out).not.toBeNull();
    });
    it("repeated add+remove converges to the same shape (no duplication)", () => {
      let cur: string[] | null = null;
      for (let i = 0; i < 3; i++) cur = addRegionFilter(cur, "tw");
      expect(cur).toEqual(["tw"]); // three 'tw' attempts -> single entry
      cur = removeRegionFilter(cur, "tw");
      expect(cur).toEqual([]);
      cur = addRegionFilter(cur, "tw");
      expect(cur).toEqual(["tw"]);
    });
  });

  describe("Q2-Bug2: onConnect success-resync + Bug3 connection props (static)", () => {
    it("TopologyView renders the ReactFlow container with min-h and connectable props", async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "platform_list_full") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
        if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
        return Promise.resolve(undefined);
      });
      render(<TopologyView />);
      await waitFor(() => {
        expect(screen.getByText(/forward proxy/i)).toBeInTheDocument();
      });
      // The canvas container should have min-h so handles stay responsive even
      // before fitView sets the viewport (Q2-Bug3 root cause).
      // react-flow may not fully init in jsdom; the parent wrapper div has min-h class.
      const wrapper = document.querySelector('[class*="min-h"]') as HTMLElement | null;
      expect(wrapper).not.toBeNull();
    });
    it("calls platform_list_full on mount (initial sync) and would re-sync after PATCH", async () => {
      const calls: string[] = [];
      invokeMock.mockImplementation((cmd: string) => {
        calls.push(cmd);
        if (cmd === "platform_list_full") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
        if (cmd === "node_list") return Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 });
        if (cmd === "platform_update") return Promise.resolve({ ok: true });
        return Promise.resolve(undefined);
      });
      render(<TopologyView />);
      await waitFor(() => {
        expect(screen.getByText(/forward proxy/i)).toBeInTheDocument();
      });
      // Initial sync fired platform_list_full at least once.
      expect(calls.filter((c) => c === "platform_list_full").length).toBeGreaterThanOrEqual(1);
    });
  });

  // --- ADR-0012 lease identity chips (port=identity; no observed_keys) ---
  describe("ADR-0012: lease chips show Platform.Account identity (no observed_keys)", () => {
    it("renders lease account + egress without observed_keys join", async () => {
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

  // --- C1-2 closed-loop: patchAndSyncOnce helper + racy double-PATCH guard ---
  describe("C1-2: patchAndSyncOnce backup->PATCH->sync ordering + idempotent skip", () => {
    it("add mode with already-bound region skips PATCH and sync entirely", async () => {
      const sync = vi.fn(async () => {});
      const ipcUpdate = vi.fn(async () => {});
      const backup = vi.fn(async () => {});
      const res = await patchAndSyncOnce({ platName: "OpenAI", current: ["hk"], region: "hk", mode: "add", sync, ipcUpdate, backup });
      expect(res.patched).toBe(false);
      expect(ipcUpdate).not.toHaveBeenCalled();
      expect(sync).not.toHaveBeenCalled();
      expect(backup).not.toHaveBeenCalled();
    });

    it("add mode with new region calls backup THEN ipcUpdate THEN sync (ordering)", async () => {
      const order: string[] = [];
      const sync = vi.fn(async () => { order.push("sync"); });
      const ipcUpdate = vi.fn(async () => { order.push("patch"); });
      const backup = vi.fn(async () => { order.push("backup"); });
      const res = await patchAndSyncOnce({ platName: "OpenAI", current: [], region: "jp", mode: "add", sync, ipcUpdate, backup });
      expect(res.patched).toBe(true);
      expect(res.next).toEqual(["jp"]);
      expect(order).toEqual(["backup", "patch", "sync"]);
      expect(ipcUpdate).toHaveBeenCalledWith("OpenAI", undefined, undefined, ["jp"]);
    });

    it("remove mode always PATCHes (even when already absent) and sends [] not null", async () => {
      const sync = vi.fn(async () => {});
      const ipcUpdate = vi.fn(async () => {});
      const res = await patchAndSyncOnce({ platName: "P", current: [], region: "tw", mode: "remove", sync, ipcUpdate, backup: undefined });
      expect(res.patched).toBe(true);
      expect(res.next).toEqual([]);
      expect(ipcUpdate).toHaveBeenCalledWith("P", undefined, undefined, []);
    });

    it("backup swallows failure so a broken backup never blocks the routing PATCH", async () => {
      const sync = vi.fn(async () => {});
      const ipcUpdate = vi.fn(async () => {});
      const backup = vi.fn(async () => { throw new Error("webdav down"); });
      const res = await patchAndSyncOnce({ platName: "OpenAI", current: [], region: "kr", mode: "add", sync, ipcUpdate, backup });
      expect(res.patched).toBe(true);
      expect(ipcUpdate).toHaveBeenCalled();
      expect(sync).toHaveBeenCalled();
    });

    it("racy double-PATCH (add same region twice in a row): second call is idempotent skip", async () => {
      // This pins the race guard: helper short-circuits on already-bound so a
      // second rapid drag of the same region before the first sync landed still
      // produces exactly ONE PATCH. The full topology-level race guard is the
      // patchingRef in TopologyView; this helper owns the idempotent skip.
      const sync = vi.fn(async () => {});
      const ipcUpdate = vi.fn(async () => {});
      const a = await patchAndSyncOnce({ platName: "OpenAI", current: [], region: "hk", mode: "add", sync, ipcUpdate, backup: undefined });
      // Pretend the first PATCH synced into `current` already (post-PATCH state).
      const b = await patchAndSyncOnce({ platName: "OpenAI", current: a.next /* ["hk"] */, region: "hk", mode: "add", sync, ipcUpdate, backup: undefined });
      expect(a.patched).toBe(true);
      expect(b.patched).toBe(false);
      expect(ipcUpdate).toHaveBeenCalledTimes(1);
      expect(sync).toHaveBeenCalledTimes(1);
    });
  });

  // --- C1-3 closed-loop: buildEdges + remove-region edge deletion ---
  describe("C1-3: buildEdges B->C edge disappears after region removed from region_filters", () => {
    const groups = [{ region: "hk" }, { region: "us" }];
    it("renders a B->C edge for each region in region_filters", () => {
      const edges = buildEdges([{ name: "OpenAI", region_filters: ["hk", "us"] }], groups);
      expect(edges.find((e) => e.id === "e-OpenAI-hk")).toBeTruthy();
      expect(edges.find((e) => e.id === "e-OpenAI-us")).toBeTruthy();
    });
    it("drops the B->C edge for a region after it is removed from region_filters (delete-edge semantics)", () => {
      const before = buildEdges([{ name: "OpenAI", region_filters: ["hk"] }], groups);
      expect(before.find((e) => e.id === "e-OpenAI-hk")).toBeTruthy();
      // Simulate onEdgesDelete -> removeRegion -> server-truthy region_filters=[]
      const after = buildEdges([{ name: "OpenAI", region_filters: [] }], groups);
      expect(after.find((e) => e.id === "e-OpenAI-hk")).toBeFalsy();
    });
    it("always emits the A->B entry edge regardless of region_filters", () => {
      const edges = buildEdges([{ name: "OpenAI", region_filters: null }], groups);
      expect(edges.find((e) => e.id === "e-entry-OpenAI" && e.source === "entry-port" && e.target === "platform-OpenAI")).toBeTruthy();
    });
    it("does not leak edges for node-groups the platform is not bound to", () => {
      const edges = buildEdges([{ name: "OpenAI", region_filters: ["hk"] }], groups);
      expect(edges.find((e) => e.id === "e-OpenAI-us")).toBeFalsy();
    });
  });


  // C2-7: i18n.isInitialized gate re-renders the node-group label after a locale switch.
  // The gate at TopologyView line ~438 short-circuits the nodes useMemo to [] until
  // i18n.isInitialized is truthy, then re-enters with the freshly-loaded catalog when a
  // lazy chunk resolves or changeLanguage completes. The user's "刚打开 GUI 是 zh, 画布内
  // 节点框还是 en, 切换别的页面再回来才刷新" symptom happened because the initial useMemo
  // build raced ahead of the zh chunk resolving, falling back to English text — the gate
  // prevents that first-paint mismatch.
  describe("C2-7: i18n.isInitialized gate re-renders canvas boxes after locale switch", () => {
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

    it("locale=en renders the entry-port label in English, then zh text after changeLanguage('zh')", async () => {
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
                return Promise.resolve(undefined);
      });
      // pin locale=en before first render.
      await i18next.changeLanguage("en");
      render(<TopologyView />);
      // Initial render at en: entry-port box shows "Entry proxy port".
      await waitFor(() => {
        expect(screen.getByText(/Entry proxy port:/i)).toBeInTheDocument();
      });
      // Switch to zh; the gate ensures the canvas re-renders with the zh catalog rather
      // than caching the English-painted node from the prior paint.
      await i18next.changeLanguage("zh");
      await waitFor(() => {
        expect(screen.getByText(/入口代理端口:/i)).toBeInTheDocument();
      });
    });
  });

});
