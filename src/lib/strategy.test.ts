import { describe, it, expect } from "vitest";
import {
  STRATEGY_IDS,
  normalizeStrategy,
  strategyToI18nKey,
  strategyToResinPolicy,
  mapResinToShell,
  isValidStrategyId,
  isLegacyStrategyToken,
  bClassLabel,
} from "./strategy";

describe("STRATEGY_IDS (round 8 ticket 01 / D-002)", () => {
  it("is exactly Resin's three allocation policies", () => {
    expect([...STRATEGY_IDS]).toEqual([
      "BALANCED",
      "PREFER_LOW_LATENCY",
      "PREFER_IDLE_IP",
    ]);
  });
});

describe("normalizeStrategy (the legislated six -> three table)", () => {
  it("maps the withdrawn shell options onto the real policies", () => {
    expect(normalizeStrategy("random")).toBe("BALANCED");
    expect(normalizeStrategy("bandwidth")).toBe("BALANCED");
    expect(normalizeStrategy("protocol_weight")).toBe("BALANCED");
    expect(normalizeStrategy("latency")).toBe("PREFER_LOW_LATENCY");
    expect(normalizeStrategy("sequential")).toBe("PREFER_IDLE_IP");
    expect(normalizeStrategy("quality")).toBe("PREFER_IDLE_IP");
  });

  it("round-trips the canonical values and tolerates case/whitespace", () => {
    for (const id of STRATEGY_IDS) expect(normalizeStrategy(id)).toBe(id);
    expect(normalizeStrategy(" balanced ")).toBe("BALANCED");
    expect(normalizeStrategy("prefer_idle_ip")).toBe("PREFER_IDLE_IP");
  });

  it("returns null for an unknown token instead of coercing it", () => {
    expect(normalizeStrategy("p2c")).toBeNull();
    expect(normalizeStrategy("")).toBeNull();
  });
});

describe("strategyToI18nKey", () => {
  it("maps every canonical value to its strategy.* key", () => {
    expect(strategyToI18nKey("BALANCED")).toBe("strategy.balanced");
    expect(strategyToI18nKey("PREFER_LOW_LATENCY")).toBe("strategy.preferLowLatency");
    expect(strategyToI18nKey("PREFER_IDLE_IP")).toBe("strategy.preferIdleIp");
  });

  it("resolves a withdrawn shell option to the canonical label", () => {
    expect(strategyToI18nKey("random")).toBe("strategy.balanced");
    expect(strategyToI18nKey("quality")).toBe("strategy.preferIdleIp");
  });

  it("passes unknown values through as-is", () => {
    expect(strategyToI18nKey("SOMETHING_NEW")).toBe("SOMETHING_NEW");
    expect(strategyToI18nKey("")).toBe("");
  });
});

describe("strategyToResinPolicy / mapResinToShell", () => {
  it("is the identity on the canonical set", () => {
    for (const id of STRATEGY_IDS) {
      expect(strategyToResinPolicy(id)).toBe(id);
      expect(mapResinToShell(id)).toBe(id);
    }
  });

  it("still normalizes a legacy value in flight", () => {
    expect(strategyToResinPolicy("random")).toBe("BALANCED");
    expect(strategyToResinPolicy("latency")).toBe("PREFER_LOW_LATENCY");
    expect(mapResinToShell("sequential")).toBe("PREFER_IDLE_IP");
  });

  it("falls back to BALANCED (Resin's own default) for unknown input", () => {
    expect(strategyToResinPolicy("invalid")).toBe("BALANCED");
    expect(mapResinToShell("UNKNOWN")).toBe("BALANCED");
  });
});

describe("isValidStrategyId", () => {
  it("accepts exactly the three canonical values", () => {
    for (const s of STRATEGY_IDS) expect(isValidStrategyId(s)).toBe(true);
    expect(isValidStrategyId("random")).toBe(false);
    expect(isValidStrategyId("balanced")).toBe(false);
    expect(isValidStrategyId("invalid")).toBe(false);
  });
});

describe("isLegacyStrategyToken", () => {
  it("flags the withdrawn catalog only", () => {
    for (const s of ["random", "sequential", "latency", "quality", "bandwidth", "protocol_weight"]) {
      expect(isLegacyStrategyToken(s)).toBe(true);
    }
    for (const s of ["BALANCED", "PREFER_LOW_LATENCY", "PREFER_IDLE_IP", "p2c", ""]) {
      expect(isLegacyStrategyToken(s)).toBe(false);
    }
  });
});

describe("bClassLabel (ticket 01: no parameter interpolation left)", () => {
  const t = (key: string): string =>
    ({
      "strategy.balanced": "Balanced",
      "strategy.preferLowLatency": "Prefer low latency",
      "strategy.preferIdleIp": "Prefer idle IP",
    } as Record<string, string>)[key] ?? key;

  it("renders the policy label for canonical and legacy values alike", () => {
    expect(bClassLabel("BALANCED", t)).toBe("Balanced");
    expect(bClassLabel("PREFER_LOW_LATENCY", t)).toBe("Prefer low latency");
    expect(bClassLabel("PREFER_IDLE_IP", t)).toBe("Prefer idle IP");
    expect(bClassLabel("random", t)).toBe("Balanced");
    expect(bClassLabel("quality", t)).toBe("Prefer idle IP");
  });

  it("passes an unknown value through as-is", () => {
    expect(bClassLabel("UNKNOWN_STRAT", t)).toBe("UNKNOWN_STRAT");
  });
});
