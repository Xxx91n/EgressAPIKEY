import { describe, it, expect, vi } from "vitest";
import { translateError } from "./i18n-error";

/// T3-Q3: translateError static unit tests — no React render needed.
/// Covers the regex-based fallback used by both PlatformsView and
/// SubscriptionsView catch blocks.

// Minimal i18next-like mock: a Map of key -> text, supporting {{code}}
// interpolation so we can verify the parent-key fallback path.
function makeT(keys: Record<string, string>): any {
  return vi.fn((key: string, opts?: any) => {
    const tmpl = keys[key];
    if (tmpl === undefined) return opts?.defaultValue ?? "";
    let out = tmpl;
    if (opts && typeof opts === "object") {
      for (const [k, v] of Object.entries(opts)) {
        out = out.split("{{" + k + "}}").join(String(v));
      }
    }
    return out;
  });
}

describe("translateError", () => {
  it("maps a known Resin error key via t()", () => {
    const t = makeT({ "error.cannotDeleteDefaultPlatform": "cannot delete default" });
    expect(translateError(new Error("error.cannotDeleteDefaultPlatform"), t)).toBe("cannot delete default");
  });

  it("for error.<name>.<code> falls back to parent key with {{code}} interpolation when per-code key missing", () => {
    const t = makeT({ "error.subscriptionFetch": "Subscription source error (HTTP {{code}})" });
    // key returned by Rust map_resin_error for HTTP 525:
    const out = translateError(new Error("error.subscriptionFetch.525"), t);
    expect(out).toBe("Subscription source error (HTTP 525)");
  });

  it("prefers per-code translation over parent fallback when defined", () => {
    const t = makeT({
      "error.subscriptionFetch": "Subscription source error (HTTP {{code}})",
      "error.subscriptionFetch.525": "Origin SSL handshake failed (HTTP 525); origin server issue, not a client problem",
    });
    expect(translateError(new Error("error.subscriptionFetch.525"), t)).toBe(
      "Origin SSL handshake failed (HTTP 525); origin server issue, not a client problem"
    );
  });

  it("falls back to the literal raw string when no translation exists", () => {
    const t = makeT({});
    const out = translateError(new Error("some.untranslated.error"), t);
    expect(out).toBe("some.untranslated.error");
  });

  it("handles plain Error message (non-error-prefix) by treating it as a literal", () => {
    const t = makeT({});
    const out = translateError(new Error("Network is unreachable"), t);
    expect(out).toBe("Network is unreachable");
  });

  // T8-Bug2: IpcError objects from Tauri must NOT render as "[object Object]".
  // Tauri serializes IpcError as {kind, data:{i18n_key,...}}. The old
  // String(e) path turned this into "[object Object]". translateError now
  // inlines the object narrowing and extracts i18n_key.
  it("T8-Bug2: IpcError {kind:Internal, data:{i18n_key}} renders localized msg not [object Object]", () => {
    const t = makeT({ "error.cannotDeleteDefaultPlatform": "cannot delete Default platform" });
    const ipcErr = { kind: "Internal", data: { msg: "raw message", i18n_key: "error.cannotDeleteDefaultPlatform" } };
    const out = translateError(ipcErr, t);
    expect(out).toBe("cannot delete Default platform");
    expect(out).not.toBe("[object Object]");
  });

  it("T8-Bug2: IpcError {kind:BindConflict, data:{port}} renders localized bind conflict", () => {
    const t = makeT({ "error.bindConflict": "Port already in use" });
    const ipcErr = { kind: "BindConflict", data: { port: 17111, i18n_key: "error.bindConflict" } };
    const out = translateError(ipcErr, t);
    expect(out).toBe("Port already in use");
    expect(out).not.toBe("[object Object]");
  });

  it("T8-Bug2: IpcError {kind:ResinUpstream, data:{excerpt}} renders localized upstream error", () => {
    const t = makeT({ "error.resinUpstream": "Upstream error: {{excerpt}}" });
    const ipcErr = { kind: "ResinUpstream", data: { status: 503, excerpt: "timeout", i18n_key: "error.resinUpstream" } };
    const out = translateError(ipcErr, t);
    expect(out).toBe("Upstream error: timeout");
    expect(out).not.toBe("[object Object]");
  });

  it("T20-P4: IpcError {kind:InvalidStrategy, data:{value, accepted}} renders with value and accepted", () => {
    const t = makeT({ "error.invalidStrategy": "Invalid strategy value: {{value}}. Accepted: {{accepted}}" });
    const ipcErr = {
      kind: "InvalidStrategy",
      data: {
        value: "balanced",
        accepted: ["random", "sequential", "latency", "quality", "bandwidth", "protocol_weight"],
        i18n_key: "error.invalidStrategy",
      },
    };
    const out = translateError(ipcErr, t);
    expect(out).toBe("Invalid strategy value: balanced. Accepted: random, sequential, latency, quality, bandwidth, protocol_weight");
    expect(out).not.toBe("[object Object]");
    expect(out).not.toContain("{{");
  });

  it("T20-P4: IpcError {kind:ResinUpstream} renders with status and excerpt interpolated", () => {
    const t = makeT({ "error.resinUpstream": "Backend returned HTTP {{status}}: {{excerpt}}" });
    const ipcErr = {
      kind: "ResinUpstream",
      data: { status: 503, excerpt: "service unavailable", i18n_key: "error.resinUpstream" },
    };
    const out = translateError(ipcErr, t);
    expect(out).toBe("Backend returned HTTP 503: service unavailable");
    expect(out).not.toContain("{{");
  });
});
