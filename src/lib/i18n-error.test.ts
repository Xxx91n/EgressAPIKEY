import { describe, it, expect, vi, beforeEach } from "vitest";
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
});
