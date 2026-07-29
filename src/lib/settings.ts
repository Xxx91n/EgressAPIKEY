/**
 * Persisted-user-preferences bridge to tauri-plugin-store.
 *
 * The Zustand appStore stays pure (no Tauri import) so it remains unit-testable
 * in vitest without a webview. This module is the only place that talks to
 * the Tauri store plugin; views/effects call these helpers and then update
 * the appStore + apply side effects (html class, tray refresh) themselves.
 *
 * Persisted keys (all in settings.json, single source of truth): "lang" (Locale),
 * "theme" (Theme), "laneCount" (number 1..50), "gatewayBind" (loopback addr),
 * "mihomoApi" (http(s):// URL). The Rust tray reads "lang" directly via
 * tauri-plugin-store (see src-tauri/src/tray.rs::current_lang) so the tray is
 * localised even before the webview mounts. gatewayBind/mihomoApi are read
 * server-side by the Rust shell into CoreConfig; mihomoApi ONLY feeds
 * MihomoController::new which refuses non-loopback URLs (AGENTS §7.6 — never
 * expose these via a raw-String #[tauri::command]).
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
  } catch (e) {
    // UX bug #2: surface tauri-plugin-store failures instead of silently
    // swallowing — the old /* noop */ let saves fail invisibly so the user
    // believed edits were lost. We keep the function non-throwing (vitest runs
    // without a webview) but log so devtools surfaces the real cause.
    console.warn("[settings] save failed:", e);
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
  } catch (e) {
    // UX bug #2: surface tauri-plugin-store failures instead of silently
    // swallowing — the old /* noop */ let saves fail invisibly so the user
    // believed edits were lost. We keep the function non-throwing (vitest runs
    // without a webview) but log so devtools surfaces the real cause.
    console.warn("[settings] save failed:", e);
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
  } catch (e) {
    // UX bug #2: surface tauri-plugin-store failures instead of silently
    // swallowing — the old /* noop */ let saves fail invisibly so the user
    // believed edits were lost. We keep the function non-throwing (vitest runs
    // without a webview) but log so devtools surfaces the real cause.
    console.warn("[settings] save failed:", e);
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
  } catch (e) {
    // UX bug #2: surface tauri-plugin-store failures instead of silently
    // swallowing — the old /* noop */ let saves fail invisibly so the user
    // believed edits were lost. We keep the function non-throwing (vitest runs
    // without a webview) but log so devtools surfaces the real cause.
    console.warn("[settings] save failed:", e);
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
  } catch (e) {
    // UX bug #2: surface tauri-plugin-store failures instead of silently
    // swallowing — the old /* noop */ let saves fail invisibly so the user
    // believed edits were lost. We keep the function non-throwing (vitest runs
    // without a webview) but log so devtools surfaces the real cause.
    console.warn("[settings] save failed:", e);
  }
}

// --- View persistence (last-active tab) ---
export async function loadView(): Promise<string | null> {
  try {
    return await store().get<string>("view") ?? null;
  } catch {
    return null;
  }
}

export async function saveView(view: string): Promise<void> {
  try {
    await store().set("view", view);
    await store().save();
  } catch (e) { console.warn("[settings] save failed:", e); }
}

// --- Process routes persistence (pure-frontend state) ---
export async function loadProcessRoutes<T>(): Promise<T[] | null> {
  try {
    const v = await store().get<T[]>("processRoutes");
    return Array.isArray(v) ? v : null;
  } catch { return null; }
}

export async function saveProcessRoutes(routes: unknown[]): Promise<void> {
  try {
    await store().set("processRoutes", routes);
    await store().save();
  } catch (e) { console.warn("[settings] save failed:", e); }
}

// --- WebDAV backup config persistence (clash-verge-rev pattern) ---
export async function loadWebdavConfig(): Promise<{ url: string; username: string; password: string } | null> {
  try {
    const url = await store().get<string>("webdavUrl");
    const username = await store().get<string>("webdavUsername");
    const password = await store().get<string>("webdavPassword");
    if (url && username) return { url, username, password: password ?? "" };
    return null;
  } catch { return null; }
}

export async function saveWebdavConfig(url: string, username: string, password: string): Promise<void> {
  try {
    await store().set("webdavUrl", url);
    await store().set("webdavUsername", username);
    await store().set("webdavPassword", password);
    await store().save();
  } catch (e) { console.warn("[settings] save failed:", e); }
}
