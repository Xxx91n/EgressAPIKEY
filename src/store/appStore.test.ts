import { describe, it, expect, beforeEach, vi } from "vitest";
import { useAppStore } from "./appStore";

// mock the settings module so saving through tauri-plugin-store (which is
// unavailable in vitest) is a no-op and we silence the console.warn noise.
vi.mock("../lib/settings", () => ({
  saveView: vi.fn().mockResolvedValue(undefined),
  saveProcessRoutes: vi.fn().mockResolvedValue(undefined),
}));

describe("appStore", () => {
  beforeEach(() => {
    useAppStore.setState({
      platforms: [],
      processRoutes: [],
      subscriptions: [],
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

  it("adds and removes process routes", () => {
    useAppStore.getState().addProcessRoute("ollama", 3);
    expect(useAppStore.getState().processRoutes).toHaveLength(1);
    const id = useAppStore.getState().processRoutes[0].id;
    useAppStore.getState().removeProcessRoute(id);
    expect(useAppStore.getState().processRoutes).toHaveLength(0);
  });

  it("records subscriptions", () => {
    useAppStore.getState().addSubscription("https://example.invalid/sub.yaml", 42);
    expect(useAppStore.getState().subscriptions[0].nodeCount).toBe(42);
  });

  it("switches to the effectiveConfig view (ticket 13)", () => {
    useAppStore.getState().setView("effectiveConfig");
    expect(useAppStore.getState().view).toBe("effectiveConfig");
  });

  it("switches view", () => {
    useAppStore.getState().setView("settings");
    expect(useAppStore.getState().view).toBe("settings");
  });

  // T14-4: shallow selector export + getState pattern (hooks cannot call outside React render)
  it("useShallow is re-exported from appStore", async () => {
    const mod = await import("./appStore");
    expect(typeof mod.useShallow).toBe("function");
  });

  it("individual selector pattern returns correct slices via getState", () => {
    useAppStore.setState({
      platforms: [{ name: "p1", accounts: [], regexFilters: null, regionFilters: null, allocationPolicy: "BALANCED", routableNodeCount: 0, stickyTtl: "" }],
      processRoutes: [],
      subscriptions: [],
      locale: "ja",
    });
    const s = useAppStore.getState();
    expect(s.locale).toBe("ja");
    expect(s.platforms.length).toBe(1);
  });

  it("store updates trigger correct getState re-read", () => {
    useAppStore.setState({ ...useAppStore.getState(), locale: "ko" });
    expect(useAppStore.getState().locale).toBe("ko");
    useAppStore.setState({ ...useAppStore.getState(), locale: "en" });
    expect(useAppStore.getState().locale).toBe("en");
  });
});
