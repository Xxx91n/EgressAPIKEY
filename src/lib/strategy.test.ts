import { describe, it, expect } from "vitest";
import { strategyToI18nKey, strategyToResinPolicy, STRATEGY_IDS, isValidStrategyId, bClassLabel } from "./strategy";

describe("strategyToI18nKey", () => {
  it("maps shell StrategyId values to strategy.* i18n keys", () => {
    expect(strategyToI18nKey("random")).toBe("strategy.random");
    expect(strategyToI18nKey("sequential")).toBe("strategy.sequential");
    expect(strategyToI18nKey("latency")).toBe("strategy.latency");
    expect(strategyToI18nKey("quality")).toBe("strategy.bQuality");
    expect(strategyToI18nKey("bandwidth")).toBe("strategy.bandwidth");
    expect(strategyToI18nKey("protocol_weight")).toBe("strategy.protocolWeight");
  });

  it("maps Resin-native values for back-compat (server returns Resin enum)", () => {
    expect(strategyToI18nKey("BALANCED")).toBe("strategy.balanced");
    expect(strategyToI18nKey("PREFER_LOW_LATENCY")).toBe("strategy.preferLowLatency");
    expect(strategyToI18nKey("PREFER_IDLE_IP")).toBe("strategy.preferIdleIp");
  });

  it("passes unknown values through as-is", () => {
    expect(strategyToI18nKey("SOMETHING_NEW")).toBe("SOMETHING_NEW");
    expect(strategyToI18nKey("")).toBe("");
  });
});

describe("strategyToResinPolicy", () => {
  it("maps random/sequential/bandwidth/protocol_weight → BALANCED", () => {
    expect(strategyToResinPolicy("random")).toBe("BALANCED");
    expect(strategyToResinPolicy("bandwidth")).toBe("BALANCED");
    expect(strategyToResinPolicy("protocol_weight")).toBe("BALANCED");
  });

  it("maps latency → PREFER_LOW_LATENCY", () => {
    expect(strategyToResinPolicy("latency")).toBe("PREFER_LOW_LATENCY");
  });

  it("maps sequential/quality → PREFER_IDLE_IP", () => {
    expect(strategyToResinPolicy("sequential")).toBe("PREFER_IDLE_IP");
    expect(strategyToResinPolicy("quality")).toBe("PREFER_IDLE_IP");
  });

  it("passes through Resin-native values unchanged", () => {
    expect(strategyToResinPolicy("BALANCED")).toBe("BALANCED");
    expect(strategyToResinPolicy("PREFER_LOW_LATENCY")).toBe("PREFER_LOW_LATENCY");
    expect(strategyToResinPolicy("PREFER_IDLE_IP")).toBe("PREFER_IDLE_IP");
  });

  it("falls back to BALANCED for unknown values", () => {
    expect(strategyToResinPolicy("invalid")).toBe("BALANCED");
  });
});

describe("isValidStrategyId", () => {
  it("accepts all 6 shell strategy values", () => {
    for (const s of STRATEGY_IDS) {
      expect(isValidStrategyId(s)).toBe(true);
    }
  });

  it("rejects Resin-native and unknown values", () => {
    expect(isValidStrategyId("BALANCED")).toBe(false);
    expect(isValidStrategyId("balanced")).toBe(false);
    expect(isValidStrategyId("invalid")).toBe(false);
  });
})

// T8-Bug3: mapResinToShell must be the inverse of strategyToResinPolicy.
// All three UI surfaces (create dialog, right-pane select, tag span) must
// show the same strategy for the same Resin allocation_policy.
import { mapResinToShell } from "./strategy";

describe("mapResinToShell (T8-Bug3)", () => {
  it("BALANCED -> random (shell StrategyId)", () => {
    expect(mapResinToShell("BALANCED")).toBe("random");
  });
  it("PREFER_LOW_LATENCY -> latency", () => {
    expect(mapResinToShell("PREFER_LOW_LATENCY")).toBe("latency");
  });
  it("PREFER_IDLE_IP -> sequential", () => {
    expect(mapResinToShell("PREFER_IDLE_IP")).toBe("sequential");
  });
  it("unknown -> random (safe default)", () => {
    expect(mapResinToShell("UNKNOWN")).toBe("random");
  });
  it("round-trip: strategyToResinPolicy(mapResinToShell(x)) === x for all Resin enums", () => {
    for (const p of ["BALANCED", "PREFER_LOW_LATENCY", "PREFER_IDLE_IP"] as const) {
      const shell = mapResinToShell(p);
      const back = strategyToResinPolicy(shell);
      expect(back).toBe(p);
    }
  });
});


// T18-3 (ADR-0042 S3): bClassLabel strategy badge with parameter interpolation.
describe("bClassLabel (T18-3)", () => {
  // Simple stub translator mirroring the en locale strategy.* keys.
  const t = (key: string, opts?: Record<string, unknown>): string => {
    const map: Record<string, string> = {
      "strategy.bParamsRandom": "Random",
      "strategy.bParamsSequential": "Sequential (N={{n}})",
      "strategy.bParamsLatency": "Latency (<{{threshold}}ms)",
      "strategy.bParamsQuality": "Quality (\u2265{{score}})",
      "strategy.bParamsBandwidth": "Bandwidth (\u00d7{{weight}})",
      "strategy.bParamsProtocolWeight": "Protocol Weight",
    };
    let out = map[key] ?? key;
    if (opts) {
      for (const [k, v] of Object.entries(opts)) {
        out = out.replace(new RegExp("{{" + k + "}}", "g"), String(v));
      }
    }
    return out;
  };

  it("renders Random for random strategy", () => {
    expect(bClassLabel("random", undefined, t)).toBe("Random");
  });

  it("renders Sequential (N=5) with round_robin_n param", () => {
    expect(bClassLabel("sequential", { round_robin_n: 5 }, t)).toBe("Sequential (N=5)");
  });

  it("renders Latency (<200ms) with latency_threshold_ms param", () => {
    expect(bClassLabel("latency", { latency_threshold_ms: 200 }, t)).toBe("Latency (<200ms)");
  });

  it("renders Quality (\u226580) with quality_score param", () => {
    expect(bClassLabel("quality", { quality_score: 80 }, t)).toBe("Quality (\u226580)");
  });

  it("renders Bandwidth (\u00d73) with bandwidth_weight param", () => {
    expect(bClassLabel("bandwidth", { bandwidth_weight: 3 }, t)).toBe("Bandwidth (\u00d73)");
  });

  it("renders Protocol Weight for protocol_weight strategy", () => {
    expect(bClassLabel("protocol_weight", undefined, t)).toBe("Protocol Weight");
  });

  it("uses default 0 when params are omitted (no crash, badge still renders)", () => {
    expect(bClassLabel("sequential", undefined, t)).toBe("Sequential (N=0)");
    expect(bClassLabel("latency", undefined, t)).toBe("Latency (<0ms)");
  });

  it("back-compat: Resin-native BALANCED maps to strategy.bParamsRandom", () => {
    expect(bClassLabel("BALANCED", undefined, t)).toBe("Random");
    expect(bClassLabel("PREFER_LOW_LATENCY", { latency_threshold_ms: 50 }, t)).toBe("Latency (<50ms)");
    expect(bClassLabel("PREFER_IDLE_IP", { quality_score: 70 }, t)).toBe("Quality (\u226570)");
  });

  it("unknown strategy value passes through as-is", () => {
    expect(bClassLabel("UNKNOWN_STRAT", undefined, t)).toBe("UNKNOWN_STRAT");
  });
});
