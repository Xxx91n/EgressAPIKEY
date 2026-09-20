/**
 * Headless request guard + fetch dispatch.
 *
 * Consumes the declarative route table (./headless-routes.ts) and the
 * disabled-command registry (./headless-availability.ts). lib/ipc.ts's
 * invoke() delegates to invokeHeadless() whenever isTauri() is false, so
 * headless surface changes land in the two data modules and never touch
 * the invoke-wrapper body.
 */
import { CMD_TO_HTTP, type HttpRoute } from "./headless-routes";
import { DISABLED_COMMANDS, IpcUnavailableError } from "./headless-availability";

/** Fill `{arg}` placeholders from the Tauri args as URL-encoded segments. */
export function buildPath(route: HttpRoute, args?: Record<string, unknown>): string {
  return route.path.replace(/\{([A-Za-z0-9_]+)\}/g, (_m, key: string) => {
    const v = args?.[key];
    if (v === undefined || v === null) {
      throw new Error(`[ipc] missing path arg "${key}" for ${route.method} ${route.path}`);
    }
    return encodeURIComponent(String(v));
  });
}

/** GET query string: constant params first, then the declared arg passthrough. */
export function buildQuery(route: HttpRoute, args?: Record<string, unknown>): string {
  const qs = new URLSearchParams();
  for (const [k, v] of Object.entries(route.queryConst ?? {})) qs.set(k, v);
  for (const [argKey, param] of Object.entries(route.query ?? {})) {
    const v = args?.[argKey];
    if (v === undefined || v === null) continue;
    qs.set(param, String(v));
  }
  const s = qs.toString();
  return s ? `?${s}` : "";
}

/** Non-GET body: unwrap the Tauri arg envelope, rename camelCase keys, and
 *  drop nulls (the "only provided fields" contract Resin and the BFF share). */
export function buildBody(route: HttpRoute, args?: Record<string, unknown>): string | undefined {
  if (route.method === "GET" || (!args && !route.bodyConst)) return undefined;
  let payload: Record<string, unknown> = { ...(route.bodyConst ?? {}), ...(args ?? {}) };
  if (route.bodyArg) {
    const inner = args?.[route.bodyArg];
    if (inner === undefined || inner === null) return undefined;
    if (typeof inner !== "object" || Array.isArray(inner)) {
      throw new Error(`[ipc] ${route.bodyArg} must be a JSON object`);
    }
    payload = inner as Record<string, unknown>;
  }
  if (route.bodyKeys) {
    const renamed: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(payload)) {
      if (v === null || v === undefined) continue;
      renamed[route.bodyKeys[k] ?? k] = v;
    }
    payload = renamed;
  }
  return JSON.stringify(payload);
}

export async function invokeHttp<T>(route: HttpRoute, args?: Record<string, unknown>): Promise<T> {
  const init: RequestInit = {
    method: route.method,
    headers: { "Content-Type": "application/json" },
  };
  const body = buildBody(route, args);
  if (body !== undefined) init.body = body;
  const url = `${buildPath(route, args)}${buildQuery(route, args)}`;
  const r = await fetch(url, init);
  if (!r.ok) {
    const text = await r.text().catch(() => "");
    throw new Error(`IPC ${route.method} ${url} -> ${r.status}: ${text.slice(0, 256)}`);
  }
  if (r.status === 204) return undefined as T;
  const json = await r.json();
  // T22: Resin wraps list reads as { items: [...], total, limit, offset }.
  // Unwrap ONLY where the caller expects a bare array - an object-returning
  // read (metrics_probe_history carries bucket_seconds beside items) must keep
  // the wrapper, otherwise the sibling fields are silently dropped.
  if (route.unwrapItems && json && typeof json === "object" && Array.isArray((json as { items?: unknown }).items)) {
    const items = (json as { items: unknown[] }).items;
    if (route.project) {
      return items.map((it) => (it as Record<string, unknown>)[route.project as string]) as T;
    }
    return items as T;
  }
  return json as T;
}

/** Single headless dispatch point: route lookup -> typed guard -> fetch.
 *  A command with neither a CMD_TO_HTTP route nor a DISABLED_COMMANDS entry
 *  raises a typed IpcUnavailableError (reason "unknown") instead of the
 *  former opaque Tauri-only string. The mode check itself stays in
 *  lib/ipc.ts's invoke() — this seam is only ever entered in headless mode. */
export async function invokeHeadless<T = unknown>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const route = CMD_TO_HTTP[cmd];
  if (!route) {
    throw new IpcUnavailableError(cmd, DISABLED_COMMANDS[cmd] ?? "unknown");
  }
  return invokeHttp<T>(route, args);
}
