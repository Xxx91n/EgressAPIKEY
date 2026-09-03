/**
 * Persisted-user-preferences bridge to tauri-plugin-store.
 *
 * The Zustand appStore stays pure (no Tauri import) so it remains unit-testable
 * in vitest without a webview. This module is the only place that talks to
 * the Tauri store plugin; views/effects call these helpers and then update
 * the appStore + apply side effects (html class, tray refresh) themselves.
 *
 * Persisted keys (all in settings.json, single source of truth): "lang" (Locale),
 * "theme" (Theme). P19: "topologyViewport" ({x,y,zoom} canvas pan/zoom memory),
 * P19: "localSubOrder" (string[]) - user's drag-reorder override for subscriptions.
 * P19: "topologyViewport" ({x,y,zoom} canvas pan/zoom memory),
 * P19: "localSubOrder" (string[]) - user's drag-reorder override for subscriptions. The Rust tray reads "lang" directly via
 * tauri-plugin-store (see src-tauri/src/tray.rs::current_lang) so the tray is
 * localised even before the webview mounts. The legacy network preference
 * keys were never effective after T3-A (ADR-0012: the Resin sidecar owns the
 * listen port); purgeLegacyDeadKeys() removes any residue once at startup.
 */

import { LazyStore } from "@tauri-apps/plugin-store";
import { invoke } from "@tauri-apps/api/core";

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

// --- Migration (arch/02): one-time removal of legacy dead keys ---
// These keys were read by a pre-T3-A main.rs block that dropped its
// CoreConfig on the floor (ADR-0012: the Resin sidecar owns the listen
// port). This const is the ONLY remaining live-code mention of the dead
// keys — delete this block and its test once the migration window closes.
const LEGACY_DEAD_KEYS = ["gatewayBind", "mihomoApi"] as const;

export async function purgeLegacyDeadKeys(): Promise<void> {
  try {
    let removed = false;
    for (const key of LEGACY_DEAD_KEYS) {
      if ((await store().get<string>(key)) != null) {
        await store().delete(key);
        removed = true;
        console.info(`[settings] removed legacy dead key "${key}" (never effective since T3-A)`);
      }
    }
    if (removed) await store().save();
  } catch {
    // not in a tauri context (vitest) — nothing to purge
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

// Process routes (ticket 17 / ADR-0055): the L1 persistence pair was
// DELETED — the family lives in the L2 whitebox (egressapikey-ports.json
// process_routes field) with the Rust process_route_* command family as the
// single write entry. The frontend reads/writes through src/lib/ipc.ts only.
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


// --- P19 item 1: Topology canvas viewport memory ({x, y, zoom}) ---
// ReactFlow onMoveEnd fires after every pan/zoom completes; we persist the
// resulting viewport so reopening the topology view lands the user back at
// the exact spot they left. Ponytail: persisted as a single small object,
// re-applied on mount via useReactFlow().setViewport before data lands so the
// fitView() call (now gated behind "no saved viewport") doesn't fight it.
export interface TopologyState {
  x: number;
  y: number;
  zoom: number;
  viewMode: "subscription" | "region";
  locked: boolean;
}
/// Backward compat alias.
export type TopologyViewport = Pick<TopologyState, "x" | "y" | "zoom">;

export async function loadTopologyState(): Promise<TopologyState | null> {
  try {
    let v = await store().get<TopologyState>("topologyState");
    // T15-review migration: if topologyState absent, fall back to legacy topologyViewport key
    if (!v) {
      const legacy = await store().get<TopologyState>("topologyViewport");
      if (legacy && typeof legacy.x === "number" && typeof legacy.y === "number" && typeof legacy.zoom === "number") {
        v = { x: legacy.x, y: legacy.y, zoom: legacy.zoom, viewMode: legacy.viewMode === "region" ? "region" : "subscription", locked: legacy.locked === true };
      }
    }
    if (!v || typeof v.x !== "number" || typeof v.y !== "number" || typeof v.zoom !== "number")
      return null;
    return {
      x: v.x, y: v.y, zoom: v.zoom,
      viewMode: v.viewMode === "region" ? "region" : "subscription",
      locked: v.locked === true,
    };
  } catch { return null; }
}
/// Backward compat: load only viewport portion.
export async function loadTopologyViewport(): Promise<TopologyViewport | null> {
  const s = await loadTopologyState();
  return s ? { x: s.x, y: s.y, zoom: s.zoom } : null;
}

export async function saveTopologyState(s: TopologyState): Promise<void> {
  try {
    await store().set("topologyState", s);
    await store().save();
  } catch (e) { console.warn("[settings] save failed:", e); }
}
/// Backward compat: save only viewport portion (merges with existing state).
export async function saveTopologyViewport(vp: TopologyViewport): Promise<void> {
  const existing = await loadTopologyState();
  await saveTopologyState({ ...existing, ...vp, viewMode: existing?.viewMode ?? "subscription", locked: existing?.locked ?? false });
}

// --- P19 item 6: Subscription list drag-order override ---
// Resin /api/v1/subscriptions returns the list sorted by updated_at (server-
// defined). The user's drag-reorder would be lost on every 10s refresh.
// We persist a string[] of subscription names in the user's chosen order; the
// refresh sorts server results by this override first, and new subs append
// to the end so the override stays the single source of layout truth.
export async function loadSubOrder(): Promise<string[] | null> {
  try {
    const v = await store().get<string[]>("localSubOrder");
    return Array.isArray(v) ? v : null;
  } catch { return null; }
}

export async function saveSubOrder(order: string[]): Promise<void> {
  try {
    await store().set("localSubOrder", order);
    await store().save();
  } catch (e) { console.warn("[settings] save failed:", e); }
}

// --- P21-B: Key candidates (left pane of PlatformsView dual-pane) ---
// Each candidate = { uid, endpoint, apiKey } where uid = sha1(endpoint+"::"+apiKey)[:8].
// Persisted in settings.json#keyCandidates; this is shell-local, never sent to Resin.
export interface KeyCandidate {
  uid: string;
  endpoint: string;
  apiKey: string;
}

export async function loadKeyCandidates(): Promise<KeyCandidate[] | null> {
  try {
    const v = await store().get<KeyCandidate[]>("keyCandidates");
    return Array.isArray(v) ? v : null;
  } catch { return null; }
}

export async function saveKeyCandidates(candidates: KeyCandidate[]): Promise<void> {
  try {
    await store().set("keyCandidates", candidates);
    await store().save();
  } catch (e) { console.warn("[settings] save failed:", e); }
}

// --- P21-B: Splitter ratio for dual-pane PlatformsView ---
export async function loadSplitRatio(): Promise<number | null> {
  try {
    const v = await store().get<number>("splitRatio");
    return typeof v === "number" ? v : null;
  } catch { return null; }
}

export async function saveSplitRatio(ratio: number): Promise<void> {
  try {
    await store().set("splitRatio", ratio);
    await store().save();
  } catch (e) { console.warn("[settings] save failed:", e); }
}

// --- P21-C: IP channel policy map (GUI label -> Resin allocation_policy) ---
export async function loadIpChannelPolicyMap(): Promise<Record<string, string> | null> {
  try {
    const v = await store().get<Record<string, string>>("ipChannelPolicyMap");
    return v && typeof v === "object" ? v : null;
  } catch { return null; }
}

export async function saveIpChannelPolicyMap(m: Record<string, string>): Promise<void> {
  try {
    await store().set("ipChannelPolicyMap", m);
    await store().save();
  } catch (e) { console.warn("[settings] save failed:", e); }
}


export interface IpReputationConfig {
  provider: "" | "ip_quality_score" | "abuse_ip_db" | "ip_api";
  ipQualityScoreApiKey: string;
  abuseIpDbApiKey: string;
}

export async function loadIpReputationConfig(): Promise<IpReputationConfig> {
  try {
    const [provider, ipQualityScoreApiKey, abuseIpDbApiKey] = await Promise.all([
      store().get<string>("ipReputationProvider"),
      store().get<string>("ipQualityScoreApiKey"),
      store().get<string>("abuseIpDbApiKey"),
    ]);
    return {
      provider: provider === "ip_quality_score" || provider === "abuse_ip_db" || provider === "ip_api" ? provider : "",
      ipQualityScoreApiKey: typeof ipQualityScoreApiKey === "string" ? ipQualityScoreApiKey : "",
      abuseIpDbApiKey: typeof abuseIpDbApiKey === "string" ? abuseIpDbApiKey : "",
    };
  } catch { return { provider: "", ipQualityScoreApiKey: "", abuseIpDbApiKey: "" }; }
}

export async function saveIpReputationConfig(cfg: IpReputationConfig): Promise<void> {
  try {
    await Promise.all([
      store().set("ipReputationProvider", cfg.provider),
      store().set("ipQualityScoreApiKey", cfg.ipQualityScoreApiKey.slice(0, 512)),
      store().set("abuseIpDbApiKey", cfg.abuseIpDbApiKey.slice(0, 512)),
    ]);
    await store().save();
  } catch (e) { console.warn("[settings] reputation save failed:", e); }
}

// --- T10-3: Port auth default toggle persistence (ADR-0031) ---
export async function loadPortAuthDefault(): Promise<boolean | null> {
  try {
    const v = await store().get<boolean>("portAuthDefault");
    return typeof v === "boolean" ? v : true; // default true for backward compat
  } catch { return true; }
}

export async function savePortAuthDefault(value: boolean): Promise<void> {
  try {
    await store().set("portAuthDefault", value);
    await store().save();
  } catch (e) { console.warn("[settings] savePortAuthDefault failed:", e); }
}

// --- T19-P4: Node batch probe config (ADR-0044 S4) ---
// Persisted in settings.json#nodeProbe; shell-local, controls batch probe UX.
export interface NodeProbeConfig {
  concurrency: number;   // 1-50, but capped to min(concurrency, items, 10) at runtime
  timeout_ms: number;    // 1000-30000, shell-side timeout guard (Resin has its own 15s)
  batch_on_load: boolean; // default false = manual only (user choice "甲")
}

export async function loadNodeProbe(): Promise<NodeProbeConfig> {
  try {
    const v = await store().get<Partial<NodeProbeConfig>>("nodeProbe");
    return {
      concurrency: typeof v?.concurrency === "number" ? Math.max(1, Math.min(50, v.concurrency)) : 10,
      timeout_ms: typeof v?.timeout_ms === "number" ? Math.max(1000, Math.min(30000, v.timeout_ms)) : 10000,
      batch_on_load: typeof v?.batch_on_load === "boolean" ? v.batch_on_load : false,
    };
  } catch {
    return { concurrency: 10, timeout_ms: 10000, batch_on_load: false };
  }
}

export async function saveNodeProbe(cfg: NodeProbeConfig): Promise<void> {
  try {
    await store().set("nodeProbe", {
      concurrency: Math.max(1, Math.min(50, Math.floor(cfg.concurrency))),
      timeout_ms: Math.max(1000, Math.min(30000, Math.floor(cfg.timeout_ms))),
      batch_on_load: cfg.batch_on_load,
    });
    await store().save();
  } catch (e) { console.warn("[settings] saveNodeProbe failed:", e); }
}

/// Pure helper: batch chunk size = min(concurrency, itemCount, 10) — clash-verge-rev hard cap.
/// Exported for vitest coverage.
export function batchChunkSize(configured: number, itemCount: number): number {
  return Math.max(1, Math.min(configured, itemCount, 10));
}

// --- T05 (Round 5): diagnostics poll interval — typed L1 wrapper pair ---
// Replaces the former DiagnosticsView bare `invoke("get/set_store_value")`
// bypass (those commands were never registered in generate_handler!, so the
// old path failed at runtime and silently fell back to the 5000 default).
// The L1 write surface stays "src/lib/settings.ts + typed commands".
// §7.5 IPC input validation mirrors the Rust boundary: 100..=24h (ms).
const DIAG_POLL_INTERVAL_MIN_MS = 100;
const DIAG_POLL_INTERVAL_MAX_MS = 24 * 60 * 60 * 1000;

/// T05: read the diagnostics poll interval (ms). Returns the 5000 default
/// when unset or unavailable (outside Tauri / vitest).
export async function getDiagPollInterval(): Promise<number> {
  try {
    const v = await invoke<number>("get_diag_poll_interval");
    return typeof v === "number" ? v : 5000;
  } catch {
    return 5000;
  }
}

/// T05: persist the diagnostics poll interval (ms).
/// §7.5: rejects out-of-range values at the TS boundary before invoking.
export async function setDiagPollInterval(ms: number): Promise<void> {
  if (
    typeof ms !== "number" ||
    !Number.isFinite(ms) ||
    ms < DIAG_POLL_INTERVAL_MIN_MS ||
    ms > DIAG_POLL_INTERVAL_MAX_MS
  ) {
    throw new Error(
      `interval_ms must be ${DIAG_POLL_INTERVAL_MIN_MS}..=${DIAG_POLL_INTERVAL_MAX_MS}`,
    );
  }
  await invoke("set_diag_poll_interval", { intervalMs: Math.floor(ms) });
}
