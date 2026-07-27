/**
 * Persisted-user-preferences bridge to tauri-plugin-store.
 *
 * The Zustand appStore stays pure (no Tauri import) so it remains unit-testable
 * in vitest without a webview. This module is the only place that talks to
 * the Tauri store plugin; views/effects call these helpers and then update
 * the appStore + apply side effects (html class, tray refresh) themselves.
 *
 * Keys: "lang" (Locale), "theme" (Theme). The Rust tray reads "lang" directly
 * via tauri-plugin-store (see src-tauri/src/tray.rs::current_lang) so the tray
 * is localised even before the webview mounts.
 */

import { LazyStore } from "@tauri-apps/plugin-store";

const STORE = "settings.json";

function store() {
  return new LazyStore(STORE);
}

export async function loadLocale(): Promise<string | null> {
  try {
    return await store().get<string>("lang") ?? null;
  } catch {
    return null; // not in a tauri context (vitest) — fall back
  }
}

export async function saveLocale(lang: string): Promise<void> {
  try {
    await store().set("lang", lang);
    await store().save();
  } catch {
    /* noop outside tauri */
  }
}

export async function loadTheme(): Promise<string | null> {
  try {
    return await store().get<string>("theme") ?? null;
  } catch {
    return null;
  }
}

export async function saveTheme(theme: string): Promise<void> {
  try {
    await store().set("theme", theme);
    await store().save();
  } catch {
    /* noop outside tauri */
  }
}
export async function loadLaneCount(): Promise<number | null> {
  try {
    const v = await store().get<number>("laneCount");
    return typeof v === "number" ? v : null;
  } catch {
    return null;
  }
}

export async function saveLaneCount(n: number): Promise<void> {
  try {
    await store().set("laneCount", n);
    await store().save();
  } catch {
    /* noop outside tauri */
  }
}

/// Gateway bind address ("127.0.0.1:7897") — read by the Rust shell at
/// startup into CoreConfig::bind. The Rust side is the trust boundary: it
/// coerces to a loopback bind; a non-loopback value saved here is refused at
/// the kernel, never piped through a raw-string IPC command (AGENTS §7.6).
export async function loadGatewayBind(): Promise<string | null> {
  try {
    return await store().get<string>("gatewayBind") ?? null;
  } catch {
    return null;
  }
}

export async function saveGatewayBind(addr: string): Promise<void> {
  try {
    await store().set("gatewayBind", addr);
    await store().save();
  } catch {
    /* noop outside tauri */
  }
}

/// mihomo REST API base URL ("http://127.0.0.1:9090"). The Rust shell reads
/// this into CoreConfig::mihomo_api and constructs MihomoController::new from
/// it ONLY if loopback (see crates/resin-core/src/mihomo.rs loopback guard);
/// a non-loopback url saved here is rejected at construction, never via an
/// IPC command taking a raw String (§7.6) — settings.json is server-trusted.
export async function loadMihomoApi(): Promise<string | null> {
  try {
    return await store().get<string>("mihomoApi") ?? null;
  } catch {
    return null;
  }
}

export async function saveMihomoApi(url: string): Promise<void> {
  try {
    await store().set("mihomoApi", url);
    await store().save();
  } catch {
    /* noop outside tauri */
  }
}
