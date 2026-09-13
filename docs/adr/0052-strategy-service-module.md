# ADR-0052: StrategyService — one deep module owns the strategyConfig pipeline (read / validate / store / apply / snapshot / deep-edit)

Status: ACCEPTED
> Date: 2026-08-30
> Ticket: architecture-recovery 10 (strategy-service-module)
> Extends (reopens none): ADR-0036 (strategy whitebox single write entry),
> ADR-0039 SS2 (read-side strategy contract), ADR-0051 (authoritative
> snapshot IPC — the read-back the Service now owns the strategy half of),
> ADR-0042 (whitebox port truth, untouched here).

## Context

Strategy logic was spread across two Rust modules and a TS helper file with
no single owner:

- `crates/resin-core/src/strategy.rs` — the B-class catalog (`StrategyId`
  shell enum, protocol-weight table).
- `crates/resin-core/src/strategy_engine.rs` — the A-class planner
  (`StrategyConfig` document model, `compute_plan`, `parse_nodes`).
- `src/lib/strategy.ts` — frontend mapping of `StrategyId` to i18n keys
  (`strategyToI18nKey`) and to Resin's 3-value `allocation_policy` enum
  (`strategyToResinPolicy`, `mapResinToShell`).

The three are one pipeline wearing three vocabularies, but the *ownership*
of the strategyConfig lifecycle was scattered in the command layer:

- `strategy_config_get` read the whitebox JSON directly.
- `strategy_config_put` duplicated validation bounds.
- `strategy_apply` did read-time writes: it auto-cleaned stale platform
  entries and wrote the cleaned JSON back **inside the command body**
  (T11-4c behavior, untestable without a Tauri runtime).
- TopologyView assembled the whole strategyConfig JSON client-side
  (read → mutate one entry → put → apply), so the view depended on the
  whitebox JSON shape and two extra round-trips.

## Decision

1. **One Service, one home.** New `resin_core::strategy_service` module
   (exported as `StrategyService`, `FsStrategyStore`,
   `StrategyConfigStore`, `clean_stale`, `validate_strategy_config`,
   `AppliedPlatform`, `ApplyReport`). It owns:
   - `get` — read the whitebox document; missing file = defaults
     (never an error), mirroring ADR-0036 semantics.
   - `validate` — the authoritative document bounds (version=1, name
     1..128, regions/subscriptions ≤64, top_n ≤1000, manual_nodes ≤256,
     duplicate platform names rejected). The IPC layer keeps its AGENTS 7.5
     input validation; the Service is the source of truth because users can
     hand-edit the JSON.
   - `store` — the only strategyConfig write path in the shell.
   - `plan` / `apply` — compute the A-class plan; `apply` PATCHes
     `region_filters` per platform through the `ResinClient` REST seam,
     reports per-platform `patched`/`reason` (serde shape identical to
     the former command-layer JSON; TS contract unchanged), and **auto-clean
     is now a pure function** (`clean_stale`) with the two fixtures the
     ticket requires: deleted-platform (dropped + persisted) and
     all-alive (no change, no write).
   - `set_platform_regions` — a deep edit (find-or-create the platform
     entry as `a_class: region`, replace its region list, validate, store)
     used by the topology canvas.
   - snapshot read side — `authoritative_snapshot` reads the whitebox
     through the Service (`get` + `store_ref().path()`); the ADR-0051
     merge in `snapshot.rs` is unchanged.
2. **Thin IPC facades.** `strategy_config_get` / `strategy_config_put` /
   `strategy_apply` shrink to construct-the-Service + delegate + map-error.
   New deep IPC `strategy_platform_regions_set` (validated at the TS
   boundary per the validate-then-invoke contract: name ≤128 no control
   chars, regions ≤64 × 1..32 chars, response shape re-checked) carries the
   canvas edit in ONE call.
3. **View discipline.** TopologyView's `patchRegionViaStrategyConfig` no
   longer reads or mutates the strategyConfig JSON shape; it calls
   `ipcStrategyPlatformRegionsSet` + `ipcStrategyApply`. Snapshot-fed
   strategy fields (ADR-0051/ADR-0039 SS2) are untouched.
4. **Vocabularies stay separate, ownership is written down** (this ADR +
   CONTEXT.md "Strategy Pipeline Vocabularies" note): the catalog
   (`strategy.rs`), the planner (`strategy_engine.rs`), and the i18n
   mapping (`src/lib/strategy.ts`) are NOT merged; the Service is their
   composition point. `StrategyId` values are serialized snake_case inside
   `egressapikey-strategy.json` (e.g. `protocol_weight`); the TS-side
   `strategyToResinPolicy` many-to-one mapping to `allocation_policy`
   happens only in the display layer and never round-trips into the
   whitebox file.

## Consequences

- The auto-clean side effect is unit-tested at the Service seam (12 tests)
  instead of being buried in an async command body.
- The command layer has zero strategy logic left to test with a Tauri
  runtime; the pure tests in `commands/tests.rs` that merely re-asserted
  JSON shapes remain as regression locks.
- `apply` takes the platform id lookup as an `fn` pointer so the async
  Service stays `Send`; the shell passes `platform_id_for_name`.
- The whitebox file gains no new fields; no migration is needed.

## Verification

- `cargo test -p resin-core --lib` 145 passed (13 in strategy_service:
  get/validate/clean_stale×2/store/set_platform_regions×3/apply-report
  serde/store_ref/fs-round-trip).
- `cargo test -p egressapikey-app --lib` 89 passed (incl. the 38 migrated
  command tests).
- `pnpm ipc:check` green with 66 commands (manifest regenerated).
- TopologyView behavior-zero-regression: the two region-edit call sites
  (`onConnect`/`onEdgesDelete`) keep the same backup-before-edit →
  regions-set → apply → sync sequence.
