import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/react";
import App from "./App";
import { useAppStore } from "./store/appStore";

afterEach(() => cleanup());

// jsdom does not implement window.matchMedia; App's theme bootstrap calls it.
if (!window.matchMedia) {
  vi.stubGlobal(
    "matchMedia",
    vi.fn().mockImplementation((query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener() { },
      removeListener() { },
      addEventListener() { },
      removeEventListener() { },
      dispatchEvent: () => false,
    })),
  );
}

// webview-smoke contract (R12-B3): the e2e harness navigates via
// button[data-testid="nav-<view>"]. The side rail is icon-only - the label
// lives on aria-label/title, textContent is empty - so nav-<key> is the only
// stable, locale-independent selector. These tests pin that contract.
const NAV_KEYS = [
  "topology",
  "platforms",
  "processRoute",
  "subscriptions",
  "nodes",
  "effectiveConfig",
  "diagnostics",
  "settings",
] as const;

describe("App side rail (webview-smoke nav contract)", () => {
  beforeEach(() => {
    useAppStore.setState({ view: "topology" });
  });

  it("exposes data-testid=nav-<key> on every side-rail button", async () => {
    render(<App />);
    for (const k of NAV_KEYS) {
      await waitFor(
        () => expect(screen.getByTestId("nav-" + k)).toBeInTheDocument(),
        { timeout: 5000 },
      );
    }
  });

  it("nav buttons are icon-only (empty textContent) with an aria-label", async () => {
    render(<App />);
    const btn = await screen.findByTestId("nav-settings", {}, { timeout: 5000 });
    expect(btn.textContent ?? "").toBe("");
    expect(btn.getAttribute("aria-label")).toBeTruthy();
  });

  it("clicking nav-platforms switches the active view", async () => {
    render(<App />);
    const btn = await screen.findByTestId("nav-platforms", {}, { timeout: 5000 });
    fireEvent.click(btn);
    await waitFor(() => expect(useAppStore.getState().view).toBe("platforms"));
    expect(btn.getAttribute("aria-pressed")).toBe("true");
  });
});
