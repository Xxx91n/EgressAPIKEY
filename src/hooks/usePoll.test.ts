/**
 * usePoll test suite
 * Verifies: starts polling, pauses when hidden, resumes when visible,
 * cleans up on unmount, passes AbortSignal to fn.
 */

import { describe, it, expect, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { usePoll } from "./usePoll";

describe("usePoll", () => {
  it("calls fn immediately when fireImmediately=true", () => {
    const fn = vi.fn((_signal: AbortSignal) => {});
    renderHook(() => usePoll(fn, { intervalMs: 1000, fireImmediately: true }));
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("does NOT call fn when fireImmediately=false (only on interval)", async () => {
    const fn = vi.fn();
    renderHook(() => usePoll(fn, { intervalMs: 1000, fireImmediately: false }));
    expect(fn).toHaveBeenCalledTimes(0);
  });

  it("passes AbortSignal to fn", () => {
    const fn = vi.fn((_signal: AbortSignal) => {});
    renderHook(() => usePoll(fn, { intervalMs: 1000, fireImmediately: true }));
    expect(fn).toHaveBeenCalledTimes(1);
    const signal = fn.mock.calls[0][0];
    expect(signal).toBeInstanceOf(AbortSignal);
    expect(signal.aborted).toBe(false);
  });

  it("stops polling on unmount", async () => {
    const fn = vi.fn();
    const { unmount } = renderHook(() => usePoll(fn, { intervalMs: 100, fireImmediately: false }));
    unmount();
    await new Promise((r) => setTimeout(r, 200));
    expect(fn).not.toHaveBeenCalled();
  });

  it("pauses polling when document becomes hidden", async () => {
    const fn = vi.fn();
    renderHook(() => usePoll(fn, { intervalMs: 50, pauseWhenHidden: true, fireImmediately: false }));

    // Simulate hidden
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
    document.dispatchEvent(new Event("visibilitychange"));

    await new Promise((r) => setTimeout(r, 200));
    expect(fn).not.toHaveBeenCalled();
  });

  it("resumes polling when document becomes visible again", async () => {
    const fn = vi.fn();
    renderHook(() => usePoll(fn, { intervalMs: 50, pauseWhenHidden: true, fireImmediately: false }));

    // Hidden then visible
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
    document.dispatchEvent(new Event("visibilitychange"));
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
    document.dispatchEvent(new Event("visibilitychange"));

    // Should fire immediately on resume (at least once)
    await new Promise((r) => setTimeout(r, 30));
    expect(fn.mock.calls.length).toBeGreaterThanOrEqual(1);
  });
});
