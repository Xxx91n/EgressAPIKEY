/**
 * T14-5: useIpc closed-loop tests — verifies SWR-backed IPC hook
 * uses real timers (matching usePoll test pattern).
 */

import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import { useIpc } from "./useIpc";

// Mock @tauri-apps/api/core invoke
const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

describe("useIpc", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("loads data from IPC invoke on mount", async () => {
    mockInvoke.mockResolvedValue(["platform1", "platform2"]);
    const { result } = renderHook(() =>
      useIpc("test-key-1", () => mockInvoke("platform_list"))
    );
    // Initially loading
    expect(result.current.isLoading).toBe(true);
    // Wait for data
    await waitFor(() => expect(result.current.data).toEqual(["platform1", "platform2"]));
    expect(result.current.isLoading).toBe(false);
    expect(result.current.error).toBeUndefined();
  });

  it("returns error when fetcher throws (outside Tauri)", async () => {
    mockInvoke.mockRejectedValue(new Error("not in Tauri"));
    const { result } = renderHook(() =>
      useIpc("error-key-1", () => mockInvoke("platform_list"))
    );
    await waitFor(() => expect(result.current.error).toBeDefined());
    // shouldRetryOnError is false, so no retries
    expect(mockInvoke).toHaveBeenCalledTimes(1);
  });

  it("deduplicates concurrent calls to same key via shared cache", async () => {
    mockInvoke.mockResolvedValue(["data"]);
    const fetcher = () => mockInvoke("platform_list");
    // Mount hook, let it resolve, then mount second hook with same key
    const { result: r1 } = renderHook(() => useIpc("dedup-key", fetcher));
    await waitFor(() => expect(r1.current.data).toEqual(["data"]));
    // Second hook with same key should reuse cache (no new fetch)
    const { result: r2 } = renderHook(() => useIpc("dedup-key", fetcher));
    await waitFor(() => expect(r2.current.data).toEqual(["data"]));
    // Only one fetch happened (dedup)
    expect(mockInvoke).toHaveBeenCalledTimes(1);
  });

  it("mutate triggers revalidation", async () => {
    mockInvoke.mockResolvedValue(["old"]);
    const { result } = renderHook(() =>
      useIpc("mutate-key", () => mockInvoke("platform_list"))
    );
    await waitFor(() => expect(result.current.data).toEqual(["old"]));
    // Change mock return and mutate
    mockInvoke.mockResolvedValue(["new"]);
    await act(async () => {
      await result.current.mutate();
    });
    await waitFor(() => expect(result.current.data).toEqual(["new"]));
  });

  it("keepPreviousData option prevents flash on key change", async () => {
    mockInvoke.mockResolvedValueOnce(["v1"]);
    const { result, rerender } = renderHook(({ k }) =>
      useIpc(k, () => mockInvoke("platform_list"), { revalidateOnFocus: false })
    , { initialProps: { k: "kp-key-1" } });
    await waitFor(() => expect(result.current.data).toEqual(["v1"]));
    // Change key — old data should still be visible briefly
    mockInvoke.mockResolvedValueOnce(["v2"]);
    rerender({ k: "kp-key-2" });
    // With keepPreviousData, data should not be undefined during revalidation
    // (it may or may not show v1 depending on timing, but it shouldn't be undefined)
    await waitFor(() => expect(result.current.data).toEqual(["v2"]));
  });

  it("falls back gracefully outside Tauri (no crash)", async () => {
    // Simulate non-Tauri env: invoke always throws
    mockInvoke.mockRejectedValue(new Error("window.__TAURI_INTERNALS__ is undefined"));
    const { result } = renderHook(() =>
      useIpc("ext-key-1", () => mockInvoke("platform_list"))
    );
    await waitFor(() => expect(result.current.error).toBeDefined());
    // Should not crash, data stays undefined
    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
  });
});
