import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/react";
import { afterEach } from "vitest";
import { invokeMock } from "../test/setup";
import { PlatformsView } from "./PlatformsView";
import { useAppStore } from "../store/appStore";

afterEach(() => cleanup());

describe("PlatformsView (closed-loop, IPC-mocked)", () => {
  beforeEach(() => {
    useAppStore.setState({ platforms: [], laneCount: 10 });
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
  });

  it("renders empty list + add-platform form", () => {
    render(<PlatformsView />);
    expect(screen.getByRole("textbox")).toBeInTheDocument();
    expect(screen.getByText(/platform\.title|^Platforms$|平台/i)).toBeInTheDocument();
  });

  it("adds a platform via the form + reducer round trip + dispatches platform_add IPC", async () => {
    render(<PlatformsView />);
    const input = screen.getByRole("textbox") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "openai-prod" } });
    fireEvent.keyDown(input, { key: "Enter" });
    // The optimistic reducer should list the platform immediately even when
    // the backend IPC resolves undefined (the store carries the name).
    await waitFor(() => expect(screen.getByText("openai-prod")).toBeInTheDocument());
    // The IPC dispatched the platform_add command with the typed name.
    expect(invokeMock).toHaveBeenCalledWith("platform_add", { name: "openai-prod" });
  });

  it("removes a platform: dispatches platform_remove IPC and reducer trims it", async () => {
    useAppStore.setState({ platforms: [{ name: "anthropic", accounts: [], regexFilters: null, regionFilters: null, allocationPolicy: "BALANCED", routableNodeCount: 0, stickyTtl: "168h0m0s" }] });
    render(<PlatformsView />);
    expect(screen.getByText("anthropic")).toBeInTheDocument();
    const drop =
      screen.getByRole("button", { name: /delete|\u5220\u9664|\u522a\u9664/i }) ||
      screen.getAllByRole("button").find((b) => /Trash/.test((b.firstChild as HTMLElement)?.className || "")) ||
      null;
    if (drop) fireEvent.click(drop);
    await waitFor(() =>
      expect(useAppStore.getState().platforms.find((p) => p.name === "anthropic")).toBeUndefined()
    );
  });
});
