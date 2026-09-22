# ADR-0074: Platform strategy orchestration - one state machine, graded autonomy

Status: ACCEPTED
- **Date**: 2026-09-21 (r11-wave-c grill D-003; execution ticket R11-06)
- **Extends**: [ADR-0052](0052-strategy-service-module.md), ADR-0056/0057/0058 (authoritative write entry + converge phases), [ADR-0073](0073-deletion-authority-explicit-paths-only.md) (deletion authority)
- **Research basis**: wave-C grill atomcode session (Envoy outlier-ejection cooldowns, Resilience4j sliding-window breakers, K8s HPA stabilization, SRE L2→L3 gating — cross-verified at source)

## Context

Round-11's platform strategy surface can observe per-platform health
(`port_health_check` end-to-end probes, node-pool metrics) but cannot act
on it: a degraded region stays bound until a human edits the whitebox.
The grill weighed three autonomy models (manual-only / full-auto / graded)
and ruled for the graded hybrid (D-003): the SAME state machine runs under
two autonomy tiers, and the tier is a per-action execution gate, not a
second code path.

## Decision

1. **State machine**: per-platform `healthy → degraded → observing →
   (cement | regress)` with `cooldown` ejection debounce; implemented as a
   pure evaluator (`resin_core::orchestration::evaluate_section`) fed by
   per-tick verdicts. Signal windows are process-local memory only — a
   restart clears the evidence, so a stale verdict can never fire a switch
   on boot.

2. **Two tiers, one path**: `params.autonomy = auto | suggest`; absent =
   transport default (headless = auto, desktop = suggest). Under `suggest`
   every transition parks as a `pending` proposal with a human-readable
   diff and executes only through `orchestration_approve`; under `auto`
   the identical proposal executes immediately. There is no second
   transition implementation.

3. **Action surface is reversible PATCHes only**: region set changes via
   `set_platform_regions` + `apply` — the ONE authoritative write entry
   (generation bump, backup ring, audit row). Deletion, backup rollback,
   cross-platform fan-out and account-binding changes are structurally
   outside the controller's action enum.

4. **Six invariants** (all configurable via `OrchestrationParams`,
   conservative factory defaults): consecutive-failure fast trip +
   sliding-window failure-rate breach (slow calls count as failures —
   proxy risk control surfaces as challenge/timeout, not 5xx); tolerance
   band — a candidate region must beat the current set by ≥20% ok-share or
   ≥2× lower error share; observation cementing requires M consecutive
   good cycles, any regression restarts (and rolls back to the recorded
   baseline via PATCH, not backup restore); Envoy-style exponential
   cooldown backoff with cap; `min_switch_interval` per platform;
   `max_switches_per_hour` global budget; per-round `max_switch_ratio`
   avalanche guard.

5. **Config home**: optional `orchestration` section on
   `egressapikey-strategy.json` (serde default = off, zero migration). The
   section's rows are status-subresource bookkeeping — writes re-enter the
   store entry (validated, backup-ringed, audited) but never bump the
   desired-state generation, exactly the `record_subscription_phase`
   precedent.

6. **Driver**: a 60s in-shell tick on both transports (desktop Tauri task,
   headless tokio task), gated by `params.enabled` — the loop is a cheap
   no-op when off. Manual `orchestration_tick` calls the same impl.

## Consequences

- Upgrade discipline (D-003): run `suggest` first, collect audit rows,
  relax defaults only from evidence. Every transition lands in
  `audit.jsonl` via the store entry.
- Orchestration manages only region-class platforms (`a_class: Region`);
  manual/subscription/quality rows are not in its repertoire this round.
- No new data-plane behavior: signals reuse `port_health_check` probes and
  `list_nodes` metrics; the controller issues no new traffic shape.
- No third write entry: the section rides the existing strategy store.
