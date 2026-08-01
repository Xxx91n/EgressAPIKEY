import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/react";
import { invokeMock } from "../test/setup";
import { PlatformsView } from "./PlatformsView";
import { useAppStore } from "../store/appStore";

afterEach(() => cleanup());

describe("PlatformsView P21-B (dual-pane, IPC-mocked)", () => {
  beforeEach(() => {
    useAppStore.setState({ platforms: [], laneCount: 10 });
    invokeMock.mockReset();
    // Default: list_platforms returns empty, platform_list_full returns empty
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "platform_list_full") return Promise.resolve([]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      return Promise.resolve(undefined);
    });
  });

  it("renders left pane (candidates) + right pane (platforms)", async () => {
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByText(/candidates|候選|候选/i)).toBeInTheDocument());
    // Both panes render headings
    const headings = screen.getAllByText(/Platforms|平台|候选|candidates/i);
    expect(headings.length).toBeGreaterThanOrEqual(2);
  });

  it("adds a key candidate via the form fields", async () => {
    const { container } = render(<PlatformsView />);
    // Wait for mount
    await waitFor(() => {
      const ep = container.querySelector('input[type="text"]');
      expect(ep).toBeTruthy();
    });
    const epInput = container.querySelectorAll('input[type="text"]')[0] as HTMLInputElement;
    const keyInput = container.querySelector('input[type="password"]') as HTMLInputElement;
    fireEvent.change(epInput, { target: { value: "https://api.openai.com/v1" } });
    fireEvent.change(keyInput, { target: { value: "sk-test1234567890abcdef" } });
    const buttons = container.querySelectorAll("button");
    const addBtn = Array.from(buttons).find(b => /add.*key|添加密钥/i.test(b.textContent || ""));
    expect(addBtn).toBeTruthy();
    if (addBtn) fireEvent.click(addBtn);
    await waitFor(() => {
      expect(container.textContent).toContain("https://api.openai.com/v1");
    });
  });

  it("rejects duplicate (endpoint, apiKey) combination", async () => {
    const { container } = render(<PlatformsView />);
    await waitFor(() => {
      const ep = container.querySelector('input[type="text"]');
      expect(ep).toBeTruthy();
    });
    const epInput = container.querySelectorAll('input[type="text"]')[0] as HTMLInputElement;
    const keyInput = container.querySelector('input[type="password"]') as HTMLInputElement;
    fireEvent.change(epInput, { target: { value: "https://api.openai.com/v1" } });
    fireEvent.change(keyInput, { target: { value: "sk-test1234567890abcdef" } });
    const buttons = container.querySelectorAll("button");
    const addBtn = Array.from(buttons).find(b => /add.*key|添加密钥/i.test(b.textContent || ""))!;
    fireEvent.click(addBtn);
    await waitFor(() => expect(container.textContent).toContain("https://api.openai.com/v1"));
    // Second add (same values) should show duplicate error
    fireEvent.change(epInput, { target: { value: "https://api.openai.com/v1" } });
    fireEvent.change(keyInput, { target: { value: "sk-test1234567890abcdef" } });
    fireEvent.click(addBtn);
    await waitFor(() => expect(container.textContent).toMatch(/duplicate|重复/i));
  });

  it("P21-B: UID is derived from (endpoint, apiKey) and does not cross-pollute", async () => {
    const { container } = render(<PlatformsView />);
    await waitFor(() => {
      const ep = container.querySelector('input[type="text"]');
      expect(ep).toBeTruthy();
    });
    const epInput = container.querySelectorAll('input[type="text"]')[0] as HTMLInputElement;
    const keyInput = container.querySelector('input[type="password"]') as HTMLInputElement;
    const buttons = container.querySelectorAll("button");
    const addBtn = Array.from(buttons).find(b => /add.*key|添加密钥/i.test(b.textContent || ""))!;

    // Add candidate 1: ep1 + key1
    fireEvent.change(epInput, { target: { value: "https://api.openai.com/v1" } });
    fireEvent.change(keyInput, { target: { value: "sk-aaa111222333444555" } });
    fireEvent.click(addBtn);
    await waitFor(() => expect(container.textContent).toContain("api.openai.com"));

    // Add candidate 2: ep2 + key2 (different pair → different UID)
    fireEvent.change(epInput, { target: { value: "https://api.anthropic.com/v1" } });
    fireEvent.change(keyInput, { target: { value: "sk-bbb666777888999000" } });
    fireEvent.click(addBtn);
    await waitFor(() => expect(container.textContent).toContain("anthropic"));

    // Both candidates appear simultaneously (no cross-pollution/overwrite)
    expect(container.textContent).toContain("api.openai.com");
    expect(container.textContent).toContain("anthropic");

    // Same (endpoint, key) pair would be rejected as duplicate (tested above)
  });

  it("renders live platforms from ipcPlatformListFull", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "platform_list_full") return Promise.resolve([
        { name: "auto-abc12345", allocation_policy: "BALANCED", regex_filters: [], region_filters: [], routable_node_count: 5, sticky_ttl: "" },
        { name: "my-platform", allocation_policy: "PREFER_LOW_LATENCY", regex_filters: [], region_filters: ["US"], routable_node_count: 10, sticky_ttl: "" },
      ]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [{ account: "acct1", egress_ip: "1.2.3.4" }] });
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    await waitFor(() => expect(screen.getByText("auto-abc12345")).toBeInTheDocument());
    expect(screen.getByText("my-platform")).toBeInTheDocument();
    // Independent badge for auto-
    // Egress policy select shows BALANCED and PREFER_LOW_LATENCY
    expect(screen.getByDisplayValue("BALANCED")).toBeInTheDocument();
    expect(screen.getByDisplayValue("PREFER_LOW_LATENCY")).toBeInTheDocument();
  });
});
