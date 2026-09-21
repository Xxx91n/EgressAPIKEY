import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";

// C2-3: keyCandidates survives an app restart because they are persisted in
// settings.json#keyCandidates via tauri-plugin-store's LazyStore. The shared
// setup.ts mock for "@tauri-apps/plugin-store" hardcodes get->null and set->noop,
// which hides whether the lib actually round-trips. This dedicated spec mounts a
// per-spec in-memory LazyStore backed by a Map so the read-after-write path can
// be exercised end-to-end without touching the Tauri runtime.

const backing = new Map<string, unknown>();
let saveCalls = 0;

vi.mock("@tauri-apps/plugin-store", () => ({
  LazyStore: vi.fn().mockImplementation(() => ({
    get: vi.fn(async (k: string) => backing.get(k) ?? null),
    set: vi.fn(async (k: string, v: unknown) => {
      backing.set(k, v);
    }),
    delete: vi.fn(async (k: string) => {
      backing.delete(k);
    }),
    save: vi.fn(async () => {
      saveCalls++;
    }),
  })),
}));

// the diag-poll-interval wrapper pair goes through @tauri-apps/api/core
// invoke, not LazyStore — mock it so §7.5 boundary behavior is testable.
const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

// Import AFTER the mock so the module reads the mocked LazyStore.
import { loadKeyCandidates, saveKeyCandidates, type KeyCandidate, loadNodeProbe, saveNodeProbe, batchChunkSize, type NodeProbeConfig, purgeLegacyDeadKeys, getDiagPollInterval, setDiagPollInterval, saveView } from "./settings";
import * as settingsModule from "./settings";

beforeEach(() => { backing.clear(); saveCalls = 0; });

describe("C2-3: keyCandidates persist round-trip through settings.json#keyCandidates", () => {
  it("saveKeyCandidates then loadKeyCandidates returns the same list (in-process round-trip)", async () => {
    const list: KeyCandidate[] = [
      { uid: "12345678", endpoint: "https://api.openai.com/v1", apiKey: "sk-aaa111222333444555" },
      { uid: "9abcdef0", endpoint: "https://api.anthropic.com/v1", apiKey: "sk-bbb666777888999000" },
    ];
    await saveKeyCandidates(list);
    const got = await loadKeyCandidates();
    expect(got).not.toBeNull();
    expect(got).toHaveLength(2);
    expect(got![0].endpoint).toBe("https://api.openai.com/v1");
    expect(got![1].apiKey).toBe("sk-bbb666777888999000");
    // UIDs survive verbatim (no mutation/serialization drift)
    expect(got![0].uid).toBe("12345678");
    expect(got![1].uid).toBe("9abcdef0");
  });

  it("loadKeyCandidates returns null when the store key has never been set", async () => {
    const got = await loadKeyCandidates();
    expect(got).toBeNull();
  });

  it("saveKeyCandidates overwrites the prior list (no merge, no stale entries)", async () => {
    await saveKeyCandidates([
      { uid: "00000001", endpoint: "https://a.example/v1", apiKey: "sk-old111222333444555" },
    ]);
    await saveKeyCandidates([
      { uid: "00000002", endpoint: "https://b.example/v1", apiKey: "sk-new111222333444555" },
    ]);
    const got = await loadKeyCandidates();
    expect(got).toHaveLength(1);
    expect(got![0].endpoint).toBe("https://b.example/v1");
  });

  it("an empty array persists as an empty array (NOT collapsed to null)", async () => {
    await saveKeyCandidates([]);
    const got = await loadKeyCandidates();
    expect(Array.isArray(got)).toBe(true);
    expect(got).toHaveLength(0);
  });
});

describe("T19-P4: batchChunkSize pure helper (clash-verge-rev hard cap)", () => {
  it("returns min(concurrency, itemCount, 10) when all > 1", () => {
    expect(batchChunkSize(20, 50)).toBe(10); // capped at 10
    expect(batchChunkSize(5, 50)).toBe(5);   // concurrency is the limit
    expect(batchChunkSize(20, 3)).toBe(3);   // item count is the limit
  });

  it("returns 1 when concurrency or itemCount is 0/1", () => {
    expect(batchChunkSize(0, 50)).toBe(1);   // concurrency 0 → floor at 1
    expect(batchChunkSize(50, 0)).toBe(1);   // itemCount 0 → floor at 1
    expect(batchChunkSize(1, 1)).toBe(1);    // both 1
  });

  it("caps at 10 even when concurrency and itemCount are very large", () => {
    expect(batchChunkSize(100, 100)).toBe(10);
    expect(batchChunkSize(1000, 10000)).toBe(10);
  });
});

describe("T19-P4: nodeProbe config round-trip through settings.json#nodeProbe", () => {
  it("saveNodeProbe then loadNodeProbe returns clamped values", async () => {
    const cfg: NodeProbeConfig = { concurrency: 15, timeout_ms: 8000, batch_on_load: true };
    await saveNodeProbe(cfg);
    const got = await loadNodeProbe();
    expect(got.concurrency).toBe(15);
    expect(got.timeout_ms).toBe(8000);
    expect(got.batch_on_load).toBe(true);
  });

  it("loadNodeProbe clamps concurrency to 1-50 range", async () => {
    await saveNodeProbe({ concurrency: 100, timeout_ms: 10000, batch_on_load: false });
    const got = await loadNodeProbe();
    expect(got.concurrency).toBe(50);
  });

  it("loadNodeProbe clamps timeout_ms to 1000-30000 range", async () => {
    await saveNodeProbe({ concurrency: 10, timeout_ms: 500, batch_on_load: false });
    const got = await loadNodeProbe();
    expect(got.timeout_ms).toBe(1000);
  });

  it("loadNodeProbe returns defaults when store key is unset", async () => {
    backing.delete("nodeProbe");
    const got = await loadNodeProbe();
    expect(got.concurrency).toBe(10);
    expect(got.timeout_ms).toBe(10000);
    expect(got.batch_on_load).toBe(false);
  });
});

describe("arch/02: legacy dead network keys are purged on startup, live keys untouched", () => {
  it("removes both legacy dead keys and preserves every other key", async () => {
    backing.set("gatewayBind", "127.0.0.1:7897");
    backing.set("mihomoApi", "http://127.0.0.1:9090");
    backing.set("lang", "zh");
    backing.set("theme", "dark");
    await purgeLegacyDeadKeys();
    expect(backing.get("gatewayBind")).toBeUndefined();
    expect(backing.get("mihomoApi")).toBeUndefined();
    expect(backing.get("lang")).toBe("zh");
    expect(backing.get("theme")).toBe("dark");
    expect(saveCalls).toBe(1); // one persisted save, not per-key churn
  });

  it("is a no-op when the dead keys are already absent (no save churn)", async () => {
    backing.set("lang", "en");
    await purgeLegacyDeadKeys();
    expect(backing.get("lang")).toBe("en");
    expect(saveCalls).toBe(0);
  });

  it("is idempotent: a second run finds nothing left to remove", async () => {
    backing.set("gatewayBind", "127.0.0.1:7897");
    await purgeLegacyDeadKeys();
    await purgeLegacyDeadKeys();
    expect(backing.get("gatewayBind")).toBeUndefined();
    expect(saveCalls).toBe(1);
  });

  it("no longer exports the legacy dead-key load/save helpers", () => {
    const mod = settingsModule as unknown as Record<string, unknown>;
    expect(mod.loadGatewayBind).toBeUndefined();
    expect(mod.saveGatewayBind).toBeUndefined();
    expect(mod.loadMihomoApi).toBeUndefined();
    expect(mod.saveMihomoApi).toBeUndefined();
  });
});

describe("T05: diagPollInterval typed L1 wrapper pair (bare store invoke eliminated)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async () => 5000);
  });

  it("getDiagPollInterval invokes the typed command and returns the number", async () => {
    invokeMock.mockImplementation(async (cmd: string) => (cmd === "get_diag_poll_interval" ? 8000 : 0));
    await expect(getDiagPollInterval()).resolves.toBe(8000);
    expect(invokeMock).toHaveBeenCalledWith("get_diag_poll_interval", expect.anything());
  });

  it("getDiagPollInterval falls back to 5000 when the command fails (outside Tauri)", async () => {
    invokeMock.mockImplementation(async () => {
      throw new Error("not in tauri");
    });
    await expect(getDiagPollInterval()).resolves.toBe(5000);
  });

  it("setDiagPollInterval accepts the §7.5 boundaries 100 and 24h", async () => {
    invokeMock.mockImplementation(async () => undefined);
    await expect(setDiagPollInterval(100)).resolves.toBeUndefined();
    expect(invokeMock).toHaveBeenCalledWith("set_diag_poll_interval", expect.objectContaining({ intervalMs: 100 }));
    await expect(setDiagPollInterval(24 * 60 * 60 * 1000)).resolves.toBeUndefined();
  });

  it("setDiagPollInterval rejects 0, negatives and above 24h at the TS boundary (no invoke)", async () => {
    invokeMock.mockImplementation(async () => undefined);
    for (const bad of [0, -1, 99, 24 * 60 * 60 * 1000 + 1, Number.NaN, Number.POSITIVE_INFINITY]) {
      await expect(setDiagPollInterval(bad)).rejects.toThrow(/interval_ms must be 100\.\.=86400000/);
    }
    // §7.5 contract: the out-of-range value never reaches the IPC layer.
    expect(invokeMock).not.toHaveBeenCalled();
  });
});

describe("HeadlessStore: serialized saves (last writer wins)", () => {
  // ADR-0071 headless store: overlapping save() calls must PUT in call
  // order - the LAST writer's snapshot must be the final write on the wire.
  // The Tauri injection flags are cleared so store() selects HeadlessStore,
  // and fetch is stubbed with deferred PUT gates to control the wire order.
  type FlagBag = { __TAURI_INTERNALS__?: unknown; isTauri?: unknown };
  const flagBags = () => [globalThis as unknown as FlagBag, window as unknown as FlagBag];
  let savedFlags: { i: unknown; f: unknown }[] = [];
  let realFetch: typeof globalThis.fetch;

  beforeEach(() => {
    savedFlags = flagBags().map((b) => ({ i: b.__TAURI_INTERNALS__, f: b.isTauri }));
    for (const b of flagBags()) {
      b.__TAURI_INTERNALS__ = undefined;
      b.isTauri = undefined;
    }
    realFetch = globalThis.fetch;
  });

  afterEach(() => {
    flagBags().forEach((b, i) => {
      b.__TAURI_INTERNALS__ = savedFlags[i]?.i;
      b.isTauri = savedFlags[i]?.f;
    });
    globalThis.fetch = realFetch;
  });

  it("an in-flight save blocks the next PUT until it resolves", async () => {
    let serverDoc: Record<string, unknown> = {};
    const putOrder: unknown[] = [];
    const gates: Array<() => void> = [];
    globalThis.fetch = (async (_input: unknown, init?: { method?: string; body?: unknown }) => {
      if (init?.method === "PUT") {
        return new Promise((resolve) => {
          gates.push(() => {
            serverDoc = JSON.parse(String(init.body));
            putOrder.push(serverDoc.view);
            resolve({ ok: true } as Response);
          });
        }) as Promise<Response>;
      }
      return { ok: true, json: async () => serverDoc } as Response;
    }) as typeof fetch;

    const p1 = saveView("alpha");
    await vi.waitFor(() => expect(gates.length).toBe(1)); // GET resolved, PUT1 in flight
    const p2 = saveView("beta");
    // Give the second save's microtasks room: an unserialized save would
    // already have its PUT in flight here (gates.length === 2).
    await new Promise((r) => setTimeout(r, 20));
    expect(gates.length).toBe(1);
    gates[0](); // PUT1 resolves -> server applies {view:"alpha"}
    await p1;
    await vi.waitFor(() => expect(gates.length).toBe(2));
    gates[1](); // PUT2 resolves last -> {view:"beta"} is the final write
    await p2;
    expect(putOrder).toEqual(["alpha", "beta"]);
    expect(serverDoc.view).toBe("beta");
  });
});
