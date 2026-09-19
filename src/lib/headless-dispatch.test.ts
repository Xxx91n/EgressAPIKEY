import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

/// R11-04: the headless request guard + fetch dispatch is now a standalone
/// module (boundary split out of lib/ipc.ts). Covers the route-consumption
/// seam invoke() delegates to when isTauri() is false: invokeHeadless does
/// the lookup, throws the typed guard error, and the build*/invokeHttp
/// helpers own path/query/body shaping.

import {
  buildBody,
  buildPath,
  buildQuery,
  invokeHeadless,
  invokeHttp,
} from "./headless-dispatch";
import { IpcUnavailableError } from "./headless-availability";
import type { HttpRoute } from "./headless-routes";

let fetchMock: ReturnType<typeof vi.fn>;
let originalFetch: typeof fetch | undefined;

beforeEach(() => {
  originalFetch = globalThis.fetch;
  fetchMock = vi.fn();
  (globalThis as { fetch?: typeof fetch }).fetch = fetchMock as unknown as typeof fetch;
});

afterEach(() => {
  (globalThis as { fetch?: typeof fetch }).fetch = originalFetch as typeof fetch;
});

describe("headless-dispatch: buildPath", () => {
  it("fills {arg} placeholders as URL-encoded segments", () => {
    const route: HttpRoute = { method: "POST", path: "/api/v1/nodes/{nodeHash}/actions/probe-{kind}" };
    expect(buildPath(route, { nodeHash: "ab c", kind: "egress" }))
      .toBe("/api/v1/nodes/ab%20c/actions/probe-egress");
  });

  it("throws a descriptive error when a path arg is missing", () => {
    const route: HttpRoute = { method: "GET", path: "/api/v1/ports/{port}" };
    expect(() => buildPath(route, {})).toThrow(/missing path arg "port"/);
    expect(() => buildPath(route, undefined)).toThrow(/missing path arg/);
  });
});

describe("headless-dispatch: buildQuery", () => {
  it("emits queryConst params even with no args", () => {
    const route: HttpRoute = { method: "GET", path: "/api/v1/nodes", queryConst: { limit: "500" } };
    expect(buildQuery(route, undefined)).toBe("?limit=500");
  });

  it("maps declared args to their query-param names and skips absent args", () => {
    const route: HttpRoute = {
      method: "GET",
      path: "/api/v1/metrics/history/probes",
      query: { from: "from", to: "to" },
    };
    expect(buildQuery(route, { from: "F", to: "T" })).toBe("?from=F&to=T");
    expect(buildQuery(route, { from: "F" })).toBe("?from=F");
    expect(buildQuery(route, {})).toBe("");
    // Undeclared args never leak into the query string.
    expect(buildQuery(route, { from: "F", evil: "x" })).toBe("?from=F");
  });
});

describe("headless-dispatch: buildBody", () => {
  it("returns undefined for GET routes and for missing args", () => {
    const get: HttpRoute = { method: "GET", path: "/api/v1/x" };
    expect(buildBody(get, { a: 1 })).toBeUndefined();
    const post: HttpRoute = { method: "POST", path: "/api/v1/x" };
    expect(buildBody(post, undefined)).toBeUndefined();
  });

  it("serializes the Tauri args verbatim when no bodyArg/bodyKeys", () => {
    const route: HttpRoute = { method: "POST", path: "/api/v1/platforms" };
    expect(buildBody(route, { name: "openai" })).toBe('{"name":"openai"}');
  });

  it("bodyArg unwraps the Tauri arg envelope (Resin never sees {\"body\":...})", () => {
    const route: HttpRoute = { method: "PATCH", path: "/api/v1/system/config", bodyArg: "body" };
    expect(buildBody(route, { body: { a: 1 } })).toBe('{"a":1}');
    // null/undefined inner body -> no body at all
    expect(buildBody(route, { body: null })).toBeUndefined();
    // non-object inner -> typed throw, never a malformed wire body
    expect(() => buildBody(route, { body: "x" })).toThrow(/must be a JSON object/);
    expect(() => buildBody(route, { body: [1] })).toThrow(/must be a JSON object/);
  });

  it("bodyKeys renames camelCase keys and drops null/undefined fields", () => {
    const route: HttpRoute = {
      method: "PUT",
      path: "/api/v1/account-header-rules",
      bodyKeys: { urlPrefix: "url_prefix" },
    };
    expect(buildBody(route, { urlPrefix: "/api", extra: null, keep: 1 }))
      .toBe('{"url_prefix":"/api","keep":1}');
  });
});

describe("headless-dispatch: invokeHttp", () => {
  it("unwraps the Resin {items:[...]} wrapper and projects rows", async () => {
    const route: HttpRoute = {
      method: "GET", path: "/api/v1/platforms", unwrapItems: true, project: "name",
    };
    fetchMock.mockResolvedValueOnce(new Response(
      JSON.stringify({ items: [{ name: "a" }, { name: "b" }], total: 2 }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    ));
    const r = await invokeHttp<string[]>(route);
    expect(r).toEqual(["a", "b"]);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("keeps the items-wrapper for object-returning reads (no unwrapItems)", async () => {
    const route: HttpRoute = { method: "GET", path: "/api/v1/metrics/history/probes" };
    fetchMock.mockResolvedValueOnce(new Response(
      JSON.stringify({ bucket_seconds: 60, items: [{ total_count: 3 }] }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    ));
    const r = await invokeHttp<{ bucket_seconds: number; items: unknown[] }>(route);
    expect(r.bucket_seconds).toBe(60);
    expect(r.items).toHaveLength(1);
  });

  it("204 resolves undefined; !ok throws with status + body excerpt", async () => {
    const route: HttpRoute = { method: "DELETE", path: "/api/v1/platforms" };
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 204 }));
    await expect(invokeHttp(route, { name: "a" })).resolves.toBeUndefined();

    fetchMock.mockResolvedValueOnce(new Response("upstream conflict", { status: 409 }));
    await expect(invokeHttp(route, { name: "a" })).rejects.toThrow(/409.*upstream conflict/);
  });
});

describe("headless-dispatch: invokeHeadless (the request guard)", () => {
  it("a routed command fetches the declared URL with the declared method", async () => {
    fetchMock.mockResolvedValueOnce(new Response(
      JSON.stringify({ items: [{ name: "a" }] }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    ));
    const r = await invokeHeadless<string[]>("platform_list");
    expect(r).toEqual(["a"]);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("/api/v1/platforms");
    expect(init.method).toBe("GET");
  });

  it("POST sends the JSON body without a __trace_id (fetch path has none)", async () => {
    fetchMock.mockResolvedValueOnce(new Response(
      JSON.stringify({ ok: true }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    ));
    await invokeHeadless("platform_add", { name: "openai" });
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("/api/v1/platforms");
    expect(init.method).toBe("POST");
    const body = JSON.parse(init.body as string) as Record<string, unknown>;
    expect(body.name).toBe("openai");
    expect(body.__trace_id).toBeUndefined();
  });

  it("a disabled-listed command throws IpcUnavailableError with the typed reason (no fetch)", async () => {
    const err: unknown = await invokeHeadless("strategy_apply").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(IpcUnavailableError);
    const typed = err as IpcUnavailableError;
    expect(typed.command).toBe("strategy_apply");
    expect(typed.reason).toBe("shell_local_snapshot");
    expect(typed.i18nKey).toBe("ipc.disabled.shell_local_snapshot");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("a command with neither route nor registry entry throws reason \"unknown\" (no fetch)", async () => {
    const err: unknown = await invokeHeadless("totally_made_up_cmd").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(IpcUnavailableError);
    expect((err as IpcUnavailableError).reason).toBe("unknown");
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
