import { describe, it, expect, vi, beforeEach } from "vitest";

/// ADR-0061: regression guard for the config_export/import
/// wrappers. The backend now reads/writes the L2 whitebox files (never Resin),
/// and the CMD_TO_HTTP mapping to the non-existent /api/v1/config/* routes was
/// deleted — config_export/config_import are Tauri-only. These tests lock the
/// wrapper dispatch (command name + forwarded args) so the TS boundary stays
/// aligned with the whitebox-source backend.

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

// import AFTER the mock is registered (matches ipc.test.ts).
import { ipcConfigExport, ipcConfigImport } from "./ipc";

describe("config export/import wrappers (ADR-0061)", () => {
  beforeEach(() => { invokeMock.mockReset(); });

  it("ipcConfigExport dispatches the Tauri command", async () => {
    invokeMock.mockResolvedValue({ strategy: { version: 1, platforms: [] }, ports: { version: 1, entry_ports: [] } });
    await ipcConfigExport();
    expect(invokeMock).toHaveBeenCalledWith("config_export", expect.anything());
  });

  it("ipcConfigImport forwards the config document to the backend", async () => {
    invokeMock.mockResolvedValue({ platforms_created: 1, subscriptions_created: 0, errors: [] });
    const doc = {
      version: 1,
      strategy: { version: 1, platforms: [] },
      ports: { version: 1, entry_ports: [] },
    };
    await ipcConfigImport(doc);
    expect(invokeMock).toHaveBeenCalledWith("config_import", expect.objectContaining({ config: doc }));
  });
});
