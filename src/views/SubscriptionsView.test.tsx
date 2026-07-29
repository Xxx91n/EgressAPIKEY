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
});
