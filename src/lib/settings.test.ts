import { describe, it, expect, beforeEach, vi } from "vitest";

// C2-3: keyCandidates survives an app restart because they are persisted in
// settings.json#keyCandidates via tauri-plugin-store's LazyStore. The shared
// setup.ts mock for "@tauri-apps/plugin-store" hardcodes get->null and set->noop,
// which hides whether the lib actually round-trips. This dedicated spec mounts a
// per-spec in-memory LazyStore backed by a Map so the read-after-write path can
// be exercised end-to-end without touching the Tauri runtime.

const backing = new Map<string, unknown>();

vi.mock("@tauri-apps/plugin-store", () => ({
  LazyStore: vi.fn().mockImplementation(() => ({
    get: vi.fn(async (k: string) => backing.get(k) ?? null),
    set: vi.fn(async (k: string, v: unknown) => {
      backing.set(k, v);
    }),
    save: vi.fn(async () => {}),
  })),
}));

// Import AFTER the mock so the module reads the mocked LazyStore.
import { loadKeyCandidates, saveKeyCandidates, type KeyCandidate } from "./settings";

beforeEach(() => backing.clear());

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
