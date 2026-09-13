# ADR-0051: Authoritative Snapshot IPC — one pre-merged read-back of L2 + L3 config

Status: ACCEPTED
> Date: 2026-08-30
> Ticket: architecture-recovery 07 (authoritative-snapshot-ipc)
> Extends (reopens none): ADR-0036 (strategy whitebox single write entry),
> ADR-0039 SS2 (read-side strategy contract — ALL strategy fields travel to
> the view), ADR-0042 S2/S6 (whitebox port truth + restore). The three-layer
> authority model and the snapshot vocabulary are legislated in
> docs/architecture/ARCHITECTURE.md § Config Authority and CONTEXT.md
> (Authoritative Snapshot / Authoritative Write Entry).

## Context

Before this ADR, answering "did my change actually take effect?" required
guesswork. TopologyView.sync() fetched `platform_list_full` +
`strategy_config_get` + `port_list` separately and merged strategyConfig
over Resin platform rows IN THE VIEW (the ADR-0036 read-side shortcut that
ADR-0039 SS2 had specified, implemented at the wrong layer). Divergence
between the whitebox file and the Resin runtime was silently reconciled: the
view showed whitebox intent even when a strategy_apply PATCH had failed, and
a failed port restore was invisible unless the user noticed a missing chip.
Each view re-implemented the merge, so the merge logic could not be unit
tested at a seam and could not be reused by the future StrategyService
(ticket 10).

## Decision

1. **One deep read IPC.** New `authoritative_snapshot` Tauri command
   (read-only, no inputs, returns `Result<AuthoritativeSnapshot, IpcError>`)
   reads the three configuration sources in one call:
   - L2 strategy whitebox: `egressapikey-strategy.json` (missing file =
     `StrategyConfig::default()`, mirroring `strategy_apply`),
   - L2 ports whitebox + partner: `WhiteboxConfigStore::snapshot()` UNION
     `DbPool::list_ports()` (whitebox file is truth, ADR-0042 S2; DB rows
     absent from the whitebox still surface so partner drift stays visible),
   - L3 Resin runtime: `GET /api/v1/platforms`, `GET /api/v1/endpoints`,
     `GET /api/v1/nodes` via the ResinClient REST seam only.
   When the sidecar is not Running, the command does NOT fail: it returns
   `resinReachable: false` with empty runtime data — the whitebox half
   remains assertable while the sidecar is down.

2. **The merge lives in resin-core, once.**
   `crates/resin-core/src/snapshot.rs` owns the pure three-state merge:
   - `consistent` — whitebox and Resin runtime agree (region sets compared
     order-, duplicate- and case-insensitively);
   - `divergent` — both sides readable but disagree; BOTH values are
     carried, the snapshot never picks a winner;
   - `missingOnResin` — the platform/port exists in the whitebox (or, for
     an enabled port, its Resin endpoint vanished) but is absent on the
     other side. A disabled port with no Resin endpoint is
     consistent-by-intent (disabled means "no listener" is the desired
     state). Runtime-only platforms (created outside the strategy pipeline)
     surface as `divergent` with an empty whitebox side — never dropped,
     so "apply missing" drift is visible.
   The A-class plan fed into the comparison is computed by the same
   `compute_plan` entry `strategy_apply` uses, so `consistent` means
   "the next apply would produce exactly the runtime state". While the
   whitebox strategy file exists, runtime-only platforms render with the
   T22-4 subscription default; with no whitebox file at all there is no
   strategy intent to default to (`a_class` empty).

3. **Views consume; they never re-merge.** TopologyView.sync() now polls
   `authoritative_snapshot` + `node_list` + `lease_map` only. The
   former cfgRaw merge block (ADR-0036 read-side / ADR-0039 SS2 logic
   implemented in the view) is deleted; strategy fields arrive pre-merged in
   the snapshot (ADR-0039 SS2 contract preserved at the correct layer —
   the merge moved OUT of the view, not out of existence). SettingsView
   gains a read-only "Effective config" card (explicit Refresh button,
   no polling) that renders the per-platform/per-port state tags verbatim:
   consistent / divergent / missing-on-Resin, plus a sidecar-down notice.
   The view is forbidden from ever treating the card as editable.

4. **Wire contract.** Serde: per-variant tagged unions with
   `state` ∈ {consistent, divergent, missingOnResin} (camelCase tag to
   match the TS discriminated union); per-variant payload fields stay
   snake_case like every other IPC payload; top-level
   `AuthoritativeSnapshot` fields are camelCase. The TS wrapper
   `ipcAuthoritativeSnapshot()` takes no arguments and sanitizes the
   response as untrusted (string caps 512, array caps 64/4096, port range
   check, unknown `state` values dropped) per AGENTS.md §7.6.
   ADR-0045 error contract applies (`IpcError` + `map_resin_error`).

5. **Ownership path.** The command layer stays a thin executor; the merge
   module is deliberately placed in resin-core so ticket 10 (StrategyService)
   can adopt it by moving the strategy read/apply pipeline beside it without
   a second relocation of the merge logic.

## Consequences

- "Did my change take effect" becomes one assertable read: consumers check
  the `state` tags, not deltas between independent fetches.
- View-layer cross-store merge code is deleted (the sanctioned merge point
  is unique); the 5s TopologyView poll now issues 3 IPC calls instead of 5.
- Divergence is no longer silently reconciled: both values travel to any
  consumer that wants to show them (SettingsView surfaces the tag; canvas
  keeps rendering whitebox intent as "what the next apply would enforce").
- The snapshot is display truth, NOT a write path: every edit still flows
  through the ADR-0036 / ADR-0042 write entries. Read Retry (GET-only)
  applies to the three Resin reads inside the command.
- Runtime-only platform defaults depend on whether the whitebox strategy
  file exists on disk — a deliberate T22-4-compatible choice, unit-tested.
- Tests: 11 Rust unit tests in snapshot.rs (three legislated fixtures:
  consistent / divergent / missing, plus serde round-trip, case-insensitive
  region comparison, port merge, runtime-only default); TopologyView suite
  migrated to feed the view through the snapshot shim (71 tests, all green);
  full vitest 299 green.
