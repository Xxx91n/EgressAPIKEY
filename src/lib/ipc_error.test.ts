import { describe, it, expect } from "vitest";
import { extractIpcErr, ipcErrI18nKey } from "./ipc";

/// Phase 5-2: IpcError extractIpcErr closed-loop tests (ADR-0026 Q6-Q8).
/// Covers 4 variant narrowing + i18n_key presence + template params.

describe("extractIpcErr variant narrowing", () => {
  it("narrow BindConflict with port number", () => {
    const raw = { kind: "BindConflict", data: { port: 17111, i18n_key: "error.bindConflict" } };
    const err = extractIpcErr(raw);
    expect(err.kind).toBe("BindConflict");
    if (err.kind === "BindConflict") {
      expect(err.data.port).toBe(17111);
      expect(err.data.i18n_key).toBe("error.bindConflict");
    }
  });

  it("narrow InvalidStrategy with accepted list", () => {
    const raw = {
      kind: "InvalidStrategy",
      data: {
        value: "balanced",
        accepted: ["random", "sequential", "latency", "quality", "bandwidth", "protocol_weight"],
        i18n_key: "error.invalidStrategy",
      },
    };
    const err = extractIpcErr(raw);
    expect(err.kind).toBe("InvalidStrategy");
    if (err.kind === "InvalidStrategy") {
      expect(err.data.value).toBe("balanced");
      expect(err.data.accepted).toHaveLength(6);
      expect(err.data.accepted[0]).toBe("random");
      expect(err.data.i18n_key).toBe("error.invalidStrategy");
    }
  });

  it("narrow ResinUpstream with status + excerpt", () => {
    const raw = {
      kind: "ResinUpstream",
      data: { status: 503, excerpt: "service unavailable", i18n_key: "error.resinUpstream" },
    };
    const err = extractIpcErr(raw);
    expect(err.kind).toBe("ResinUpstream");
    if (err.kind === "ResinUpstream") {
      expect(err.data.status).toBe(503);
      expect(err.data.excerpt).toContain("service unavailable");
      expect(err.data.i18n_key).toBe("error.resinUpstream");
    }
  });

  it("narrow Internal with msg", () => {
    const raw = { kind: "Internal", data: { msg: "serde deserialize failed", i18n_key: "error.internal" } };
    const err = extractIpcErr(raw);
    expect(err.kind).toBe("Internal");
    if (err.kind === "Internal") {
      expect(err.data.msg).toBe("serde deserialize failed");
      expect(err.data.i18n_key).toBe("error.internal");
    }
  });

  it("fallback to Internal when error is a plain string (Tauri default rejection)", () => {
    const err = extractIpcErr("some raw error string");
    expect(err.kind).toBe("Internal");
    if (err.kind === "Internal") {
      expect(err.data.msg).toBe("some raw error string");
      expect(err.data.i18n_key).toBe("error.internal");
    }
  });

  it("ipcErrI18nKey extracts i18n_key from each variant", () => {
    expect(ipcErrI18nKey({ kind: "BindConflict", data: { port: 8080, i18n_key: "error.bindConflict" } }))
      .toBe("error.bindConflict");
    expect(ipcErrI18nKey({ kind: "InvalidStrategy", data: { value: "x", accepted: [], i18n_key: "error.invalidStrategy" } }))
      .toBe("error.invalidStrategy");
    expect(ipcErrI18nKey({ kind: "ResinUpstream", data: { status: 500, excerpt: "", i18n_key: "error.resinUpstream" } }))
      .toBe("error.resinUpstream");
    expect(ipcErrI18nKey({ kind: "Internal", data: { msg: "", i18n_key: "error.internal" } }))
      .toBe("error.internal");
    // Unknown shape -> fallback key
    expect(ipcErrI18nKey(new Error("boom"))).toBe("error.internal");
  });
});
