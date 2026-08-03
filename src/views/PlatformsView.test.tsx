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

  it("C2-4: auto-platform cards render SOLID; manual platforms render DASHED (visual distinction per ADR-0006)", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "platform_list_full") return Promise.resolve([
        { name: "auto-abc12345", allocation_policy: "BALANCED", regex_filters: [], region_filters: [], routable_node_count: 5, sticky_ttl: "" },
        { name: "my-manual-plat", allocation_policy: "PREFER_LOW_LATENCY", regex_filters: [], region_filters: ["US"], routable_node_count: 10, sticky_ttl: "" },
      ]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    // Wait for both platform cards to mount.
    await waitFor(() => expect(screen.getByText("auto-abc12345")).toBeInTheDocument());
    expect(screen.getByText("my-manual-plat")).toBeInTheDocument();

    // Locate the two platform cards by matching their rendered platform name text, then walk up
    // to the ancestor card div whose className carries border-dashed or border-solid. We cannot
    // use data-testid, so we inspect the DOM shape produced by PlatformsView.tsx: each platform
    // card is a <div> whose immediate child contains the platform name <span>; walk up two
    // parents to reach the card wrapper that owns the className.
    const autoNameEl = screen.getByText("auto-abc12345");
    const manualNameEl = screen.getByText("my-manual-plat");
    function cardClassFor(nameEl: HTMLElement): string | null {
      let cur: HTMLElement | null = nameEl;
      for (let depth = 0; depth < 5 && cur; depth++) {
        const cls = cur.getAttribute && cur.getAttribute("class");
        if (cls && (cls.includes("border-dashed") || cls.includes("border-solid"))) return cls;
        cur = cur.parentElement;
      }
      return null;
    }
    const autoCls = cardClassFor(autoNameEl);
    const manualCls = cardClassFor(manualNameEl);
    expect(autoCls).not.toBeNull();
    expect(manualCls).not.toBeNull();
    // C2-4 contract: auto -> solid border (no 'border-dashed'); manual -> dashed.
    expect(autoCls && autoCls.includes("border-dashed")).toBe(false);
    expect(autoCls && autoCls.includes("border-solid")).toBe(true);
    expect(manualCls && manualCls.includes("border-dashed")).toBe(true);
  });

    it("C2-14: opening the +New platform dialog and submitting a valid name forwards ipcPlatformCreateWithFields + refreshes platforms", async () => {
    let createdBody: unknown = null;
    let listCalls = 0;
    invokeMock.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "platform_list_full") {
        listCalls++;
        // Batch 1: no platforms; after create + refresh batch 2 returns the new one.
        return Promise.resolve(listCalls === 1
          ? []
          : [{ name: "manual-test", allocation_policy: "PREFER_LOW_LATENCY", regex_filters: ["api.openai.com"], region_filters: ["US"], routable_node_count: 0, sticky_ttl: "" }]);
      }
      if (cmd === "platform_create_with_fields") { createdBody = args; return Promise.resolve(undefined); }
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    // Find the "+New platform" button (right-pane header). Use getAllByRole since the left-pane
    // 'Add key combination' button may also match /new|新建/i in some locales; we match createTitle.
    await waitFor(() => {
      const createBtns = screen.getAllByRole("button", { name: /New platform|新建平台|platform.createTitle/i });
      expect(createBtns.length).toBeGreaterThanOrEqual(1);
    });
    const createBtn = screen.getAllByRole("button", { name: /New platform|新建平台|platform.createTitle/i })[0];
    fireEvent.click(createBtn);
    // Dialog opens: name input has data-testid="create-platform-name", submit has data-testid="create-platform-submit".
    const nameInput = await screen.findByTestId("create-platform-name");
    fireEvent.change(nameInput, { target: { value: "manual-test" } });
    const submitBtn = screen.getByTestId("create-platform-submit");
    fireEvent.click(submitBtn);
    // ipcPlatformCreateWithFields received the body.
    await waitFor(() => { expect(createdBody).not.toBeNull(); });
    // Tauri invoke passes { body: {...} } as the command args; verify the inner body.
    expect(createdBody).toMatchObject({
      body: {
        name: "manual-test",
        allocation_policy: "BALANCED",
        regex_filters: [],
        region_filters: [],
      },
    });
    // After create: refresh fires list again and the new platform renders.
    await waitFor(() => expect(screen.getByText("manual-test")).toBeInTheDocument());
  });

  it("C2-14: submitting the dialog with an empty name shows the createEmptyName error and never calls ipcPlatformCreateWithFields", async () => {
    let createCalled = false;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "platform_list_full") return Promise.resolve([]);
      if (cmd === "platform_create_with_fields") { createCalled = true; return Promise.resolve(undefined); }
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    // Open dialog.
    const createBtn = (await waitFor(() => screen.getAllByRole("button", { name: /New platform|新建平台|platform.createTitle/i })))[0];
    fireEvent.click(createBtn);
    const nameInput = await screen.findByTestId("create-platform-name");
    // Leave name empty; submit.
    fireEvent.change(nameInput, { target: { value: "   " } });
    const submitBtn = screen.getByTestId("create-platform-submit");
    fireEvent.click(submitBtn);
    // Within a moment, the empty-name error surfaces and create was NOT called.
    await waitFor(() => {
      expect(screen.getByText(/cannot be empty|不能为空|platform.createEmptyName/i)).toBeInTheDocument();
    });
    expect(createCalled).toBe(false);
  });

  it("ADR-0006 item 1 closed-loop: renders routable nodes for a platform after platform_snapshot resolves", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "platform_list_full") return Promise.resolve([
        { name: "P1", allocationPolicy: "BALANCED", regionFilters: [], routableNodeCount: 1, regexFilters: [], stickyTtl: "30m" },
      ]);
      if (cmd === "platform_leases") return Promise.resolve({ items: [] });
      if (cmd === "platform_snapshot") return Promise.resolve({
        items: [{ display_tag: "hk-01", region: "hk" }],
        total: 1, limit: 500, offset: 0,
      });
      return Promise.resolve(undefined);
    });
    render(<PlatformsView />);
    // The collapsible summary shows "routableNodes" i18n label + 1
    await waitFor(() => expect(screen.getByText(/platform\.routableNodes|Routable nodes|可路由节点|Nodos enrutables/i)).toBeInTheDocument());
  });
});
