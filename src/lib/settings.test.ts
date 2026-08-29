import { describe, it, expect, beforeEach, vi } from "vitest";

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

// Import AFTER the mock so the module reads the mocked LazyStore.
import { loadKeyCandidates, saveKeyCandidates, type KeyCandidate, loadNodeProbe, saveNodeProbe, batchChunkSize, type NodeProbeConfig, purgeLegacyDeadKeys } from "./settings";
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
