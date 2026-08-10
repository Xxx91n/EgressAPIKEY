import { describe, it, expect } from "vitest";
import { strategyToI18nKey, strategyToResinPolicy, STRATEGY_IDS, isValidStrategyId } from "./strategy";

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
});
