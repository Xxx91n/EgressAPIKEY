import { describe, it, expect, vi, beforeEach } from "vitest";

/// Phase 5-1: trace_id passthrough closed-loop test (ADR-0026 Q1-Q5).
/// Verifies that the invoke wrapper always generates a UUID v4 and injects
/// it as __trace_id into the args passed to the Tauri backend.

let invokeMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.resetModules();
  invokeMock = vi.fn(async (_cmd: string, _args?: Record<string, unknown>) => undefined);
  vi.doMock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
});

describe("invokeWithTrace trace_id injection", () => {
  it("always injects __trace_id as UUID v4 format", async () => {
    const { ipcPlatformAdd } = await import("./ipc");
    invokeMock.mockResolvedValueOnce(undefined);
    await ipcPlatformAdd("test-platform");
    expect(invokeMock).toHaveBeenCalledTimes(1);
    const args = invokeMock.mock.calls[0][1] as Record<string, unknown>;
    expect(args.__trace_id).toBeDefined();
    expect(typeof args.__trace_id).toBe("string");
    // UUID v4 format: 8-4-4-4-12 hex dashes
    const uuid = args.__trace_id as string;
    expect(uuid).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[0-9a-f]{4}-[0-9a-f]{12}$/i);
  });

  it("generates unique trace_id on each call", async () => {
    const { ipcPlatformAdd } = await import("./ipc");
    invokeMock.mockResolvedValue(undefined);
    await ipcPlatformAdd("a");
    await ipcPlatformAdd("b");
    const id1 = (invokeMock.mock.calls[0][1] as Record<string, unknown>).__trace_id;
    const id2 = (invokeMock.mock.calls[1][1] as Record<string, unknown>).__trace_id;
    expect(id1).not.toBe(id2);
  });

  it("injects __trace_id alongside existing args", async () => {
    const { ipcPlatformRemove } = await import("./ipc");
    invokeMock.mockResolvedValueOnce(true);
    await ipcPlatformRemove("test");
    const args = invokeMock.mock.calls[0][1] as Record<string, unknown>;
    expect(args.name).toBe("test");
    expect(args.__trace_id).toBeDefined();
  });
});

/// Audit — verify no silent .catch(() => {}) remains in source files.
/// This is a source-level guard: it scans the actual .ts/.tsx files for
/// the empty-catch pattern and fails if any are found.
describe("T10 silent catch audit", () => {
  it("no .catch(() => {}) in src/ source files", async () => {
    const fs = await import("fs");
    const path = await import("path");
    const projectRoot = path.resolve(__dirname, "..", "..");

    function walkDir(dir: string, exts: string[]): string[] {
      const results: string[] = [];
      for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory() && !entry.name.startsWith(".") && entry.name !== "node_modules" && entry.name !== "dist") {
          results.push(...walkDir(full, exts));
        } else if (entry.isFile() && exts.some(e => entry.name.endsWith(e))) {
          results.push(full);
        }
      }
      return results;
    }

    const srcDir = path.join(projectRoot, "src");
    const files = walkDir(srcDir, [".ts", ".tsx"]);
    // Match .catch(() => {}) only in code lines (not comments/strings/test files).
    // Excludes lines starting with /// or // or * (comments).
    const emptyCatchPattern = /.catch\(\(\)\s*=>\s*\{\s*\}\)/;
    const violations: string[] = [];

    for (const file of files) {
      // Skip test files themselves
      if (file.endsWith(".test.ts") || file.endsWith(".test.tsx")) continue;
      const fc = fs.readFileSync(file, "utf8");
      const lines = fc.split("\n");
      for (let i = 0; i < lines.length; i++) {
        const trimmed = lines[i].trimStart();
        // Skip comment lines
        if (trimmed.startsWith("//") || trimmed.startsWith("*") || trimmed.startsWith("/*")) continue;
        if (emptyCatchPattern.test(lines[i])) {
          violations.push(path.relative(projectRoot, file) + ":" + (i + 1));
        }
      }
    }

    if (violations.length > 0) {
      throw new Error("Empty .catch(() => {}) found at:\n  " + violations.join("\n  "));
    }
    expect(violations).toHaveLength(0);
  });
});
