import { describe, it, expect, beforeEach, afterEach } from "vitest";
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
});
