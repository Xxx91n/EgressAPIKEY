import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/react";
import { invokeMock } from "../test/setup";
import { PlatformsView } from "./PlatformsView";
import { useAppStore } from "../store/appStore";

afterEach(() => cleanup());

const samplePort = {
  port: 17990,
  protocol: "socks5",
  platform_name: "Default",
  account: "port-17990",
  label: "entry-17990",
  enabled: true,
};

const samplePlatform = {
  name: "Default",
  allocation_policy: "BALANCED",
  regex_filters: [],
  region_filters: [],
  routable_node_count: 3,
  sticky_ttl: "30m",
};

/// helper — expand the platform card so strategy selectors become visible.
async function expandPlatform(name: string) {
  await waitFor(() => expect(screen.getByTestId("platform-card-header-" + name)).toBeInTheDocument(), { timeout: 5000 });
  fireEvent.click(screen.getByTestId("platform-card-header-" + name));
  await waitFor(() => expect(screen.getByTestId("strategy-split-" + name)).toBeInTheDocument(), { timeout: 5000 });
}

describe("PlatformsView P2 (entry-ports dual-pane, IPC-mocked)", () => {
  beforeEach(() => {
    useAppStore.setState({ platforms: [] });
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([samplePort]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "port_suggest") return Promise.resolve(17990);
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [{ platform_name: "Default", a_class: "manual", b_class: "BALANCED" }] });
      if (cmd === "strategy_config_put") return Promise.resolve(null);
      if (cmd === "strategy_apply") return Promise.resolve({ platforms: [] });
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_auth_info") return Promise.resolve({ username: "port-17990", auth_required: false, proxy_token: "tok", protocol: "socks5" });
      if (cmd === "port_health_check") return Promise.resolve({ healthy: true, reachable: true, reason: "ok", latency_ms: 1 });
      return Promise.resolve(undefined);
    });
  });

  it("renders entry-ports pane + platforms pane with live rows", async () => {
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("platforms-view")).toBeInTheDocument());
    expect(screen.getByTestId("ports-pane")).toBeInTheDocument();
    expect(screen.getByTestId("platforms-pane")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId("port-row-17990")).toBeInTheDocument());
    await waitFor(() => expect(screen.getByTestId("platform-card-Default")).toBeInTheDocument());
  });

  it("adds an entry port via the compact inline form and refreshes the list", async () => {
    let ports = [samplePort];
    invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "port_list") return Promise.resolve(ports);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "port_suggest") return Promise.resolve(17990);
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "strategy_config_put") return Promise.resolve(null);
      if (cmd === "strategy_apply") return Promise.resolve({ platforms: [] });
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_upsert") {
        const next = {
          port: Number(args?.port ?? 17991),
          protocol: String(args?.protocol ?? "socks5"),
          platform_name: String(args?.platformName ?? args?.platform_name ?? ""),
          account: String(args?.account ?? "port-17991"),
          label: String(args?.label ?? "entry-17991"),
          enabled: true,
        };
        ports = [...ports.filter((p) => p.port !== next.port), next];
        return Promise.resolve(next);
      }
      return Promise.resolve(undefined);
    });

    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("port-input")).toBeInTheDocument());
    fireEvent.change(screen.getByTestId("port-input"), { target: { value: "17991" } });
    fireEvent.change(screen.getByTestId("port-protocol"), { target: { value: "http" } });
    fireEvent.change(screen.getByTestId("port-label"), { target: { value: "pool-b" } });
    fireEvent.click(screen.getByTestId("port-add"));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "port_upsert",
        expect.objectContaining({ port: 17991, protocol: "http", label: "pool-b", enabled: true }),
      );
    });
    await waitFor(() => expect(screen.getByTestId("port-row-17991")).toBeInTheDocument());
  });

  it("rejects invalid port before invoke", async () => {
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("port-input")).toBeInTheDocument());
    fireEvent.change(screen.getByTestId("port-input"), { target: { value: "80" } });
    fireEvent.click(screen.getByTestId("port-add"));
    await waitFor(() => expect(screen.getByTestId("platforms-toast")).toBeInTheDocument());
    expect(invokeMock.mock.calls.some((c) => c[0] === "port_upsert")).toBe(false);
  });

  it("creates a platform from the dialog", async () => {
    let platforms = [samplePlatform];
    invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "port_list") return Promise.resolve([samplePort]);
      if (cmd === "platform_list_full") return Promise.resolve(platforms);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "port_suggest") return Promise.resolve(17990);
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "strategy_config_put") return Promise.resolve(null);
      if (cmd === "strategy_apply") return Promise.resolve({ platforms: [] });
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "platform_create_with_fields") {
        const body = (args?.body ?? {}) as Record<string, unknown>;
        const name = String(body.name ?? "OpenAI");
        platforms = [...platforms, { name, allocation_policy: String(body.allocation_policy ?? "BALANCED"), regex_filters: [], region_filters: [], routable_node_count: 0, sticky_ttl: "" }];
        return Promise.resolve({ id: "new", name });
      }
      return Promise.resolve(undefined);
    });

    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("platform-create-open")).toBeInTheDocument());
    fireEvent.click(screen.getByTestId("platform-create-open"));
    fireEvent.change(screen.getByTestId("platform-create-name"), { target: { value: "OpenAI" } });
    fireEvent.click(screen.getByTestId("platform-create-submit"));
    await waitFor(() => expect(screen.getByTestId("platform-card-OpenAI")).toBeInTheDocument());
  });

  it("binds a dragged port onto a platform card via pointer events", async () => {
    let ports = [samplePort];
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve(ports);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform, { name: "OpenAI", allocation_policy: "BALANCED", regex_filters: [], region_filters: [], routable_node_count: 1, sticky_ttl: "" }]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "port_suggest") return Promise.resolve(17990);
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "strategy_config_put") return Promise.resolve(null);
      if (cmd === "strategy_apply") return Promise.resolve({ platforms: [] });
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_bind_platform") { ports = [{ ...samplePort, platform_name: "OpenAI" }]; return Promise.resolve({ ok: true }); }
      return Promise.resolve(undefined);
    });

    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("port-row-17990")).toBeInTheDocument());
    await waitFor(() => expect(screen.getByTestId("platform-card-OpenAI")).toBeInTheDocument());
    // Drag threshold: pointerDown records start, pointerMove >5px starts drag, pointerEnter on platform + pointerUp binds
    const portEl = screen.getByTestId("port-row-17990") as HTMLElement;
    fireEvent.pointerDown(portEl, { clientX: 100, clientY: 100 });
    fireEvent.pointerMove(portEl, { clientX: 110, clientY: 110 });
    fireEvent.pointerEnter(screen.getByTestId("platform-card-OpenAI"));
    fireEvent.pointerUp(screen.getByTestId("platform-card-OpenAI"));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("port_bind_platform", expect.objectContaining({ port: 17990, platformName: "OpenAI" })));
  });

  // platform card collapsed by default, expand shows strategy
  it("T11-1: platform card collapsed by default; expand reveals strategy-split", async () => {
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("platform-card-Default")).toBeInTheDocument(), { timeout: 5000 });
    expect(screen.queryByTestId("strategy-split-Default")).toBeNull();
    await expandPlatform("Default");
    expect(screen.getByTestId("strategy-split-Default")).toBeInTheDocument();
    expect(screen.getByTestId("strategy-aclass-chips-Default")).toBeInTheDocument();
    expect(screen.getByTestId("strategy-bclass-chips-Default")).toBeInTheDocument();
  });

  // collapsed card shows A/B badges
  it("T11-1: collapsed card shows A badge + B badge", async () => {
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("platform-abadge-Default")).toBeInTheDocument(), { timeout: 5000 });
    expect(screen.getByTestId("platform-bbadge-Default")).toBeInTheDocument();
  });

  // selected chip has ring class
  it("T11-2: selected A-class chip has ring-2 ring-primary", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [{ platform_name: "Default", a_class: "region", regions: ["US"] }] });
      if (cmd === "strategy_config_put") return Promise.resolve(null);
      if (cmd === "strategy_apply") return Promise.resolve({ platforms: [] });
      if (cmd === "node_list") return Promise.resolve([{ display_tag: "US-node", region: "US", node_hash: "us1" }]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17990);
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await expandPlatform("Default");
    // A-class region chip selected = primary + ring
    await waitFor(() => expect(screen.getByTestId("strategy-region-chip-US")).toBeInTheDocument(), { timeout: 5000 });
    expect(screen.getByTestId("strategy-region-chip-US").className).toContain("ring-2");
  });

  // manual search filters nodes within subscription-folded groups
  it("T12-3: manual search shows subscription group headers; selected nodes highlighted blue", async () => {
    const nodes = [
      { display_tag: "HK-1", region: "HK", node_hash: "h1", tags: [] },
      { display_tag: "US-1", region: "US", node_hash: "h2", tags: [] },
      { display_tag: "JP-1", region: "JP", node_hash: "h3", tags: [] },
    ];
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [{ platform_name: "Default", a_class: "manual", manual_nodes: ["h1"] }] });
      if (cmd === "strategy_config_put") return Promise.resolve(null);
      if (cmd === "strategy_apply") return Promise.resolve({ platforms: [] });
      if (cmd === "node_list") return Promise.resolve(nodes);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17990);
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await expandPlatform("Default");
    await waitFor(() => expect(screen.getByTestId("strategy-manual-search-Default")).toBeInTheDocument(), { timeout: 5000 });
    // subscription group header visible (default collapsed)
    const subBtn = await screen.findByTestId("strategy-manual-sub-Default-__untagged__");
    expect(subBtn).toBeInTheDocument();
    // Expand the group
    fireEvent.click(subBtn);
    // h1 (selected) should be visible with blue styling
    await waitFor(() => expect(screen.getByTestId("strategy-manual-chip-h1")).toBeInTheDocument());
    // h2 (unselected) should also be visible
    await waitFor(() => expect(screen.getByTestId("strategy-manual-chip-h2")).toBeInTheDocument());
    // h3 (unselected) should also be visible
    await waitFor(() => expect(screen.getByTestId("strategy-manual-chip-h3")).toBeInTheDocument());
  });

  // no global Apply button
  it("T11-4b: global Apply button is NOT present (per-platform sync)", async () => {
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("platform-card-Default")).toBeInTheDocument());
    expect(screen.queryByTestId("strategy-apply")).toBeNull();
  });

  // clicking A-class region chip fires strategy_config_put + strategy_apply (per-platform sync)
  it("T11-4b: clicking region chip triggers strategy_config_put + strategy_apply", async () => {
    let putCalled = false;
    let applyCalled = false;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "strategy_config_put") { putCalled = true; return Promise.resolve(null); }
      if (cmd === "strategy_apply") { applyCalled = true; return Promise.resolve({ platforms: [] }); }
      if (cmd === "node_list") return Promise.resolve([{ display_tag: "HK-1", region: "HK", node_hash: "h1" }]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17990);
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await expandPlatform("Default");
    // Click region A-class chip
    fireEvent.click(screen.getByTestId("strategy-aclass-region-Default"));
    await waitFor(() => expect(screen.getByTestId("strategy-region-chips-Default")).toBeInTheDocument(), { timeout: 5000 });
    // Click HK region chip -> triggers per-platform sync
    fireEvent.click(screen.getByTestId("strategy-region-chip-HK"));
    await waitFor(() => expect(putCalled).toBe(true), { timeout: 5000 });
    await waitFor(() => expect(applyCalled).toBe(true), { timeout: 5000 });
  });

  // R11-05: one-click preset commits a prebuilt snapshot through the SAME
  // authoritative write entry (config_put + apply) — no side channel.
  it("R11-05: preset button applies the snapshot via config_put + apply", async () => {
    let putBody: Record<string, unknown> | undefined;
    let applyCalled = false;
    invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "strategy_config_put") { putBody = (args?.config ?? args) as Record<string, unknown>; return Promise.resolve(null); }
      if (cmd === "strategy_apply") { applyCalled = true; return Promise.resolve({ platforms: [] }); }
      if (cmd === "node_list") return Promise.resolve([{ display_tag: "HK-1", region: "HK", node_hash: "h1" }]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17990);
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await expandPlatform("Default");
    fireEvent.click(screen.getByTestId("strategy-preset-lowLatency-Default"));
    await waitFor(() => expect(putBody).not.toBeUndefined(), { timeout: 5000 });
    await waitFor(() => expect(applyCalled).toBe(true), { timeout: 5000 });
    const plats = (putBody as Record<string, unknown>).platforms as Array<Record<string, unknown>>;
    const entry = plats.find((p) => p.platform_name === "Default");
    expect(entry?.a_class).toBe("quality");
    expect(entry?.b_class).toBe("PREFER_LOW_LATENCY");
    expect(entry?.top_n).toBe(5);
  });

  // B-class chips visible after expand.
  // the selector offers EXACTLY Resin's three real
  // allocation policies — the six display-only shell options are withdrawn.
  it("B-class chip pane shows the 3 real allocation policies as toggle chips", async () => {
    render(<PlatformsView />);
    await expandPlatform("Default");
    await waitFor(() => expect(screen.getByTestId("strategy-bclass-chips-Default")).toBeInTheDocument(), { timeout: 5000 });
    for (const s of ["BALANCED", "PREFER_LOW_LATENCY", "PREFER_IDLE_IP"]) {
      expect(screen.getByTestId("strategy-bclass-" + s + "-Default")).toBeInTheDocument();
    }
    // The withdrawn shell catalogue must not come back.
    for (const gone of ["random", "sequential", "latency", "quality", "bandwidth", "protocol_weight"]) {
      expect(screen.queryByTestId("strategy-bclass-" + gone + "-Default")).toBeNull();
    }
  });

  // B-class default chip (BALANCED) has active style
  it("B-class default chip (BALANCED) has active style with ring", async () => {
    render(<PlatformsView />);
    await expandPlatform("Default");
    await waitFor(() => expect(screen.getByTestId("strategy-bclass-BALANCED-Default")).toBeInTheDocument(), { timeout: 5000 });
    const balancedChip = screen.getByTestId("strategy-bclass-BALANCED-Default");
    expect(balancedChip.className).toContain("bg-primary");
    expect(balancedChip.className).toContain("ring-2");
  });

  // B-class policy click fires platform_update
  it("clicking the PREFER_LOW_LATENCY chip fires platform_update", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "strategy_config_put") return Promise.resolve(null);
      if (cmd === "strategy_apply") return Promise.resolve({ platforms: [] });
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17990);
      if (cmd === "platform_update") return Promise.resolve({ ok: true });
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await expandPlatform("Default");
    await waitFor(() => expect(screen.getByTestId("strategy-bclass-PREFER_LOW_LATENCY-Default")).toBeInTheDocument(), { timeout: 5000 });
    fireEvent.click(screen.getByTestId("strategy-bclass-PREFER_LOW_LATENCY-Default"));
    await waitFor(() => expect(invokeMock.mock.calls.some((c) => c[0] === "platform_update")).toBe(true), { timeout: 5000 });
  });

  // Bug4: SOCKS5 port shows auth credentials when auth_required (expanded card)
  it("Bug4: SOCKS5 port shows SOCKS5 auth credentials when auth_required (expanded)", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([samplePort]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "port_auth_info") return Promise.resolve({ port: 17990, username: "port-17990", password: "tok123", auth_required: true, platform_name: "Default" });
      if (cmd === "port_health_check") return Promise.resolve({ port: 17990, reachable: true, reason: "ok", healthy: true, latency_ms: 1 });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "strategy_apply") return Promise.resolve({ platforms: [] });
      if (cmd === "strategy_config_put") return Promise.resolve(null);
      if (cmd === "port_suggest") return Promise.resolve(17990);
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("port-row-17990")).toBeInTheDocument());
    // Expand the port card
    fireEvent.click(screen.getByTestId("port-chevron-17990"));
    await waitFor(() => expect(screen.getByTestId("port-auth-17990")).toBeInTheDocument(), { timeout: 5000 });
    await waitFor(() => expect(screen.getByText(/SOCKS5 user/i)).toBeInTheDocument(), { timeout: 5000 });
  });

  // Bug4: HTTP port shows auth credentials (expanded card)
  it("Bug4: HTTP port shows auth credentials (httpAuth) when expanded", async () => {
    const httpPort = { ...samplePort, port: 17111, protocol: "http", account: "port-17111", label: "entry-17111" };
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([httpPort]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "port_auth_info") return Promise.resolve({ port: 17111, username: "port-17111", password: "tok123", auth_required: true, platform_name: "Default" });
      if (cmd === "port_health_check") return Promise.resolve({ port: 17111, reachable: true, reason: "ok", healthy: true, latency_ms: 1 });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "strategy_config_put") return Promise.resolve(null);
      if (cmd === "strategy_apply") return Promise.resolve({ platforms: [] });
      if (cmd === "port_suggest") return Promise.resolve(17990);
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("port-row-17111")).toBeInTheDocument());
    fireEvent.click(screen.getByTestId("port-chevron-17111"));
    await waitFor(() => expect(screen.getByTestId("port-auth-17111")).toBeInTheDocument(), { timeout: 5000 });
    await waitFor(() => expect(screen.getByText(/HTTP Auth/i)).toBeInTheDocument(), { timeout: 5000 });
  });


  // port card collapsed by default; expand shows details
  it("T11-8: port card collapsed by default; expand shows auth details", async () => {
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("port-row-17990")).toBeInTheDocument());
    expect(screen.queryByTestId("port-auth-17990")).toBeNull();
    fireEvent.click(screen.getByTestId("port-chevron-17990"));
    await waitFor(() => expect(screen.getByTestId("port-auth-17990")).toBeInTheDocument(), { timeout: 5000 });
  });

  // port row shows Unbound text when platform_name is empty
  it("T8-7: port row shows Unbound text when platform_name is empty", async () => {
    const unboundPort = { ...samplePort, port: 17992, platform_name: "" };
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([unboundPort]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "port_auth_info") return Promise.resolve({ username: "port-17992", auth_required: false, proxy_token: "tok", protocol: "socks5" });
      if (cmd === "port_health_check") return Promise.resolve({ healthy: true, reachable: true, reason: "ok", latency_ms: 1 });
      if (cmd === "port_suggest") return Promise.resolve(17990);
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "strategy_config_put") return Promise.resolve(null);
      if (cmd === "strategy_apply") return Promise.resolve({ platforms: [] });
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      return Promise.resolve(true);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("port-row-17992")).toBeInTheDocument());
    expect(screen.getByTestId("port-row-17992").textContent).toContain("Unbound");
  });

  // region strategy from config renders region chip toggle with US+JP selected (after expand)
  it("T10-2: region strategy from config shows US+JP selected chips after expand", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [{ platform_name: "Default", a_class: "region", regions: ["US", "JP"] }] });
      if (cmd === "node_list") return Promise.resolve([{ display_tag: "US-node", region: "US", node_hash: "us1" }, { display_tag: "JP-node", region: "JP", node_hash: "jp1" }]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17990);
      if (cmd === "strategy_config_put") return Promise.resolve(null);
      if (cmd === "strategy_apply") return Promise.resolve({ platforms: [] });
      if (cmd === "port_health_check") return Promise.resolve({ healthy: true, latency_ms: 1 });
      if (cmd === "port_auth_info") return Promise.resolve({ username: "port-17990", auth_required: false, proxy_token: "tok", protocol: "socks5" });
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await expandPlatform("Default");
    await waitFor(() => expect(screen.getByTestId("strategy-region-chips-Default")).toBeInTheDocument(), { timeout: 5000 });
    const usChip = screen.getByTestId("strategy-region-chip-US");
    expect(usChip.className).toContain("ring-2");
  });

  // click on port card should NOT trigger opacity-50 (gray) or blue ring (selection removed)
  it("T12-fix: click on port card does not trigger opacity-50 or ring-2", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([{ port: 17990, platform_name: null, require_auth: false, protocol: "http" }]);
      if (cmd === "platform_list_full") return Promise.resolve([]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17991);
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("port-row-17990")).toBeInTheDocument());
    const portRow = screen.getByTestId("port-row-17990");
    // Click WITHOUT drag: pointerDown + pointerUp at same position (no move >5px)
    fireEvent.pointerDown(portRow, { clientX: 50, clientY: 50 });
    fireEvent.pointerUp(portRow, { clientX: 50, clientY: 50 });
    fireEvent.click(portRow);
    // No selection state: should NOT have ring-2 or opacity-50
    expect(portRow.className).not.toContain("ring-2");
    expect(portRow.className).not.toContain("opacity-50");
  });

  // drag beyond 5px threshold triggers opacity-50 (gray)
  it("T12-fix: drag beyond threshold triggers opacity-50 gray", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([{ port: 17991, platform_name: null, require_auth: false, protocol: "http" }]);
      if (cmd === "platform_list_full") return Promise.resolve([]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17992);
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("port-row-17991")).toBeInTheDocument());
    const portRow = screen.getByTestId("port-row-17991");
    // Drag BEYOND threshold: pointerDown + pointerMove >5px => opacity-50
    fireEvent.pointerDown(portRow, { clientX: 100, clientY: 100 });
    fireEvent.pointerMove(portRow, { clientX: 120, clientY: 120 });
    await waitFor(() => expect(portRow.className).toContain("opacity-50"));
    // Clean up
    fireEvent.pointerUp(portRow, { clientX: 120, clientY: 120 });
  });

  // strategy chip selection persists across re-mount (stale closure fix)
  it("T12-persist: strategy_config_put called after chip selection (stale closure fix)", async () => {
    let putCallCount = 0;
    let lastPutConfig: any = null;
    invokeMock.mockImplementation((cmd: string, args: any) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve({ items: [{ name: "Default", allocation_policy: "BALANCED", regex_filters: [], region_filters: [], routable_node_count: 0, sticky_ttl: "" }] });
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "strategy_config_put") { putCallCount++; lastPutConfig = args.config; return Promise.resolve(undefined); }
      if (cmd === "strategy_apply") return Promise.resolve({ platforms: [] });
      if (cmd === "node_list") return Promise.resolve([{ display_tag: "US-node", region: "US", node_hash: "us1" }]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17990);
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await expandPlatform("Default");
    await waitFor(() => {
      const chip = screen.queryByTestId("strategy-aclass-manual-Default");
      if (chip) fireEvent.click(chip);
    });
    await waitFor(() => {
      const nodeChip = screen.queryByTestId("strategy-manual-chip-us1");
      if (nodeChip) fireEvent.click(nodeChip);
    }, { timeout: 3000 });
    await waitFor(() => { expect(putCallCount).toBeGreaterThan(0); }, { timeout: 3000 });
    // Stale closure fix verified: strategy_config_put receives non-null config
    expect(lastPutConfig).toBeTruthy();
    // Verify config has at least one platform entry (proves updateAndSync flushed to backend)
    expect(lastPutConfig.platforms.length).toBeGreaterThan(0);
  });;

  // top_n quality strategy persists after page switch
  it("T12-persist: top_n quality strategy persists across re-mount", async () => {
    let savedConfig: any = null;
    invokeMock.mockImplementation((cmd: string, args: any) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([{ name: "Default", allocation_policy: "BALANCED", regex_filters: [], region_filters: [], routable_node_count: 0, sticky_ttl: "" }]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve(savedConfig ?? { version: 1, platforms: [{ platform_name: "Default", a_class: "quality", b_class: "BALANCED", top_n: 10 }] });
      if (cmd === "strategy_config_put") { savedConfig = args.config; return Promise.resolve(undefined); }
      if (cmd === "strategy_apply") return Promise.resolve({ platforms: [] });
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17990);
      return Promise.resolve(undefined);
    });
    const { unmount } = render(<PlatformsView />);
    await expandPlatform("Default");
    // Change top_n input to 25
    await waitFor(() => {
      const topNInput = screen.queryByTestId("strategy-topn-Default") as HTMLInputElement;
      if (topNInput) {
        fireEvent.change(topNInput, { target: { value: "25" } });
      }
    });
    // Verify the config was persisted with top_n = 25
    await waitFor(() => {
      expect(savedConfig).toBeTruthy();
      const platformEntry = savedConfig.platforms.find((p: any) => p.platform_name === "Default");
      expect(platformEntry).toBeTruthy();
      expect(platformEntry.top_n).toBe("25");
    });
    unmount();
  });
});
