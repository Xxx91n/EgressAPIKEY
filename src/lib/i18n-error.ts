/**
 * T3-Q3: Unified error display helper. The Rust map_resin_error function
 * returns i18n keys (e.g. "error.cannotDeleteDefaultPlatform") for known
 * Resin errors. For unknown errors it returns the raw string. This helper
 * tries t() on the key; if the key has no translation, falls back to the
 * raw string so the user always sees something meaningful.
 *
 * T3-Q2: keys of the form "error.<name>.<digits>" (e.g.
 * "error.subscriptionFetch.525", "error.subscriptionFetch.403") carry an
 * HTTP status code suffix. We parse it into a `{ code }` interpolation
 * variable so the generic "error.<name>" translation can reference
 * `{{code}}` and surface "Subscription fetch failed (HTTP 525)" without
 * needing per-code translations. Per-code keys still win when defined.
 */
import type { TFunction } from "i18next";

export function translateError(e: unknown, t: TFunction): string {
  const raw = e instanceof Error ? e.message : String(e);
  // map_resin_error keys start with "error." — try the literal key first
  // (per-code translations like "error.subscriptionFetch.525" win here).
  if (raw.startsWith("error.")) {
    // For keys of shape "error.<name>.<digits>", fall back to the
    // "error.<name>" parent key with the code as an interpolation variable.
    const m = raw.match(/^(error\.[a-zA-Z0-9_]+)\.(\d+)$/);
    if (m) {
      const parentKey = m[1];
      const code = m[2];
      // Prefer the per-code key; if missing, fall back to the parent key
      // with `{{code}}` interpolation, then to the literal raw string.
      const exact = t(raw, { defaultValue: "" });
      if (exact && exact !== raw) return exact;
      const parent = t(parentKey, { defaultValue: "", code });
      if (parent) return parent;
    }
    return t(raw, { defaultValue: raw });
  }
  // Unknown errors: try as a key first, fall back to literal
  return t(raw, { defaultValue: raw });
}
