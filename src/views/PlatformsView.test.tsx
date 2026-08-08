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
});
