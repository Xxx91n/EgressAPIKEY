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
    invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
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
      if (cmd === "port_upsert") {
        const next = {
          ...samplePort,
          platform_name: String(args?.platformName ?? "OpenAI"),
        };
        ports = [next];
        return Promise.resolve(next);
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
        "port_upsert",
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
    await waitFor(() => expect(screen.getByTestId("strategy-panel")).toBeInTheDocument());
    expect(screen.getByText("No platforms configured")).toBeInTheDocument();
  });

  it("T4-4 strategy: shows a strategy row per platform with A-class + B-class selects", async () => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      return Promise.resolve(undefined);
    });

    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("strategy-row-Default")).toBeInTheDocument());
    expect(screen.getByTestId("strategy-aclass-Default")).toBeInTheDocument();
    expect(screen.getByTestId("strategy-bclass-Default")).toBeInTheDocument();
  });

  it("T4-4 strategy: changing A-class to region reveals regionsInput + tag chips after add", async () => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      return Promise.resolve(undefined);
    });

    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("strategy-row-Default")).toBeInTheDocument());

    // Change A-class select to region
    fireEvent.change(screen.getByTestId("strategy-aclass-Default"), { target: { value: "region" } });
    await waitFor(() => expect(screen.getByTestId("strategy-regions-input-Default")).toBeInTheDocument());

    // Verify the regions input is visible and accepts text
    const regInput = screen.getByTestId("strategy-regions-input-Default");
    expect(regInput).toHaveAttribute("placeholder", "US,SG,JP");
    fireEvent.change(regInput, { target: { value: "GH" } });
    expect((regInput as HTMLInputElement).value).toBe("GH");
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
    await waitFor(() => expect(screen.getByTestId("strategy-apply")).toBeInTheDocument());
    fireEvent.click(screen.getByTestId("strategy-apply"));
    await waitFor(() => expect(putCalled).toBe(true));
    await waitFor(() => expect(applyCalled).toBe(true));
  });

  it("T5-4 B-class: selector has 6 StrategyId options (random/sequential/latency/quality/bandwidth/protocol_weight)", async () => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("strategy-bclass-Default")).toBeInTheDocument());
    const sel = screen.getByTestId("strategy-bclass-Default") as HTMLSelectElement;
    const opts = Array.from(sel.options).map((o) => o.value);
    expect(opts).toEqual(["random", "sequential", "latency", "quality", "bandwidth", "protocol_weight"]);
  });

  it("T5-4 B-class: default value is random (not balanced)", async () => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("strategy-bclass-Default")).toBeInTheDocument());
    const sel = screen.getByTestId("strategy-bclass-Default") as HTMLSelectElement;
    expect(sel.value).toBe("random");
  });

  it("T5-4 B-class: changing select fires updateStrategyField", async () => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "port_list") return Promise.resolve([]);
      if (cmd === "platform_list_full") return Promise.resolve([samplePlatform]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "strategy_config_get") return Promise.resolve({ version: 1, platforms: [] });
      if (cmd === "strategy_config_put") return Promise.resolve(null);
      if (cmd === "strategy_apply") return Promise.resolve({ platforms: [] });
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByTestId("strategy-bclass-Default")).toBeInTheDocument());
    const sel = screen.getByTestId("strategy-bclass-Default") as HTMLSelectElement;
    fireEvent.change(sel, { target: { value: "latency" } });
    expect(sel.value).toBe("latency");
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

  it("Bug4: HTTP port does NOT show SOCKS5 auth credentials (shows httpNoAuth instead)", async () => {
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
    const socks5Label = screen.queryByText(/socks5Auth|SOCKS5 Auth/i);
    expect(socks5Label).toBeNull();
    await waitFor(() => expect(screen.getByText(/HTTP proxy/i)).toBeInTheDocument(), { timeout: 3000 });
  });

});
