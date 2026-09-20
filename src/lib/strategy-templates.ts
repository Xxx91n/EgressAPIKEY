/**
 * Strategy presets ("templates").
 *
 * A preset is a PREBUILT platform-strategy snapshot — not a new write path.
 * Applying one merges the preset's fields into the platform's
 * PlatformStrategy entry and commits through the SAME authoritative write
 * entry every chip uses: strategy_config_put -> strategy_apply
 * (ADR-0036/0052). The generation bump and the audit row are produced by
 * that path unchanged; a preset adds no privilege and no bypass.
 *
 * The presets are deliberately region-level honest (D-002): no preset
 * promises per-node pinning Resin does not have. `quality` presets land as
 * the top-N nodes' REGION SET; `region` with an empty list means "all
 * healthy regions" (see strategy_engine::resolve_regions).
 */
import type { PlatformStrategy } from "./ipc";

export type StrategyPresetId = "balanced" | "lowLatency" | "idleIp";

export interface StrategyPreset {
  id: StrategyPresetId;
  /** i18n key for the preset name. */
  nameKey: string;
  /** i18n key for the one-line honest description (rendered as title). */
  descKey: string;
  /** Fields merged into the platform's strategy entry on apply. */
  patch: Partial<Omit<PlatformStrategy, "platform_name">>;
}

export const STRATEGY_PRESETS: StrategyPreset[] = [
  {
    id: "balanced",
    nameKey: "strategy.presetBalanced",
    descKey: "strategy.presetBalancedDesc",
    // Region class with an empty set = all healthy regions (engine
    // semantics), under BALANCED allocation.
    patch: { a_class: "region", regions: [], b_class: "BALANCED" },
  },
  {
    id: "lowLatency",
    nameKey: "strategy.presetLowLatency",
    descKey: "strategy.presetLowLatencyDesc",
    // Top-5 fastest nodes -> their region set, under low-latency allocation.
    patch: { a_class: "quality", top_n: 5, b_class: "PREFER_LOW_LATENCY" },
  },
  {
    id: "idleIp",
    nameKey: "strategy.presetIdleIp",
    descKey: "strategy.presetIdleIpDesc",
    // Wider top-20 quality pool, idle-IP preference on the egress side.
    patch: { a_class: "quality", top_n: 20, b_class: "PREFER_IDLE_IP" },
  },
];
