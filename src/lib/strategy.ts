/**
 * the shell strategy
 * vocabulary IS Resin's `allocation_policy` enum.
 *
 * The GUI used to expose six shell options that collapsed many-to-one onto
 * Resin's three real policies. That catalogue is withdrawn: the selector now
 * offers exactly the three values Resin supports, and the whitebox stores the
 * Resin wire value verbatim — so the mapping that used to live here
 * collapsed to the identity and there is nothing left to translate.
 *
 * The former six names are still ACCEPTED on read (`normalizeStrategy`) so a
 * whitebox file written before the convergence keeps rendering, but they are
 * never produced: the one-time rewrite to the canonical spelling lives in Rust
 * (`resin_core::strategy_engine::migrate_b_class_values`), and the Rust
 * `StrategyId` rejects an unknown token outright.
 *
 * Sole vocabulary owner: this file is the ONLY strategy <->
 * allocation_policy mapping left in the repo — used for view display labels
 * and the webview-side translation before the platform PATCH. The Rust side
 * holds the same three values as the storage type; do not reintroduce a second
 * mapping copy in Rust (ADR-0052 keeps the vocabularies separate and this file
 * is the display layer).
 */
export const STRATEGY_IDS = [
  "BALANCED",
  "PREFER_LOW_LATENCY",
  "PREFER_IDLE_IP",
] as const;

export type StrategyId = (typeof STRATEGY_IDS)[number];

/// Resin allocation_policy enum (the backend wire format) — identical to the
/// shell vocabulary after the convergence.
export type AllocationPolicy = StrategyId;

export const ALLOCATION_POLICIES: AllocationPolicy[] = [...STRATEGY_IDS];

/// The withdrawn six-option shell catalogue, accepted on read only.
const LEGACY_B_CLASS_TOKENS = [
  "random",
  "sequential",
  "latency",
  "quality",
  "bandwidth",
  "protocol_weight",
] as const;

/**
 * The legislated many-to-one convergence table: a canonical value
 * or one of the six withdrawn shell options -> the real policy. Mirrors
 * `resin_core::strategy::StrategyId::parse` row for row. Returns null for an
 * unrecognized token — never silently coerced.
 */
export function normalizeStrategy(s: string): AllocationPolicy | null {
  switch (s.trim().toUpperCase()) {
    case "BALANCED":
    case "RANDOM":
    case "BANDWIDTH":
    case "PROTOCOL_WEIGHT":
      return "BALANCED";
    case "PREFER_LOW_LATENCY":
    case "LATENCY":
      return "PREFER_LOW_LATENCY";
    case "PREFER_IDLE_IP":
    case "SEQUENTIAL":
    case "QUALITY":
      return "PREFER_IDLE_IP";
    default:
      return null;
  }
}

/// Map a strategy value -> i18n key (uses committed strategy.* keys). Legacy
/// tokens resolve to the canonical label: a withdrawn option no longer has a
/// label of its own.
export function strategyToI18nKey(s: string): string {
  switch (normalizeStrategy(s)) {
    case "BALANCED":
      return "strategy.balanced";
    case "PREFER_LOW_LATENCY":
      return "strategy.preferLowLatency";
    case "PREFER_IDLE_IP":
      return "strategy.preferIdleIp";
    default:
      return s;
  }
}

/// Map a shell strategy value -> Resin allocation_policy for the backend PATCH.
/// Identity after the convergence; kept as the single translation point AND as
/// the tolerant normalizer for a legacy value still in flight. An unknown value
/// falls back to BALANCED (Resin's own default).
export function strategyToResinPolicy(s: StrategyId | string): AllocationPolicy {
  return normalizeStrategy(s) ?? "BALANCED";
}

/// Map a Resin allocation_policy value back to the shell vocabulary for UI
/// display. Identity after the convergence.
export function mapResinToShell(p: string): StrategyId {
  return normalizeStrategy(p) ?? "BALANCED";
}

/// Validate that a string is a valid shell StrategyId (canonical values only).
export function isValidStrategyId(s: string): s is StrategyId {
  return (STRATEGY_IDS as readonly string[]).includes(s);
}

/// True when the value is one of the six withdrawn shell options (i.e. it still
/// needs the one-time rewrite to its canonical spelling).
export function isLegacyStrategyToken(s: string): boolean {
  return (LEGACY_B_CLASS_TOKENS as readonly string[]).includes(
    s.trim().toLowerCase(),
  );
}

/// the B-class badge label. The former
/// per-strategy parameter interpolation (round-robin N, latency threshold ms,
/// quality score, bandwidth weight) went away with `BClassParams` — those
/// were display-only values no backend ever read. The badge now states the ONE
/// thing that is real: which egress-selection policy is in effect.
export function bClassLabel(
  strategy: string,
  t: (key: string, opts?: Record<string, unknown>) => string,
): string {
  const normalized = normalizeStrategy(strategy);
  return normalized ? t(strategyToI18nKey(normalized)) : strategy;
}
