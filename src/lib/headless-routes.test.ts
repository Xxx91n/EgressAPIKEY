import { describe, it, expect } from "vitest";

/// R11-04: the cmd -> REST route table is now a standalone module
/// (boundary split out of lib/ipc.ts). These tests pin its declarative
/// invariants so a future route addition (R11-03 headless parity) cannot
/// silently break the contract the dispatch guard relies on.

import { CMD_TO_HTTP, type HttpRoute } from "./headless-routes";

const VALID_METHODS = new Set(["GET", "POST", "PATCH", "PUT", "DELETE"]);

describe("headless-routes CMD_TO_HTTP table", () => {
  it("every route targets the BFF /api/v1 prefix with a valid HTTP method", () => {
    const entries: [string, HttpRoute][] = Object.entries(CMD_TO_HTTP);
    expect(entries.length).toBeGreaterThan(0);
    for (const [cmd, route] of entries) {
      expect(VALID_METHODS.has(route.method), `${cmd} method`).toBe(true);
      expect(route.path.startsWith("/api/v1/"), `${cmd} path`).toBe(true);
    }
  });

  it("path placeholders are well-formed {argName} segments", () => {
    for (const [cmd, route] of Object.entries(CMD_TO_HTTP)) {
      const placeholders = route.path.match(/\{[^}]*\}/g) ?? [];
      for (const p of placeholders) {
        expect(/^\{[A-Za-z0-9_]+\}$/.test(p), `${cmd} placeholder ${p}`).toBe(true);
      }
    }
  });

  it("bodyArg / bodyKeys are declared on non-GET methods only", () => {
    // The fetch body is only ever built for non-GET routes; a GET route
    // declaring body fields would be dead config (and a probable mistake).
    for (const [cmd, route] of Object.entries(CMD_TO_HTTP)) {
      if (route.bodyArg !== undefined || route.bodyKeys !== undefined) {
        expect(route.method === "GET", `${cmd} declares body fields on GET`).toBe(false);
      }
    }
  });

  it("pins representative contract routes (R11-03 regression fence)", () => {
    // platform_list: bare string[] in both modes -> unwrapItems + project.
    expect(CMD_TO_HTTP.platform_list).toMatchObject({
      method: "GET", path: "/api/v1/platforms", unwrapItems: true, project: "name",
    });
    // node_list hard-codes limit=500 to mirror resin_client.rs.
    expect(CMD_TO_HTTP.node_list).toMatchObject({
      method: "GET", path: "/api/v1/nodes", queryConst: { limit: "500" },
    });
    // Path-template route: kind selects the actions/{verb} segment.
    expect(CMD_TO_HTTP.node_probe).toMatchObject({
      method: "POST", path: "/api/v1/nodes/{nodeHash}/actions/probe-{kind}",
    });
    // bodyArg unwraps the Tauri {"body":{...}} envelope.
    expect(CMD_TO_HTTP.system_config_patch).toMatchObject({
      method: "PATCH", path: "/api/v1/system/config", bodyArg: "body",
    });
  });
});
