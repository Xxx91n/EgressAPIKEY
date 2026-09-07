/**
 * Architecture-recovery ticket 07 (spec D-C2.2) — global converge loop tests.
 *
 * Covers the checkpoints at the store level:
 *   A. polling cadence: default 5s; Converged + settled (>60s) backoff to 30s;
 *   B. sidecar-status event boost: an event refetches immediately;
 *   C. the tray mirror's own state machine lives in src-tauri/src/tray.rs
 *      Rust unit tests; this suite locks the frontend half of the contract —
 *      the store drives everything through the EXISTING snapshot IPC
 *      (authoritative_snapshot), no new tray command is invoked;
 *   D. visibility pause: hidden clears the cadence, resume refetches.
 *
 * The Tauri core invoke is mocked in src/test/setup.ts (invokeMock); the
 * event module is mocked locally so tests can fire sidecar-status handlers;
 * fake timers (pinned to a real 2026 epoch so lastApplyAt survives the
 * ipc.ts timestamp sanitizer) drive the poll loop deterministically.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { invokeMock } from "../test/setup";
import {
  useAppStore,
  convergePollIntervalMs,
  CONVERGE_POLL_INTERVAL_MS,
  CONVERGE_POLL_CONVERGED_MS,
  CONVERGE_SETTLE_SECONDS,
  SIDECAR_STATUS_EVENT,
} from "./appStore";
import type { AuthoritativeSnapshot } from "../lib/ipc";

// Capture the registered sidecar-status listeners so tests can fire them.
type Listener = (evt?: { payload: unknown }) => void;
const registered: Listener[] = [];
const listenMock = vi.fn(async (_event: string, handler: Listener) => {
  registered.push(handler);
  return () => {
    const i = registered.indexOf(handler);
    if (i >= 0) registered.splice(i, 1);
  };
});

vi.mock("@tauri-apps/api/event", () => ({
  listen: (...a: unknown[]) => listenMock(a[0] as string, a[1] as Listener),
}));

// 2026-01-01T00:00:00Z in Unix seconds — positive and under the ipc.ts
// SNAPSHOT_TS_MAX ceiling, so the sanitizer keeps lastApplyAt intact.
const NOW_SEC = 1_767_225_600;

let nextId = 0;
function snap(overrides: Partial<AuthoritativeSnapshot> = {}): AuthoritativeSnapshot {
  nextId++;
  return {
    strategyVersion: 1,
    platforms: [],
    ports: [],
    routes: [],
    subscriptions: [],
    resinReachable: true,
    lastCheckedAt: NOW_SEC,
    strategyGeneration: 3,
    strategyAppliedGeneration: 3,
    convergePhase: "Converged",
    lastApplyAt: NOW_SEC - 30,
    lastApplyError: undefined,
    ...overrides,
  };
}

function snapshotCalls(): number {
  return invokeMock.mock.calls.filter((c) => c[0] === "authoritative_snapshot").length;
}

function fireSidecarStatus(payload: string): void {
  for (const h of [...registered]) h({ payload });
}

let activeUnsub: (() => void) | null = null;
function subscribe(): () => void {
  activeUnsub = useAppStore.getState().subscribeToConverge();
  return activeUnsub;
}

beforeEach(() => {
  vi.useFakeTimers({ now: new Date(NOW_SEC * 1000) });
  registered.length = 0;
  listenMock.mockClear();
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "authoritative_snapshot") return Promise.resolve(snap());
    return Promise.resolve(undefined);
  });
  useAppStore.setState({ convergeSnapshot: null, convergePausedByVisibility: false, convergeSubscribed: false });
});

afterEach(() => {
  if (activeUnsub !== null) {
    activeUnsub();
    activeUnsub = null;
  }
  vi.useRealTimers();
});

describe("converge loop — pure cadence selector (checkpoint A)", () => {
  it("defaults to the 5s interval for a null snapshot", () => {
    expect(convergePollIntervalMs(null, NOW_SEC)).toBe(CONVERGE_POLL_INTERVAL_MS);
    expect(CONVERGE_POLL_INTERVAL_MS).toBe(5000);
    expect(CONVERGE_POLL_CONVERGED_MS).toBe(30_000);
    expect(CONVERGE_SETTLE_SECONDS).toBe(60);
  });

  it("backs off to 30s only when Converged AND lastApplyAt is older than 60s", () => {
    const base = { convergePhase: "Converged" as const };
    // Settled (apply 61s ago) → slow cadence.
    expect(convergePollIntervalMs(snap({ ...base, lastApplyAt: NOW_SEC - 61 }), NOW_SEC)).toBe(CONVERGE_POLL_CONVERGED_MS);
    // Fresh green apply (within the settle window) → still fast.
    expect(convergePollIntervalMs(snap({ ...base, lastApplyAt: NOW_SEC - 30 }), NOW_SEC)).toBe(CONVERGE_POLL_INTERVAL_MS);
    // Exactly at the boundary (60s) → not "> 60s" → fast.
    expect(convergePollIntervalMs(snap({ ...base, lastApplyAt: NOW_SEC - 60 }), NOW_SEC)).toBe(CONVERGE_POLL_INTERVAL_MS);
    // Converged but never applied (undefined lastApplyAt) → fast.
    expect(convergePollIntervalMs(snap({ ...base, lastApplyAt: undefined }), NOW_SEC)).toBe(CONVERGE_POLL_INTERVAL_MS);
    // Non-Converged phases never earn the backoff.
    for (const phase of ["Drifted", "ApplyFailed", "PendingApply", "NeverApplied", "Unknown"] as const) {
      expect(
        convergePollIntervalMs(snap({ convergePhase: phase, lastApplyAt: NOW_SEC - 3600 }), NOW_SEC)
      ).toBe(CONVERGE_POLL_INTERVAL_MS);
    }
  });

  it("live loop: a settled Converged snapshot schedules the next tick at 30s, not 5s", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "authoritative_snapshot"
        ? Promise.resolve(snap({ convergePhase: "Converged", lastApplyAt: NOW_SEC - 3600 }))
        : Promise.resolve(undefined)
    );
    subscribe();
    await vi.advanceTimersByTimeAsync(0);
    expect(useAppStore.getState().convergeSnapshot?.convergePhase).toBe("Converged");
    const afterFirst = snapshotCalls();
    await vi.advanceTimersByTimeAsync(CONVERGE_POLL_INTERVAL_MS);
    expect(snapshotCalls()).toBe(afterFirst);
    await vi.advanceTimersByTimeAsync(CONVERGE_POLL_CONVERGED_MS - CONVERGE_POLL_INTERVAL_MS);
    expect(snapshotCalls()).toBe(afterFirst + 1);
  });

  it("live loop: a fresh Converged apply (within the settle window) keeps the 5s cadence", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "authoritative_snapshot"
        ? Promise.resolve(snap({ convergePhase: "Converged", lastApplyAt: NOW_SEC - 5 }))
        : Promise.resolve(undefined)
    );
    subscribe();
    await vi.advanceTimersByTimeAsync(0);
    const afterFirst = snapshotCalls();
    await vi.advanceTimersByTimeAsync(CONVERGE_POLL_INTERVAL_MS);
    expect(snapshotCalls()).toBe(afterFirst + 1);
    useAppStore.setState({ convergeSnapshot: null });
  });
});

describe("converge loop — subscribe/poll lifecycle", () => {
  it("fetches immediately on subscribe and keeps polling every 5s", async () => {
    subscribe();
    await vi.advanceTimersByTimeAsync(0);
    expect(useAppStore.getState().convergeSnapshot).not.toBeNull();
    expect(useAppStore.getState().convergeSubscribed).toBe(true);
    const afterFirst = snapshotCalls();
    await vi.advanceTimersByTimeAsync(CONVERGE_POLL_INTERVAL_MS);
    expect(snapshotCalls()).toBe(afterFirst + 1);
    await vi.advanceTimersByTimeAsync(CONVERGE_POLL_INTERVAL_MS);
    expect(snapshotCalls()).toBe(afterFirst + 2);
  });

  it("is idempotent: double subscribe does not double the cadence", async () => {
    subscribe();
    useAppStore.getState().subscribeToConverge();
    await vi.advanceTimersByTimeAsync(CONVERGE_POLL_INTERVAL_MS * 3);
    // 1 (immediate) + 3 (ticks at 5s/10s/15s) = 4, not 8.
    expect(snapshotCalls()).toBe(4);
  });

  it("unsubscribing halts polling and clears the subscription flag", async () => {
    const unsub = subscribe();
    await vi.advanceTimersByTimeAsync(0);
    unsub();
    expect(useAppStore.getState().convergeSubscribed).toBe(false);
    const afterStop = snapshotCalls();
    await vi.advanceTimersByTimeAsync(CONVERGE_POLL_INTERVAL_MS * 3);
    expect(snapshotCalls()).toBe(afterStop);
  });

  it("degrades to null snapshot on fetch failure (no fake Unknown pill), loop stays alive", async () => {
    subscribe();
    await vi.advanceTimersByTimeAsync(0);
    expect(useAppStore.getState().convergeSnapshot).not.toBeNull();
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "authoritative_snapshot" ? Promise.reject(new Error("sidecar down")) : Promise.resolve(undefined)
    );
    await vi.advanceTimersByTimeAsync(CONVERGE_POLL_INTERVAL_MS);
    expect(useAppStore.getState().convergeSnapshot).toBeNull();
    // Restore the sidecar: the next tick recovers the snapshot.
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "authoritative_snapshot" ? Promise.resolve(snap()) : Promise.resolve(undefined)
    );
    await vi.advanceTimersByTimeAsync(CONVERGE_POLL_INTERVAL_MS);
    expect(useAppStore.getState().convergeSnapshot).not.toBeNull();
  });
});

describe("converge loop — sidecar-status boost (checkpoint B)", () => {
  it("subscribes to the existing sidecar-status channel and refetches on event", async () => {
    subscribe();
    await vi.advanceTimersByTimeAsync(0);
    expect(listenMock).toHaveBeenCalledWith(SIDECAR_STATUS_EVENT, expect.any(Function));
    const before = snapshotCalls();
    fireSidecarStatus("restarting");
    await vi.advanceTimersByTimeAsync(0);
    expect(snapshotCalls()).toBe(before + 1);
  });

  it("boosts on every lifecycle payload (healthy/unhealthy/terminated/restarting)", async () => {
    subscribe();
    await vi.advanceTimersByTimeAsync(0);
    const before = snapshotCalls();
    for (const payload of ["healthy", "unhealthy", "terminated", "restarting"]) {
      fireSidecarStatus(payload);
      await vi.advanceTimersByTimeAsync(0);
    }
    expect(snapshotCalls()).toBe(before + 4);
  });

  it("boosts after unsubscribe are ignored (unlisten removes the handler)", async () => {
    const unsub = subscribe();
    await vi.advanceTimersByTimeAsync(0);
    unsub();
    expect(registered.length).toBe(0);
    const before = snapshotCalls();
    fireSidecarStatus("unhealthy");
    await vi.advanceTimersByTimeAsync(CONVERGE_POLL_INTERVAL_MS * 2);
    expect(snapshotCalls()).toBe(before);
  });
});

describe("converge loop — visibility pause (checkpoint D)", () => {
  function setVisibility(state: "visible" | "hidden"): void {
    Object.defineProperty(document, "visibilityState", { configurable: true, value: state });
    document.dispatchEvent(new Event("visibilitychange"));
  }

  it("pauses polling while hidden and refetches once on resume", async () => {
    subscribe();
    await vi.advanceTimersByTimeAsync(0);
    setVisibility("hidden");
    expect(useAppStore.getState().convergePausedByVisibility).toBe(true);
    const whileHidden = snapshotCalls();
    await vi.advanceTimersByTimeAsync(CONVERGE_POLL_INTERVAL_MS * 3);
    expect(snapshotCalls()).toBe(whileHidden);
    setVisibility("visible");
    // Resume refetches immediately (before any 5s tick).
    expect(useAppStore.getState().convergePausedByVisibility).toBe(false);
    expect(snapshotCalls()).toBe(whileHidden + 1);
    await vi.advanceTimersByTimeAsync(0);
    // ...and the cadence re-arms (next tick 5s later).
    await vi.advanceTimersByTimeAsync(CONVERGE_POLL_INTERVAL_MS);
    expect(snapshotCalls()).toBe(whileHidden + 2);
  });

  it("a settled Converged snapshot resumes at the 30s cadence after visibility returns", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "authoritative_snapshot"
        ? Promise.resolve(snap({ convergePhase: "Converged", lastApplyAt: NOW_SEC - 3600 }))
        : Promise.resolve(undefined)
    );
    subscribe();
    await vi.advanceTimersByTimeAsync(0);
    setVisibility("hidden");
    await vi.advanceTimersByTimeAsync(CONVERGE_POLL_CONVERGED_MS * 2);
    const whileHidden = snapshotCalls();
    setVisibility("visible");
    await vi.advanceTimersByTimeAsync(0);
    const resumed = snapshotCalls();
    expect(resumed).toBe(whileHidden + 1);
    // Re-armed at the backed-off cadence: no tick inside the 5s..29s window.
    await vi.advanceTimersByTimeAsync(CONVERGE_POLL_INTERVAL_MS);
    expect(snapshotCalls()).toBe(resumed);
    await vi.advanceTimersByTimeAsync(CONVERGE_POLL_CONVERGED_MS - CONVERGE_POLL_INTERVAL_MS);
    expect(snapshotCalls()).toBe(resumed + 1);
  });

  it("hidden at subscribe time: only the immediate fetch runs, no polling ticks", async () => {
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
    subscribe();
    await vi.advanceTimersByTimeAsync(0);
    expect(useAppStore.getState().convergePausedByVisibility).toBe(true);
    // The one-shot immediate fetch still lands (display truth bootstrap)…
    expect(snapshotCalls()).toBe(1);
    // …but no cadence ticks while hidden.
    await vi.advanceTimersByTimeAsync(CONVERGE_POLL_INTERVAL_MS * 2);
    expect(snapshotCalls()).toBe(1);
    setVisibility("visible");
    await vi.advanceTimersByTimeAsync(0);
    expect(snapshotCalls()).toBe(2);
  });
});
