import { describe, it, expect, vi, beforeEach } from "vitest";

/// Issue 1: closed-loop coverage for the IPC boundary. We mock
/// @tauri-apps/api/core's invoke and verify:
///   - IPC wrappers validate inputs (lane range, name length, URL shape)
///   - Forwards the args to the right command name
///   - Surfaces errors verbatim from the backend
/// Vitest runs WITHOUT a real Tauri runtime, so invoke would reject anyway;
/// we provide a mockable surface so we can assert the dispatched command and
/// validate the TS-side guards beyond what the unit-test store covers.

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

// import AFTER the mock is registered.
import {
  ipcPlatformAdd, ipcPlatformRemove, ipcPlatformList,
  ipcSubscriptionAdd, ipcSubscriptionRemove, ipcSubscriptionList,
  ipcProcessRouteAdd, ipcProcessRouteRemove, ipcProcessRouteList,
  ipcAccountAdd,
  ipcPlatformUpdate, ipcNodeList,
} from "./ipc";

describe("IPC wrappers (issue 1 closed-loops)", () => {
  beforeEach(() => { invokeMock.mockReset(); });

  it("platform_add forwards name and resolves on backend ok", async () => {
    invokeMock.mockResolvedValue(undefined);
    await ipcPlatformAdd("openai");
    expect(invokeMock).toHaveBeenCalledWith("platform_add", { name: "openai" });
  });

  it("platform_add rejects names with control chars (TS guard)", async () => {
    await expect(ipcPlatformAdd("bad\x00name")).rejects.toThrow(/platform invalid/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("platform_remove forwards name and returns backend bool", async () => {
    invokeMock.mockResolvedValue(true);
    const r = await ipcPlatformRemove("openai");
    expect(r).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("platform_remove", { name: "openai" });
  });

  it("platform_list returns the backend string array", async () => {
    invokeMock.mockResolvedValue(["a", "b"]);
    const r = await ipcPlatformList();
    expect(r).toEqual(["a", "b"]);
  });

  it("account_add rejects lanes outside 0..49", async () => {
    await expect(ipcAccountAdd("p", "a", 50)).rejects.toThrow(/out of range/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("subscription_add validates URL shape (http prefix)", async () => {
    await expect(ipcSubscriptionAdd("n", "ftp://x")).rejects.toThrow(/http:/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("subscription_add forwards name + url to the backend", async () => {
    invokeMock.mockResolvedValue(undefined);
    await ipcSubscriptionAdd("n", "https://x.invalid/sub");
    expect(invokeMock).toHaveBeenCalledWith("subscription_add", { name: "n", url: "https://x.invalid/sub" });
  });

  it("subscription_remove forwards the name", async () => {
    invokeMock.mockResolvedValue(true);
    await ipcSubscriptionRemove("n");
    expect(invokeMock).toHaveBeenCalledWith("subscription_remove", { name: "n" });
  });

  it("subscription_list returns the expected typed shape", async () => {
    invokeMock.mockResolvedValue([{ name: "n", node_count: 7 }]);
    const r = await ipcSubscriptionList();
    expect(r).toEqual([{ name: "n", node_count: 7 }]);
  });

  it("process_route_add rejects lane 50 (out of range)", async () => {
    await expect(ipcProcessRouteAdd("ollama", 50)).rejects.toThrow(/out of range/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("process_route_add forwards process + targetLane", async () => {
    invokeMock.mockResolvedValue(undefined);
    await ipcProcessRouteAdd("ollama", 3);
    expect(invokeMock).toHaveBeenCalledWith("process_route_add", { process: "ollama", targetLane: 3 });
  });

  it("process_route_remove rejects empty process name", async () => {
    await expect(ipcProcessRouteRemove("")).rejects.toThrow(/process invalid/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("process_route_list forwards no args and returns typed array", async () => {
    invokeMock.mockResolvedValue([{ process: "ollama", target_lane: 3 }]);
    const r = await ipcProcessRouteList();
    expect(r).toEqual([{ process: "ollama", target_lane: 3 }]);
  });

  // Phase R1: platform_update + node_list closed-loop guards.
  it("platform_update rejects invalid allocation_policy before invoke", async () => {
    await expect(ipcPlatformUpdate("OpenAI", "RANDOM" as any)).rejects.toThrow(/allocation_policy must be one of/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("platform_update rejects too many regex_filters before invoke", async () => {
    const tooMany = Array(65).fill("api.openai.com");
    await expect(ipcPlatformUpdate("OpenAI", undefined, tooMany)).rejects.toThrow(/too many/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("platform_update forwards valid fields with null for omitted ones", async () => {
    invokeMock.mockResolvedValue({ ok: true });
    await ipcPlatformUpdate("OpenAI", "PREFER_LOW_LATENCY");
    expect(invokeMock).toHaveBeenCalledWith("platform_update", {
      name: "OpenAI",
      allocationPolicy: "PREFER_LOW_LATENCY",
      regexFilters: null,
      stickyTtl: null,
    });
  });

  it("node_list forwards no args", async () => {
    invokeMock.mockResolvedValue({ items: [] });
    const r = await ipcNodeList();
    expect(r).toEqual({ items: [] });
    expect(invokeMock).toHaveBeenCalledWith("node_list");
  });
});
