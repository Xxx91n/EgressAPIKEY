// Ticket 14: the preview derivation is pure and snapshot-sourced — no IPC
// call anywhere in this file proves "no new requests" by construction.
import { describe, it, expect } from "vitest";
import {
  reconcilePreviewFromSnapshot,
  reconcilePreviewIsEmpty,
} from "./reconcile-preview";
import type { AuthoritativeSnapshot } from "./ipc";

const TS = 1700000000;

function baseSnap(overrides: Partial<AuthoritativeSnapshot> = {}): AuthoritativeSnapshot {
  return {
    strategyVersion: 1,
    resinReachable: true,
    lastCheckedAt: TS,
    platforms: [],
    ports: [],
    routes: [],
    subscriptions: [],
    strategyGeneration: 0,
    strategyAppliedGeneration: 0,
    convergePhase: "NeverApplied",
    ...overrides,
  };
}

describe("reconcilePreviewFromSnapshot (ticket 14)", () => {
  it("maps divergent platforms to patch_regions and missing to create_platform", () => {
    const snap = baseSnap({
      platforms: [
        {
          state: "divergent",
          platform_name: "beta",
          platform_id: "b",
          whitebox_regions: ["HK", "JP"],
          resin_regions: ["US"],
          resin_allocation_policy: "p2c",
          b_class: "BALANCED",
          a_class: "region",
          manual_nodes: [],
          subscriptions: [],
          acknowledged: false,
        },
        {
          state: "missingOnResin",
          platform_name: "gamma",
          platform_id: "",
          regions: ["HK"],
          a_class: "region",
          b_class: "BALANCED",
          manual_nodes: [],
          subscriptions: [],
          acknowledged: false,
        },
        {
          state: "consistent",
          platform_name: "alpha",
          platform_id: "a",
          regions: ["HK"],
          resin_allocation_policy: "p2c",
          b_class: "BALANCED",
          a_class: "region",
          manual_nodes: [],
          subscriptions: [],
          acknowledged: false,
        },
      ],
    });
    const plan = reconcilePreviewFromSnapshot(snap);
    expect(plan.platforms).toHaveLength(2);
    const beta = plan.platforms.find((p) => p.platform === "beta")!;
    expect(beta.action).toBe("patch_regions");
    expect(beta.desired_regions).toEqual(["HK", "JP"]);
    expect(beta.live_regions).toEqual(["US"]);
    const gamma = plan.platforms.find((p) => p.platform === "gamma")!;
    expect(gamma.action).toBe("create_platform");
    expect(gamma.live_regions).toEqual([]);
    // consistent entries never enter the plan
    expect(plan.platforms.find((p) => p.platform === "alpha")).toBeUndefined();
  });

  it("skips acknowledged (exempt) entities and lists missing enabled ports", () => {
    const snap = baseSnap({
      platforms: [
        {
          state: "divergent",
          platform_name: "known",
          platform_id: "k",
          whitebox_regions: ["HK"],
          resin_regions: ["US"],
          resin_allocation_policy: "p2c",
          b_class: "BALANCED",
          a_class: "region",
          manual_nodes: [],
          subscriptions: [],
          divergent_since: TS,
          acknowledged: true,
        },
      ],
      ports: [
        {
          state: "missingOnResin",
          port: 17990,
          platform_name: "alpha",
          protocol: "socks5",
          account: "",
          label: "",
          auth_required: false,
          acknowledged: false,
        },
        {
          state: "missingOnResin",
          port: 17991,
          platform_name: "alpha",
          protocol: "socks5",
          account: "",
          label: "",
          auth_required: false,
          acknowledged: true, // exempt port
        },
        {
          state: "consistent",
          port: 17992,
          platform_name: "alpha",
          protocol: "http",
          account: "",
          label: "",
          enabled: true,
          auth_required: false,
          acknowledged: false,
        },
      ],
    });
    const plan = reconcilePreviewFromSnapshot(snap);
    expect(plan.platforms).toHaveLength(0); // exempt platform skipped
    expect(plan.ports).toHaveLength(1);
    expect(plan.ports[0]).toEqual({ port: 17990, platform: "alpha", action: "create_endpoint" });
  });

  it("returns an empty plan while the sidecar is down (absence is not drift)", () => {
    const snap = baseSnap({
      resinReachable: false,
      platforms: [
        {
          state: "missingOnResin",
          platform_name: "delta",
          platform_id: "",
          regions: ["HK"],
          a_class: "region",
          b_class: "BALANCED",
          manual_nodes: [],
          subscriptions: [],
          acknowledged: false,
        },
      ],
      ports: [
        {
          state: "missingOnResin",
          port: 17990,
          platform_name: "delta",
          protocol: "socks5",
          account: "",
          label: "",
          auth_required: false,
          acknowledged: false,
        },
      ],
    });
    const plan = reconcilePreviewFromSnapshot(snap);
    expect(reconcilePreviewIsEmpty(plan)).toBe(true);
  });
});
