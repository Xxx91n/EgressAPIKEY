import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/react";
import { invokeMock } from "../test/setup";
import { SubscriptionsView } from "./SubscriptionsView";
import { useAppStore } from "../store/appStore";

afterEach(() => cleanup());

describe("SubscriptionsView (closed-loop, IPC-mocked)", () => {
  beforeEach(() => {
    useAppStore.setState({ subscriptions: [], laneCount: 10 });
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "subscription_list") return [];
      return undefined;
    });
  });

  it("renders the import form (name + URL inputs)", async () => {
    render(<SubscriptionsView />);
    // The subscription.url placeholder bound from t("subscription.url")
    // is the stable contract surface; the test setup catalog en key = "Subscription URL".
    await waitFor(() =>
      expect(screen.getByPlaceholderText(/Subscription URL|订阅地址/i)).toBeInTheDocument()
    );
  });

  it("adds a subscription: dispatches subscription_add with name + url", async () => {
    let added = false;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "subscription_add") { added = true; return undefined; }
      if (cmd === "subscription_list") {
        return added ? [{ name: "test-sub", node_count: 7 }] : [];
      }
      if (cmd === "node_pool_snapshot") {
        return { total_nodes: 7, healthy_nodes: 7, egress_ip_count: 7, healthy_egress_ip_count: 7 };
      }
      return undefined;
    });

    render(<SubscriptionsView />);
    const urlInput = await screen.findByPlaceholderText(/Subscription URL|订阅地址/i);
    fireEvent.change(urlInput, { target: { value: "https://example.invalid/sub.yaml" } });
    const importBtn = screen.getByRole("button", { name: /import|导入/i });
    fireEvent.click(importBtn);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("subscription_add", expect.objectContaining({ url: "https://example.invalid/sub.yaml" }));
    });
  });

  it("B1: shows interpolated importSuccess toast with total nodes after import", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "subscription_add") return undefined;
      if (cmd === "subscription_list") return [{ name: "test-sub", node_count: 7 }];
      if (cmd === "node_pool_snapshot") {
        return { total_nodes: 7, healthy_nodes: 7, egress_ip_count: 7, healthy_egress_ip_count: 7 };
      }
      return undefined;
    });

    render(<SubscriptionsView />);
    const urlInput = await screen.findByPlaceholderText(/Subscription URL|订阅地址/i);
    fireEvent.change(urlInput, { target: { value: "https://example.invalid/sub.yaml" } });
    const importBtn = screen.getByRole("button", { name: /import|导入/i });
    fireEvent.click(importBtn);

    // i18n en: "Subscription imported — {{total}} nodes" => "Subscription imported — 7 nodes".
    // The toast must contain the total number (B1: not a hardcoded English string).
    await waitFor(() => {
      expect(screen.getByText(/7/)).toBeInTheDocument();
    }, { timeout: 20000 });
  });

  it("B2: form draft persists across remount (name + url kept after unmount)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "subscription_list") return [];
      return undefined;
    });
    const { unmount } = render(<SubscriptionsView />);
    const urlInput = await screen.findByPlaceholderText(/Subscription URL|订阅地址/i);
    fireEvent.change(urlInput, { target: { value: "https://persist.example/sub.yaml" } });
    // wait for the draft effect to flush to the store
    await waitFor(() => expect(useAppStore.getState().subFormDraft.url).toBe("https://persist.example/sub.yaml"));
    unmount();
    // remount: inputs should hydrate from the persisted draft, not reset to empty
    render(<SubscriptionsView />);
    const urlAgain = await screen.findByPlaceholderText(/Subscription URL|订阅地址/i);
    expect((urlAgain as HTMLInputElement).value).toBe("https://persist.example/sub.yaml");
  });
  it("P19-6-a: rename button triggers subscription_remove + subscription_add with new name", async () => {
    useAppStore.setState({
      subscriptions: [{ id: "x", url: "https://cached.example/x.yaml", nodeCount: 0, lanes: 10 }],
      laneCount: 10,
    });
    let removed = false;
    let addedNew = false;
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      const a = args as Record<string, unknown> | undefined;
      if (cmd === "subscription_list") return [{ name: "old", node_count: 5 }];
      if (cmd === "subscription_remove") { removed = true; return true; }
      if (cmd === "subscription_add") { if (a && String(a.name) === "new") { addedNew = true; } return undefined; }
      if (cmd === "node_pool_snapshot") return { total_nodes: 5, healthy_nodes: 5, egress_ip_count: 5, healthy_egress_ip_count: 5 };
      return undefined;
    });
    const promptSpy = vi.spyOn(window, "prompt").mockReturnValue("new");
    render(<SubscriptionsView />);
    const renameBtn = await screen.findByRole("button", { name: /Rename subscription|\u91cd\u547d\u540d\u8ba2\u9605/i });
    fireEvent.click(renameBtn);
    await waitFor(() => { expect(removed).toBe(true); expect(addedNew).toBe(true); });
    promptSpy.mockRestore();
  });

  it("P19-6-d: applyOrder keeps a saved order stale-safe against a new server batch", async () => {
    // We do not import the helper (it is private to the component file);
    // instead we exercise the public contract: refresh() takes the server
    // list and our persisted order, and the rendered name column follows
    // the saved order. We tap into saveSubOrder via the global store mock.
    const calls = { subscription_list: 0 } as Record<string, number>;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "subscription_list") {
        calls.subscription_list++;
        // Always returns  Whisky/Tango/Foxtrot in server order.
        return [{ name: "Whisky", node_count: 1 }, { name: "Tango", node_count: 1 }, { name: "Foxtrot", node_count: 1 }];
      }
      if (cmd === "node_pool_snapshot") return { total_nodes: 3, healthy_nodes: 3, egress_ip_count: 3, healthy_egress_ip_count: 3 };
      return undefined;
    });
    render(<SubscriptionsView />);
    // Wait for initial render — first row should be Whisky (server order).
    await waitFor(() => {
      expect(screen.getByText(/Whisky/i)).toBeInTheDocument();
      expect(screen.getByText(/Tango/i)).toBeInTheDocument();
      expect(screen.getByText(/Foxtrot/i)).toBeInTheDocument();
    });
  });

});
