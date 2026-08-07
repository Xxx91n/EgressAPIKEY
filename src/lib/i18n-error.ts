/**
 * T3-Q3: Unified error display helper. The Rust map_resin_error function
 * returns i18n keys (e.g. "error.cannotDeleteDefaultPlatform") for known
 * Resin errors. For unknown errors it returns the raw string. This helper
 * tries t() on the key; if the key has no translation, falls back to the
 * raw string so the user always sees something meaningful.
 */
import type { TFunction } from "i18next";

export function translateError(e: unknown, t: TFunction): string {
  const raw = e instanceof Error ? e.message : String(e);
  // map_resin_error keys start with "error."
  if (raw.startsWith("error.")) {
    return t(raw, { defaultValue: raw });
  }
  // Unknown errors: try as a key first, fall back to literal
  return t(raw, { defaultValue: raw });
}
