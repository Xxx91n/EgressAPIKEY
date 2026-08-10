/**
 * T6-Bug2: Shell 6-option strategy as the sole UI source of truth.
 *
 * The GUI exposes 6 strategy options to the user. These map to Resin's
 * 3-value allocation_policy enum for the backend PATCH. The mapping is
 * many-to-one: multiple shell strategies collapse to the same Resin enum
 * value because Resin v1.2.0 only supports 3 allocation policies.
 *
 * strategyToI18nKey maps to the committed strategy.* i18n keys (T5 Phase 5-5).
 * strategyToResinPolicy maps to the Resin backend enum.
 */
export const STRATEGY_IDS = [
  "random",
  "sequential",
  "latency",
  "quality",
  "bandwidth",
  "protocol_weight",
] as const;

export type StrategyId = (typeof STRATEGY_IDS)[number];

/// Resin allocation_policy enum (the backend wire format).
export type AllocationPolicy = "BALANCED" | "PREFER_LOW_LATENCY" | "PREFER_IDLE_IP";

export const ALLOCATION_POLICIES: AllocationPolicy[] = [
  "BALANCED",
  "PREFER_LOW_LATENCY",
  "PREFER_IDLE_IP",
];

/// Map shell StrategyId → i18n key (uses committed strategy.* keys).
export function strategyToI18nKey(s: string): string {
  switch (s) {
    case "random": return "strategy.random";
    case "sequential": return "strategy.sequential";
    case "latency": return "strategy.latency";
    case "quality": return "strategy.bQuality";
    case "bandwidth": return "strategy.bandwidth";
    case "protocol_weight": return "strategy.protocolWeight";
    // Back-compat: Resin-native values from older code paths
    case "BALANCED": return "strategy.balanced";
    case "PREFER_LOW_LATENCY": return "strategy.preferLowLatency";
    case "PREFER_IDLE_IP": return "strategy.preferIdleIp";
    default: return s;
  }
}

/// Map shell StrategyId → Resin allocation_policy enum for backend PATCH.
export function strategyToResinPolicy(s: StrategyId | string): AllocationPolicy {
  switch (s) {
    case "random":
    case "bandwidth":
    case "protocol_weight":
    case "BALANCED":
      return "BALANCED";
    case "latency":
    case "PREFER_LOW_LATENCY":
      return "PREFER_LOW_LATENCY";
    case "sequential":
    case "quality":
    case "PREFER_IDLE_IP":
      return "PREFER_IDLE_IP";
    default:
      return "BALANCED";
  }
}

/// Validate that a string is a valid shell StrategyId.
export function isValidStrategyId(s: string): s is StrategyId {
  return (STRATEGY_IDS as readonly string[]).includes(s);
}
