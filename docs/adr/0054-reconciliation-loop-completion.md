# ADR-0054: Reconciliation-Loop Completion — one-way reconcile, preview, versioned whitebox, snapshot metadata, acknowledged exemptions, one-shot notify

Status: ACCEPTED (flipped 2026-08-31 at ticket 16 closeout, after the word-for-word alignment pass verified the sections against the landed code of tickets 12-15 and this ticket's §E implementation)
> Date: 2026-08-30
> Revised 2026-09-04 (round5 T12): §E notify semantics superseded by
> ADR-0060 — per drift episode (rising edge), not once per process.
> Ticket: architecture-recovery 12 (snapshot-metadata-acknowledged) — this
> ticket writes the OVERVIEW and sections C (metadata) / D (exemptions).
> Tickets 13-16 each land one remaining section by reference ("per
> ADR-0054 §A/§B/§E/§F"); the ADR flips to ACCEPTED at ticket 16.
> Extends (reopens none): ADR-0051 (authoritative snapshot IPC — the read
> model this ADR enriches with metadata), ADR-0036 (strategy whitebox
> single write entry), ADR-0042 (whitebox port truth + restore), ADR-0045
> (IPC error contract).

## Context

Round 2's spec identified six gaps between "the user can see whether the
config took effect" (ADR-0051 closed that) and a closed reconciliation
loop: no convergence ACTION from the view, no whitebox history/rollback,
no timestamps on drift, no known-exemption vocabulary, no notification,
and no排障 doc. A two-round atomcode research pass (OpenGitOps four
principles; ArgoCD desired/live reconcile + selfHeal default-off; Clash
Verge Rev #1715/#3395 counter-evidence for burying runtime config in
settings) placed this work as a desktop-scale desired-state reconciliation
loop, all within the existing ADR lattice.

## Decision (Overview of the six sections)

- **§A One-way reconcile (ticket 14).** A single `reconcile_now` IPC runs
  compute_plan preview -> explicit user confirm -> `strategy_apply` +
  `restore_ports_from_whitebox` -> automatic re-snapshot. Direction is
  ONE-WAY: the whitebox (L2) always wins; there is NO L3->L2 write path
  and no "accept current state" button — users who prefer the live state
  edit the whitebox instead.
- **§B Versioned whitebox (ticket 15).** Both whitebox
  stores copy the previous file to a sibling `backup/` directory (inside
  `app_config_dir()`; runtime data, never committed to git) before every
  atomic write (`<original>.<unixts>[-N].bak`; the `-N` suffix resolves
  same-second collisions so names never overwrite), rotate to keep the
  newest 10 per file, and expose `strategy_backup_list` /
  `whitebox_backup_list` (file name, unix timestamp, size; newest first)
  plus `strategy_rollback` / `whitebox_rollback` IPC. Rollback parses the
  listed backup through serde and re-enters the SAME validate-before-swap
  -> apply chain as a hand edit (strategy: Service validate + store +
  apply; ports: `WhiteboxConfigStore::apply` then
  `restore_ports_from_whitebox` for L3); it never bypasses ADR-0036 /
  ADR-0042 entries, and the rollback write is itself backed up, so a
  rollback is reversible. A backup whose content no longer parses is
  rejected before any swap. The Effective Config view lists the history
  with a per-entry rollback button; the second-confirmation dialog shows
  the target timestamp; the post-rollback re-check runs automatically (the
  next snapshot reports consistent, or carries an explicit
  non-consistent cause). Shared helpers + rotation live in
  `crates/resin-core/src/whitebox_backup.rs`.
- **§C Snapshot metadata (this ticket).** See below.
- **§D Acknowledged exemptions (this ticket).** See below.
- **§E One-shot tray notify (ticket 16).** The tray notifies exactly once
  per process when a NOT-acknowledged drift first appears; acknowledged
  entities never notify; no notification loop. Landed 2026-08-31; the
  landed mechanics are recorded in "### §E One-shot tray notify (ticket 16
  — landed here)" below.
- **§F Docs + acceptance (ticket 16).** The排障 how-to ("我改了为什么没生效")
  and the ADR flip to ACCEPTED.

### §C Snapshot metadata (ticket 12 — landed here)

1. `lastCheckedAt` — top-level `AuthoritativeSnapshot.last_checked_at`
   (wire: `lastCheckedAt`, top-level fields are camelCase per ADR-0051):
   Unix seconds when THIS snapshot was generated, stamped by the command
   layer from the wall clock. Pure metadata; it never participates in the
   three-state merge. Consecutive calls are monotonically non-decreasing.
2. `divergentSince` — per-entry optional field on the divergent and
   missingOnResin platform variants and the missingOnResin port variant
   (wire: `divergent_since`, snake_case per the ADR-0051 per-variant
   payload convention; omitted when `None`). Semantics, legislated here:
   - **In-process memory only.** The backing store is a process-local
     `DriftMemory` map (entity key -> first-drift Unix second). It is
     NEVER persisted; **a process restart clears it** and the next drift
     observation re-times from zero.
   - First observation of a drifting entity records `now`; a still-drifting
     entity KEEPS its original instant across later snapshots; an entity
     back to `consistent` has its entry removed, so a later re-drift
     re-times (there is no stale instant leak: consistent entries always
     serialize without the field).
   - Entity keys: platform name for strategy entries; decimal port number
     for port entries.
   The advance/resolve logic is a PURE function pair in
   `resin-core::snapshot` (`advance_drift_memory` /
   `divergent_since_for`) so the semantics are unit-testable without a
   Tauri runtime; the command layer owns only the static memory + the wall
   clock.
3. These fields are observability, not state: adding them changes NO merge
   outcome, and the read-side contract of ADR-0051 (views consume the
   pre-merged snapshot; three-state semantics unchanged word-for-word)
   is untouched.
4. Ticket 13 ships the read-side consumer: the one-level Effective Config
   view (nav key `effectiveConfig`, placed before diagnostics) renders the
   desired|live comparison per entity with the three-state badge, the grey
   "known" degradation for acknowledged entries, per-entry
   `divergentSince`, top-of-view `lastCheckedAt`, and a manual re-check
   button (one fetch on open, no polling). Zero write paths — the
   reconcile action landed in ticket 14 (§A); the exemption fields
   themselves landed in ticket 12 (§D).

### §D Acknowledged exemptions (ticket 12 — landed here)

1. Both whitebox documents gain an OPTIONAL `acknowledged: string[]`
   field: platform names in `egressapikey-strategy.json` (`StrategyConfig`),
   decimal port numbers in `egressapikey-ports.json` (`WhiteboxConfig`).
   Absent field = empty list (older configs load unchanged — serde
   `default`; the empty list is not serialized back —
   `skip_serializing_if`), so the change is parse-compatible both ways.
2. Validation: non-string members are a DESERIALIZATION error (the file
   fails to parse; no silent coercion, valid old files unaffected).
   `validate_acknowledged` (shared by both stores via
   `strategy_service::validate_acknowledged`) caps the list at 64 members,
   each 1..128 chars, rejecting control characters and duplicates —
   the AGENTS 7.5 bounded-array template.
3. **Exemptions never touch the three-state merge.** The merge functions
   run to completion first; `stamp_platform_acknowledged` /
   `stamp_port_acknowledged` then set the read-side `acknowledged` boolean
   on the merged OUTPUT. Drift on an acknowledged entity stays fully
   visible (state tag unchanged) — it is presented as "known" (grey badge,
   ticket 16's view) and excluded from §E notifications, but it is never
   hidden, merged away, or auto-healed.
4. TS side: the snapshot wrapper sanitizes `acknowledged` as a strict
   boolean and `divergent_since`/`lastCheckedAt` as bounded Unix-seconds
   numbers per AGENTS 7.6; the whitebox/strategy wrappers sanitize and
   validate the exemption arrays at the TS boundary (validate-then-invoke).
5. View contract (ticket 13): an acknowledged entity's badge degrades to
   the grey "known" tag and exempted rows do not surface
   `divergentSince`; the `missingOnResin` badge renders neutral zinc (not
   amber) whenever `resinReachable` is false — sidecar-down absence is
   not drift (ADR-0051).

### §E Drift-episode edge tray notify (ticket 16 — landed here; revised by ADR-0060, round5 T12)

1. The state machine is a pure pair in `src-tauri/src/tray.rs`:
   `DriftNotifyState` (process-local, `prev_has_drift: bool`, defaults to
   the no-drift baseline) + `should_fire_drift_notice(has_drift,
   prev_has_drift) -> bool` — fire only on the false→true RISING edge of
   the unacknowledged-drift predicate, i.e. **per drift episode (跃迁级，
   非进程级)**, not once per process. The static `DRIFT_NOTIFY_STATE`
   mirrors `DRIFT_MEMORY` / `RECONCILE_MEMORY`: a process restart resets to
   the no-drift baseline, so drift already present at boot notifies once
   (ArgoCD-style current-state recomputation, no cross-process memory).
   (Superseded wording, issue 16 era: "starts ARMED + re-arm on zero".)
2. **Where it hooks:** the tail of the `authoritative_snapshot` command —
   the ONLY sanctioned merge point. Every snapshot consumer (TopologyView
   5s poll, EffectiveConfigView open / manual re-check / reconcile
   re-verify / rollback re-verify) feeds the same state machine; there is
   no separate notification poller and no background loop.
3. **What counts as drift:** `count_unacknowledged_drift_entries` counts
   divergent / missingOnResin platform entries and missingOnResin port
   entries whose stamped `acknowledged` flag is false. Acknowledged entries
   are exempt here but stay fully visible in the Effective Config view (§D).
4. **Episode rule (ADR-0060 D2/D3):** the first snapshot of a drift episode
   (rising edge) fires; sustained drift is silent; the falling edge —
   drift cleared by reconcile OR absorbed by acknowledged exemptions —
   only updates the `prev_has_drift` baseline and never emits, so an
   exemption can never re-arm into a re-notice of the same drift; drift
   reappearing after a clear starts a NEW episode and notifies again.
5. **Sidecar down = silence:** when `resin_reachable` is false the hook
   returns without notifying — sidecar-down absence is not drift
   (ADR-0051), and warning the user about drift while the engine is simply
   offline would be noise. The whitebox half stays assertable in the view.
6. **Delivery:** best-effort OS notification via tauri-plugin-notification
   (`notification:default` capability; plugin registered in main.rs only
   for the GUI Builder — headless never constructs it). A dispatch failure
   is logged (tray target, warn) and swallowed: a missed toast is never
   fatal to the snapshot. Copy is per-locale via the static `drift_notice(lc)`
   table (18 rows, same lockstep contract as the tray labels; mirrors the
   `tray.driftTitle` / `tray.driftBody` keys in all 18 frontend catalogs).
   The locale is read from the persisted L1 `lang` key, which never
   influences proxy behavior — copy choice is presentation only.

### Explicitly rejected (with precedent)

- **Auto-heal / background reconcile loop:** rejected — ArgoCD ships
  `selfHeal` default-OFF because silent convergence hides the drift the
  user needs to see; our §A reconcile is explicitly user-triggered, and
  §C metadata exists precisely to make drift observable, not to feed an
  autonomous fixer.
- **L3 -> L2 reverse write ("accept current state"):** rejected — it would
  break the one-way authority of ADR-0036/ADR-0042 and turn a read-back
  into a second write entry.
- **Persistent drift memory:** rejected — a persistent `divergentSince`
  would survive restarts and misrepresent "how long has this been
  drifting" across process lifetimes the snapshot did not observe; the
  honest semantics is in-process only (documented in CONTEXT.md).

## Consequences

- The snapshot answers two more user questions without any merge change:
  "since when has this been drifting (this run)?" and "which of these did
  I already accept?".
- Both whitebox files gain one optional field each; no migration, no
  version bump (version stays 1; older files parse, newer files are
  rejected by nothing).
- Tickets 13-16 implement §A/§B/§E/§F against the interfaces this ADR
  fixes (reconcile_now name, backup naming, stamp helpers, notify-once
  rule) and may cite sections without re-legislating them.
- Tests: resin-core 156 lib tests (11 new: lastCheckedAt camelCase,
  hold/keep/clear/restart drift semantics, stamp keys, wire omit-when-none,
  acknowledged parse/validate/preserve for both stores); shell 92 lib
  tests (1 new wire-shape regression lock); vitest ipc suite 81 tests
  (7 new sanitize/validate cases).

## Verification (ticket 12 scope)

- `cargo test -p resin-core --lib` 156 passed; `cargo test -p
  egressapikey-app --lib` 92 passed; `cargo test -p resin-core --test
  integration` 3 passed.
- `npx vitest run src/lib/ipc.test.ts` 81 passed; `tsc -b` clean.
- Existing ADR-0051 eleven snapshot tests: zero regression (all still
  green, three-state semantics untouched).
