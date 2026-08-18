/**
 * T3-Q3: Unified error display helper. The Rust map_resin_error function
 * returns i18n keys (e.g. "error.cannotDeleteDefaultPlatform") for known
 * Resin errors. For unknown errors it returns the raw string. This helper
 * tries t() on the key; if the key has no translation, falls back to the
 * raw string so the user always sees something meaningful.
 *
 * T3-Q2: keys of the form "error.<name>.<digits>" (e.g.
 * "error.upstream.525", "error.upstream.403") carry an HTTP status code
 * suffix. We parse it into a `{ code }` interpolation variable so the
 * generic "error.<name>" translation can reference `{{code}}` and surface
 * "Upstream HTTP 525" without needing per-code translations. Per-code keys
 * still win when defined. (Historically the first user of this mechanism was
 * the P13 B4 fetch_clash_subscription chain; that chain was deleted in T20-v2
 * along with its error.subscriptionFetch i18n key, but the mechanism stays.)
 */
import type { TFunction } from "i18next";

export function translateError(e: unknown, t: TFunction): string {
  // T8-Bug2: if Tauri rejected with an IpcError object (externally-tagged
  // serde: {kind, data}), extract the i18n_key before String(e) turns it
  // into "[object Object]". extractIpcErr is in ipc.ts but we can't import
  // it here (cycle: ipc.ts imports translateError via PlatformsView). We
  // inline the narrow so i18n-error.ts stays dependency-free.
  if (e && typeof e === "object" && "kind" in e && "data" in e) {
    const data = (e as { data: Record<string, unknown> }).data;
    const i18nKey = typeof data?.i18n_key === "string" ? data.i18n_key : "";
    if (i18nKey) {
      // Gather all interpolation vars from the IpcError data so templates
      // like "Upstream error: {{excerpt}}" or "Port {{port}} in use" work.
      // Gather all interpolation vars used by the 4 IpcError variant templates:
      // BindConflict: {{port}}; ResinUpstream: {{excerpt}}, {{status}};
      // InvalidStrategy: {{value}}, {{accepted}}; Internal: <none, {{msg}} kept for dev display>.
      const port = typeof data?.port === "number" ? String(data.port) : "";
      const status = typeof data?.status === "number" ? String(data.status) : "";
      const excerpt = typeof data?.excerpt === "string" ? data.excerpt : "";
      const value = typeof data?.value === "string" ? data.value : "";
      let accepted = "";
      if (Array.isArray(data?.accepted)) accepted = data.accepted.map(String).join(", ");
      const vars: Record<string, string> = { port, status, excerpt, value, accepted, code: excerpt };
      // t() with interpolation; if the template still has unresolvable
      // {{...}} placeholders, don't return a partial template.
      const resolved = t(i18nKey, { defaultValue: "", ...vars });
      if (resolved && resolved !== i18nKey && !resolved.includes("{{")) return resolved;
      // Fallback to literal i18n_key.
      return t(i18nKey, { defaultValue: i18nKey });
    }
  }
  const raw = e instanceof Error ? e.message : String(e);
  // map_resin_error keys start with "error." — try the literal key first
  // (per-code translations like "error.upstream.525" win here).
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
