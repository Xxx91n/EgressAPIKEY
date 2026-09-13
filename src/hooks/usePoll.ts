/**
 * Unified polling hook with visibility-based pause/resume.
 *
 * Replaces per-View setInterval + visibilitychange boilerplate.
 * Based on the Yerd usePoll pattern: AbortController for cancel,
 * document.visibilityState for pause, single cleanup.
 *
 * Ponytail: zero deps, 40 lines, uses React 19 builtins.
 */

import { useEffect, useRef, useCallback } from "react";

/**
 * Polling options.
 */
export interface UsePollOptions {
  /** Polling interval in milliseconds. */
  intervalMs: number;
  /** Whether to pause polling when the tab is hidden (default: true). */
  pauseWhenHidden?: boolean;
  /** Whether to run the callback immediately on mount (default: false). */
  fireImmediately?: boolean;
}

/**
 * Start polling `fn` every `intervalMs`. Automatically pauses when the
 * document is hidden (configurable) and resumes when visible. The `fn`
 * receives an AbortSignal so it can cancel in-flight fetches on unmount/pause.
 *
 * Usage:
 *   usePoll(async (signal) => {
 *     const res = await fetch("/api/data", { signal });
 *     ...
 *   }, { intervalMs: 5000 });
 */
export function usePoll(
  fn: (signal: AbortSignal) => void | Promise<void>,
  options: UsePollOptions,
): void {
  const fnRef = useRef(fn);
  fnRef.current = fn; // always use the latest fn without re-subscribing

  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  const runOnce = useCallback(() => {
    // Cancel any previous in-flight poll
    abortRef.current?.abort();
    const ctrl = new AbortController();
    abortRef.current = ctrl;
    // Call synchronously; the caller's fn can be async and handle its own errors.
    // The signal is passed so async fetches can be cancelled on pause/unmount.
    try {
      fnRef.current(ctrl.signal);
    } catch {
      // caller handles errors inside fn
    }
  }, []);

  const start = useCallback(() => {
    if (intervalRef.current) return;
    intervalRef.current = setInterval(runOnce, options.intervalMs);
  }, [options.intervalMs, runOnce]);

  const stop = useCallback(() => {
    if (intervalRef.current) {
      clearInterval(intervalRef.current);
      intervalRef.current = null;
    }
    abortRef.current?.abort();
  }, []);

  useEffect(() => {
    const pauseWhenHidden = options.pauseWhenHidden !== false;

    if (options.fireImmediately) runOnce();
    start();

    if (pauseWhenHidden) {
      const onVisibilityChange = () => {
        if (document.visibilityState === "hidden") {
          stop();
        } else if (document.visibilityState === "visible") {
          runOnce(); // immediate refresh on resume
          start();
        }
      };
      document.addEventListener("visibilitychange", onVisibilityChange);
      return () => {
        document.removeEventListener("visibilitychange", onVisibilityChange);
        stop();
      };
    }

    return () => stop();
  }, [options.intervalMs, options.pauseWhenHidden, options.fireImmediately, runOnce, start, stop]);
}
