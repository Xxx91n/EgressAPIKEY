import { describe, it, expect, beforeEach } from "vitest";
import { useAppStore } from "./appStore";

describe("appStore", () => {
  beforeEach(() => {
    useAppStore.setState({
      platforms: [],
      processRoutes: [],
      subscriptions: [],
      laneCount: 10,
      locale: "en",
    });
  });

  it("adds and removes platforms without duplicates", () => {
    useAppStore.getState().addPlatform("openai");
    useAppStore.getState().addPlatform("openai");
    expect(useAppStore.getState().platforms).toHaveLength(1);
    useAppStore.getState().removePlatform("openai");
    expect(useAppStore.getState().platforms).toHaveLength(0);
  });

  it("adds accounts and binds the exit ip", () => {
    useAppStore.getState().addPlatform("anthropic");
    useAppStore.getState().addAccount("anthropic", "acct-1", 2);
    useAppStore.getState().bindExitIp("anthropic", "acct-1", "1.2.3.4");
    const p = useAppStore.getState().platforms[0];
    expect(p.accounts[0].exitIp).toBe("1.2.3.4");
    expect(p.accounts[0].lane).toBe(2);
  });

  it("clamps lane count to 1..50", () => {
    useAppStore.getState().setLaneCount(9999);
    expect(useAppStore.getState().laneCount).toBe(50);
    useAppStore.getState().setLaneCount(0);
    expect(useAppStore.getState().laneCount).toBe(1);
  });

  it("adds and removes process routes", () => {
    useAppStore.getState().addProcessRoute("ollama", 3);
    expect(useAppStore.getState().processRoutes).toHaveLength(1);
    const id = useAppStore.getState().processRoutes[0].id;
    useAppStore.getState().removeProcessRoute(id);
    expect(useAppStore.getState().processRoutes).toHaveLength(0);
  });

  it("records subscriptions with lane count", () => {
    useAppStore.getState().addSubscription("https://example.invalid/sub.yaml", 42, 10);
    expect(useAppStore.getState().subscriptions[0].nodeCount).toBe(42);
  });

  it("switches view", () => {
    useAppStore.getState().setView("settings");
    expect(useAppStore.getState().view).toBe("settings");
  });
});
