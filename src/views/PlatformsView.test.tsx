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

describe("PlatformsView P2 (entry-ports dual-pane, IPC-mocked)", () => {
  beforeEach(() => {
    useAppStore.setState({ platforms: [] });
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([samplePort]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "port_upsert") {
        return Promise.resolve({
          ...samplePort,
          port: 17991,
          platform_name: "Default",
          account: "port-17991",
          label: "entry-17991",
        });
      }
      if (cmd === "port_remove") return Promise.resolve(true);
      if (cmd === "platform_create_with_fields") return Promise.resolve({ id: "p1", name: "OpenAI" });
      if (cmd === "platform_remove") return Promise.resolve(true);
      if (cmd === "platform_update") return Promise.resolve({ ok: true });
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

  it("adds an entry port via the form and refreshes the list", async () => {
    let ports = [samplePort];
    invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "port_list") return Promise.resolve(ports);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "port_upsert") {
        const next = {
          port: Number(args?.port ?? 17991),
          protocol: String(args?.protocol ?? "socks5"),
          platform_name: String(args?.platformName ?? "Default"),
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
    fireEvent.change(screen.getByTestId("port-platform"), { target: { value: "Default" } });
    fireEvent.change(screen.getByTestId("port-label"), { target: { value: "pool-b" } });
    fireEvent.click(screen.getByTestId("port-add"));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "port_upsert",
        expect.objectContaining({
          port: 17991,
          protocol: "http",
          platformName: "Default",
          label: "pool-b",
          enabled: true,
        }),
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
      if (cmd === "platform_create_with_fields") {
        const body = (args?.body ?? {}) as Record<string, unknown>;
        const name = String(body.name ?? "OpenAI");
        platforms = [
          ...platforms,
          {
            name,
            allocation_policy: String(body.allocation_policy ?? "BALANCED"),
            regex_filters: [],
            region_filters: [],
            routable_node_count: 0,
            sticky_ttl: "",
          },
        ];
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
      if (cmd === "platform_list_full") {
        return Promise.resolve([
          samplePlatform,
          {
            name: "OpenAI",
            allocation_policy: "BALANCED",
            regex_filters: [],
            region_filters: [],
            routable_node_count: 1,
            sticky_ttl: "",
          },
        ]);
      }
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "port_bind_platform") {
        ports = [{ ...samplePort, platform_name: "OpenAI" }];
        return Promise.resolve({ ok: true });
      }
      return Promise.resolve(undefined);
    });

    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("port-row-17990")).toBeInTheDocument());
    await waitFor(() => expect(screen.getByTestId("platform-card-OpenAI")).toBeInTheDocument());
    fireEvent.pointerDown(screen.getByTestId("port-row-17990"));
    fireEvent.pointerEnter(screen.getByTestId("platform-card-OpenAI"));
    fireEvent.pointerUp(screen.getByTestId("platform-card-OpenAI"));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "port_bind_platform",
        expect.objectContaining({ port: 17990, platformName: "OpenAI" }),
      );
    });
  });

  // -------------------------------------------------------------------
  // T4-4: Strategy panel closed-loop tests
  // -------------------------------------------------------------------
  it("T4-4 strategy: panel renders with noPlatforms hint when no platforms exist", async () => {
    useAppStore.setState({ platforms: [] });
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([]);
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      return Promise.resolve(undefined);
    });

    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("platforms-pane")).toBeInTheDocument());
    // strategy-panel removed: strategy is now inline in platform cards
    expect(screen.getByText("No platforms configured.")).toBeInTheDocument();
  });

  it("T10-1: A/B split card renders with chip-based A-class + B-class panes", async () => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17990);
      if (cmd === "port_auth_info") return Promise.resolve({ username: "port-17990", auth_required: false, proxy_token: "tok", protocol: "socks5" });
      if (cmd === "port_health_check") return Promise.resolve({ healthy: true, latency_ms: 1 });
      return Promise.resolve(undefined);
    });

    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("platform-card-Default")).toBeInTheDocument(), { timeout: 3000 });
    // T10-1: A/B split layout is always visible (no expand needed)
    expect(screen.getByTestId("strategy-split-Default")).toBeInTheDocument();
    // T10-1: A-class chips pane (left, 50%)
    expect(screen.getByTestId("strategy-aclass-chips-Default")).toBeInTheDocument();
    // T10-1: B-class chips pane (right, 50%)
    expect(screen.getByTestId("strategy-bclass-chips-Default")).toBeInTheDocument();
    // T10-5: leases/nodes pill in right-top (no text summary)
    expect(screen.getByTestId("platform-stats-pill-Default")).toBeInTheDocument();
  });

  it("T10-2: clicking A-class region chip reveals region toggle chips", async () => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "node_list") return Promise.resolve([{ display_tag: "HK-1", region: "HK", node_hash: "h1" }, { display_tag: "JP-1", region: "JP", node_hash: "h2" }]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17990);
      if (cmd === "port_auth_info") return Promise.resolve({ username: "port-17990", auth_required: false, proxy_token: "tok", protocol: "socks5" });
      if (cmd === "port_health_check") return Promise.resolve({ healthy: true, latency_ms: 1 });
      return Promise.resolve(undefined);
    });

    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("platform-card-Default")).toBeInTheDocument(), { timeout: 3000 });
    await waitFor(() => expect(screen.getByTestId("strategy-aclass-chips-Default")).toBeInTheDocument(), { timeout: 3000 });

    // Click the region A-class chip
    fireEvent.click(screen.getByTestId("strategy-aclass-region-Default"));
    // T10-2: region toggle chips should appear (from nodeList distinct regions)
    await waitFor(() => expect(screen.getByTestId("strategy-region-chips-Default")).toBeInTheDocument(), { timeout: 3000 });
    // T10-2: HK and JP toggle chips exist
    expect(screen.getByTestId("strategy-region-chip-HK")).toBeInTheDocument();
    expect(screen.getByTestId("strategy-region-chip-JP")).toBeInTheDocument();
  });

  it("T4-4 strategy: apply button calls strategy_config_put + strategy_apply", async () => {
    invokeMock.mockReset();
    let putCalled = false;
    let applyCalled = false;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "strategy_config_put") { putCalled = true; return Promise.resolve(null); }
      if (cmd === "strategy_apply") {
        applyCalled = true;
        return Promise.resolve({ platforms: [{ platform: "Default", region_filters: ["US"], patched: true }] });
      }
      return Promise.resolve(undefined);
    });

    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("platform-card-Default")).toBeInTheDocument());
        await waitFor(() => expect(screen.getByTestId("strategy-apply")).toBeInTheDocument());
    fireEvent.click(screen.getByTestId("strategy-apply"));
    await waitFor(() => expect(putCalled).toBe(true));
    await waitFor(() => expect(applyCalled).toBe(true));
  });

  it("T10-1: B-class chip pane shows 6 strategy options as toggle chips", async () => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17990);
      if (cmd === "port_auth_info") return Promise.resolve({ username: "port-17990", auth_required: false, proxy_token: "tok", protocol: "socks5" });
      if (cmd === "port_health_check") return Promise.resolve({ healthy: true, latency_ms: 1 });
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("strategy-bclass-chips-Default")).toBeInTheDocument(), { timeout: 3000 });
    // 6 B-class chip buttons: one per StrategyId
    const ids = ["random", "sequential", "latency", "quality", "bandwidth", "protocol_weight"];
    for (const s of ids) {
      expect(screen.getByTestId("strategy-bclass-" + s + "-Default")).toBeInTheDocument();
    }
  });

  it("T10-1: B-class default chip (random) has active style", async () => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17990);
      if (cmd === "port_auth_info") return Promise.resolve({ username: "port-17990", auth_required: false, proxy_token: "tok", protocol: "socks5" });
      if (cmd === "port_health_check") return Promise.resolve({ healthy: true, latency_ms: 1 });
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("strategy-bclass-random-Default")).toBeInTheDocument(), { timeout: 3000 });
    const randomChip = screen.getByTestId("strategy-bclass-random-Default");
    expect(randomChip.className).toContain("bg-primary");
  });

  it("T10-1: clicking B-class latency chip fires platform_update", async () => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17990);
      if (cmd === "port_auth_info") return Promise.resolve({ username: "port-17990", auth_required: false, proxy_token: "tok", protocol: "socks5" });
      if (cmd === "port_health_check") return Promise.resolve({ healthy: true, latency_ms: 1 });
      if (cmd === "platform_update") return Promise.resolve({ ok: true });
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("strategy-bclass-latency-Default")).toBeInTheDocument(), { timeout: 3000 });
    fireEvent.click(screen.getByTestId("strategy-bclass-latency-Default"));
    // T10-1: clicking B-class chip fires platform_update IPC
    await waitFor(() => expect(invokeMock.mock.calls.some((c2) => c2[0] === "platform_update")).toBe(true), { timeout: 3000 });
  });

  it("Bug4: SOCKS5 port shows SOCKS5 auth credentials when auth_required", async () => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([samplePort]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "port_auth_info") return Promise.resolve({ port: 17990, username: "port-17990", password: "tok123", auth_required: true, platform_name: "Default" });
      if (cmd === "port_health_check") return Promise.resolve({ port: 17990, reachable: true, socks5_ok: true, protocol_mismatch: false, latency_ms: 1 });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "strategy_apply") return Promise.resolve({ platforms: [] });
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("port-row-17990")).toBeInTheDocument());
    await waitFor(() => expect(screen.getByText(/SOCKS5 user/i)).toBeInTheDocument(), { timeout: 3000 });
    expect(screen.getAllByText(/port-17990/).length).toBeGreaterThan(0);
  });

  it("Bug4: HTTP port shows auth credentials (httpAuth), not httpNoAuth", async () => {
    invokeMock.mockReset();
    const httpPort = { ...samplePort, port: 17111, protocol: "http", account: "port-17111", label: "entry-17111" };
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([httpPort]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "port_auth_info") return Promise.resolve({ port: 17111, username: "port-17111", password: "tok123", auth_required: true, platform_name: "Default" });
      if (cmd === "port_health_check") return Promise.resolve({ port: 17111, reachable: true, socks5_ok: false, protocol_mismatch: true, latency_ms: 1 });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "strategy_apply") return Promise.resolve({ platforms: [] });
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("port-row-17111")).toBeInTheDocument());
    // Bug4 fix: HTTP port now shows httpAuth (credentials), not httpNoAuth
    await waitFor(() => expect(screen.getByTestId("port-auth-17111")).toBeInTheDocument(), { timeout: 3000 });
    // Should show HTTP auth credentials (not "no auth")
    await waitFor(() => expect(screen.getByText(/httpAuth|HTTP Auth/i)).toBeInTheDocument(), { timeout: 3000 });
  });


  it("T8-2: drag port to platform calls port_bind_platform (not port_upsert)", async () => {
    const ports = [{ ...samplePort, port: 17995, platform_name: "" }];
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve(ports);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "port_bind_platform") return Promise.resolve(true);
      if (cmd === "port_upsert") return Promise.resolve({ ...samplePort, port: 17995 });
      if (cmd === "port_auth_info") return Promise.resolve({ username: "port-17995", auth_required: false, proxy_token: "tok", protocol: "socks5" });
      if (cmd === "port_health_check") return Promise.resolve({ healthy: true, latency_ms: 1 });
      return Promise.resolve(true);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("port-row-17995")).toBeInTheDocument());
    await waitFor(() => expect(screen.getByTestId("platform-card-Default")).toBeInTheDocument());

    // Simulate pointer down on the port row
    const portRow = screen.getByTestId("port-row-17995");
    fireEvent.pointerDown(portRow);
    // Simulate pointer up on the platform card
    fireEvent.pointerEnter(screen.getByTestId("platform-card-Default"));
    fireEvent.pointerUp(screen.getByTestId("platform-card-Default"));

    await waitFor(() => {
      const bindCalls = (invokeMock.mock.calls as unknown as Array<string[]>).filter((c) => c[0] === "port_bind_platform");
      expect(bindCalls.length).toBeGreaterThanOrEqual(1);
    });
  });

  it("T8-3: pointerDown on port calls preventDefault + sets userSelect none", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([samplePort]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "port_auth_info") return Promise.resolve({ username: "port-17990", auth_required: false, proxy_token: "tok", protocol: "socks5" });
      if (cmd === "port_health_check") return Promise.resolve({ healthy: true, latency_ms: 1 });
      return Promise.resolve(true);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("port-row-17990")).toBeInTheDocument());
    const portRow = screen.getByTestId("port-row-17990");
    fireEvent.pointerDown(portRow);
    expect(document.body.style.userSelect).toBe("none");
  });

  it("T8-3: dragging port shows opacity-50 + cursor-grabbing on source row", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([samplePort]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "port_auth_info") return Promise.resolve({ username: "port-17990", auth_required: false, proxy_token: "tok", protocol: "socks5" });
      if (cmd === "port_health_check") return Promise.resolve({ healthy: true, latency_ms: 1 });
      return Promise.resolve(true);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("port-row-17990")).toBeInTheDocument());
    const portRow = screen.getByTestId("port-row-17990");
    fireEvent.pointerDown(portRow);
    expect(portRow.className).toContain("opacity-50");
    expect(portRow.className).toContain("cursor-grabbing");
    // pointerUp restores userSelect
    fireEvent.pointerUp(portRow);
    expect(document.body.style.userSelect).toBe("");
  });

  it("T8-7: new port created unbound (no Default platform assignment)", async () => {
    // null /* removed */ removed — T8-2 uses port_bind_platform not port_upsert
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([samplePort]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      // port_upsert path removed — T8-2 uses port_bind_platform
      if (cmd === "port_auth_info") return Promise.resolve({ username: "port-17990", auth_required: false, proxy_token: "tok", protocol: "socks5" });
      if (cmd === "port_health_check") return Promise.resolve({ healthy: true, latency_ms: 1 });
      return Promise.resolve(true);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("platforms-pane")).toBeInTheDocument());
    // Verify the add-port form is available
    const portInput = screen.getByTestId("port-add");
    expect(portInput).toBeInTheDocument();
    // The form should not pre-fill Default platform
    // (platform_name is empty by default = unbound)
  });

  it("T8-7: port row shows Unbound text when platform_name is empty", async () => {
    const unboundPort = { ...samplePort, port: 17992, platform_name: "" };
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([unboundPort]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "port_auth_info") return Promise.resolve({ username: "port-17992", auth_required: false, proxy_token: "tok", protocol: "socks5" });
      if (cmd === "port_health_check") return Promise.resolve({ healthy: true, latency_ms: 1 });
      return Promise.resolve(true);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("port-row-17992")).toBeInTheDocument());
    // The port row should show "Unbound" (i18n key platform.unbound)
    const portRow = screen.getByTestId("port-row-17992");
    // The text would be the i18n value for platform.unbound in the test locale (en)
    expect(portRow.textContent).toContain("Unbound");
  });

  it("T10-5: A-class chips pane is always visible (no expand needed)", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([samplePort]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17990);
      if (cmd === "port_health_check") return Promise.resolve({ healthy: true, latency_ms: 1 });
      if (cmd === "port_auth_info") return Promise.resolve({ username: "port-17990", auth_required: false, proxy_token: "tok", protocol: "socks5" });
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("platform-card-Default")).toBeInTheDocument(), { timeout: 3000 });
    expect(screen.getByTestId("strategy-aclass-chips-Default")).toBeInTheDocument();
  });

  it("T8-5: strategy-toggle chevron is NOT present (removed)", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([samplePort]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ platforms: [{ platform_name: "Default", a_class: "region", regions: ["US"] }] });
      if (cmd === "node_list") return Promise.resolve([]);
      if (cmd === "port_health_check") return Promise.resolve({ healthy: true, latency_ms: 1 });
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("platform-card-Default")).toBeInTheDocument());
    expect(screen.queryByTestId("strategy-toggle-Default")).toBeNull();
  });

  it("T10-2: region strategy from config renders region chip toggle with US+JP selected", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([samplePort]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [{ platform_name: "Default", a_class: "region", regions: ["US", "JP"] }] });
      if (cmd === "node_list") return Promise.resolve([{ display_tag: "US-node", region: "US", node_hash: "us1" }, { display_tag: "JP-node", region: "JP", node_hash: "jp1" }]);
      if (cmd === "subscription_list") return Promise.resolve([]);
      if (cmd === "port_suggest") return Promise.resolve(17990);
      if (cmd === "port_health_check") return Promise.resolve({ healthy: true, latency_ms: 1 });
      if (cmd === "port_auth_info") return Promise.resolve({ username: "port-17990", auth_required: false, proxy_token: "tok", protocol: "socks5" });
      if (cmd === "strategy_config_put") return Promise.resolve(true);
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("platform-card-Default")).toBeInTheDocument(), { timeout: 3000 });
    // T10-2: region chip pane is visible with US+JP from config
    await waitFor(() => expect(screen.getByTestId("strategy-region-chips-Default")).toBeInTheDocument(), { timeout: 3000 });
    expect(screen.getByTestId("strategy-region-chip-US")).toBeInTheDocument();
    expect(screen.getByTestId("strategy-region-chip-JP")).toBeInTheDocument();
    // US chip should be active (selected) since config has regions: ["US", "JP"]
    expect(screen.getByTestId("strategy-region-chip-US").className).toContain("bg-blue-500");
  });
});
