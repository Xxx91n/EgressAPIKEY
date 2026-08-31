# ADR-0055: processRoutes migrates from L1 to the L2 whitebox (field in egressapikey-ports.json)

> Status: ACCEPTED (2026-08-31, architecture-recovery Round 3 ticket 17)
> Extends (reopens none): ADR-0036 (strategy whitebox single write entry),
> ADR-0042 (whitebox port truth + restore), ADR-0051 (authoritative
> snapshot), ADR-0054 (reconciliation loop: one-way reconcile, versioned
> whitebox, metadata, acknowledged exemptions, one-shot notify).
> Supersedes (for this config family only): the pre-ticket-17 L1 persistence
> of `processRoutes` in settings.json via tauri-plugin-store.

## Context

processRoutes (per-process -> entry-port routing rules, ProcessRouteView)
was the LAST config family outside the three-layer legislation
(ARCHITECTURE.md Config Authority): it is behavior-type configuration (it
decides which port a process's traffic enters through), so it belongs to
L2, but it lived in L1 settings.json with TWO write entries:

1. webview direct write: `src/lib/settings.ts saveProcessRoutes` wired into
   the Zustand appStore (shape `{id, process, targetPort}`), and
2. Rust `process_route_add` / `process_route_remove` (shape
   `{process, target_port}`) reading and writing the SAME settings.json key
   through tauri-plugin-store.

The two writers disagreed on wire shape (camelCase `targetPort` vs
snake_case `target_port`), nothing versioned the writes, and the family was
invisible to authoritative_snapshot, one-way reconcile, drift notification
and acknowledged exemptions. ADR-0054 closed the loop for strategy and
ports; this ADR folds the last family into the SAME machinery.

## Decision

### D1. Location: a FIELD in egressapikey-ports.json, not a new file

`WhiteboxConfig` gains two optional serde fields (`serde(default)`,
`skip_serializing_if` empty - parse-compatible both ways, no version bump):

- `process_routes: Vec<ProcessRouteRule>` - `{process: String, target_port: u16}`
  (snake_case on disk and over IPC; the `ProcessRouteRule` type moves from
  `src-tauri/src/commands/platform.rs` to `resin_core::whitebox_config`).
- `route_acknowledged: Vec<String>` - the ADR-0054 section D exemption
  vocabulary for this family (process names; same shape rules via
  `validate_acknowledged`).

Why a field and not a third whitebox file: the ticket requires reusing the
EXISTING versioned backup/rotation/rollback chain (ADR-0054 section B) and
the existing validate-before-swap write entry. Both live on
`WhiteboxConfigStore` (write_atomic -> backup_before_write -> atomic swap;
list_backups/rollback_to_backup re-enter the same apply chain). A new file
would mean a third store, a third backup wiring, a third watch/apply
transaction, and a new write-entry discipline for zero behavioral gain -
the red line (no new backup/rollback mechanism) decides this: field wins.

Routes are shell-side metadata: no DB table, no Resin state file. The
whitebox file IS the truth for routes exactly as it is for ports.

### D2. Single write entry: the Rust command family through WhiteboxConfigStore

`process_route_add` / `process_route_remove` become the ONLY writers: both
mutate a `whitebox.snapshot()` and commit through
`WhiteboxConfigStore::apply` (validate -> DB/listeners -> file with
write-before-backup -> atomic swap), the same entry as port_upsert
(ADR-0042). `process_route_list` reads the snapshot. The legacy pure
conflict helper (same port, different process -> typed error) moves to
`resin_core::whitebox_config::process_route_conflict_check` and is ALSO part
of document validation, so a hand-edited file cannot smuggle two processes
onto one port. The webview direct-write link (`loadProcessRoutes` /
`saveProcessRoutes`, the appStore persist calls and the App.tsx bootstrap
read) is DELETED - the frontend goes read-through-IPC + submit-through-IPC
only; the appStore `processRoutes` array becomes a view cache fed by
`process_route_list`.

### D3. Snapshot: per-route three-state derived from the target port

Resin v1.2.0 has NO per-process / process-group API (verified against the
upstream README 2026-08-31; the shell's own history already recorded
"Resin has no such feature" - HANDOFF_PATH_A.md). The route family is
therefore NOT mirrored by any Resin object of its own. The honest L3
semantics: a route is exactly as live as the entry port it points to.

AuthoritativeSnapshot gains routes: Vec<ProcessRouteSnapshot>. The merge
(merge_routes) uses ONLY data the snapshot pass already fetches:

- the live Resin listener set (GET /api/v1/endpoints, already read for the
  ports family), and
- the enabled desired ports from the same whitebox document.

Per rule: target port has a live listener => consistent; target port is
enabled in the whitebox but has no listener => missing_on_resin (the route
cannot carry traffic - that is real drift); target port disabled or absent
from the whitebox => consistent (inert-by-intent, mirroring the ports
family's disabled-port rule). Comparison is case-insensitive on the
process name; drift-memory keys are prefixed route:<lowercase name> so a
process name can never collide with a platform name or a decimal port key.
Sidecar-down => the caller emits the route section as computed from the
whitebox side alone, per ADR-0051 absence-is-not-drift; the notify hook
stays silent on resin_reachable=false.

### D4. One-way reconcile: route convergence rides the ports half

There is no L3 route state to converge (D3), so reconcile gains NO new
action. restore_ports_from_whitebox - the ports half of reconcile_now -
re-asserts the route target ports (whitebox always wins, one-way, no
reverse write); once those listeners exist, every route reports
consistent on the next snapshot. This is the ticket's reconcile
integration by REUSE: the routes enter the exact convergence path the
ports family already has, and no new mechanism is created.

### D5. One-time boot migration (idempotent, no new mechanism)

At boot, before `WhiteboxConfigStore::open`, the shell reads the L1
`processRoutes` value if present, parses BOTH legacy wire shapes tolerantly
(`targetPort`/`target_port`/`target_lane`; invalid entries are skipped,
never fatal), MERGES the rules into the seed document (rules whose process
name already exists in the whitebox are dropped - the whitebox wins), and
DELETES the L1 key + saves. The pure helpers `parse_legacy_l1_routes` /
`migrate_l1_process_routes` live in `resin_core::whitebox_config` and are
unit-tested for idempotency (second pass is a no-op; the L1 key is gone so
re-migration cannot re-introduce rules). No settings.json backup is taken:
settings.json is L1 GUI preferences, its own zip-backup chain is out of
scope, and the migrated data lands in the versioned L2 file on the same
boot.

### D6. Acknowledged + notify

`route_acknowledged` rides the existing ADR-0054 section D/E machinery:
`stamp_route_acknowledged` marks merged output read-side only;
`count_unacknowledged_drift_entries` counts unacknowledged route drift;
the once-per-process notify state machine is unchanged.

## Consequences

- The L1/L2 dual-write form disappears: `rg "processRoutes"` in live code
  returns only the migration reader (main.rs, keyed deletion) - no other
  read, no other write, no webview persistence path.
- Both legacy wire shapes collapse into one: `{process, target_port}`.
- settings.json loses its only behavior-type key; every remaining L1 key
  is a GUI preference, matching the layer legislation.
- ipc-manifest count is unchanged (71): no command names added or removed.
- Tests baseline moves: resin-core lib 178 -> 189 (+11), shell 95 -> 96
  (+1), vitest 335 -> 341 (+6); every move is a new-behavior test, no
  existing assertion weakened.

## Verification (ticket 17)

- `cargo test -p resin-core --lib`, `cargo test -p egressapikey-app --lib`,
  `cargo test -p resin-core --test integration`: all green at the counts
  above.
- `npx vitest run` full suite green; `pnpm i18n:check` 435 keys x 18;
  `pnpm ipc:check` manifest 71 unchanged; `tsc -b` clean.
- Double-write proof: `rg -n "processRoutes" src src-tauri crates` shows
  ONLY main.rs migration reader + docs; `rg -n "saveProcessRoutes|loadProcessRoutes"`
  returns nothing under src/.
- Migration idempotency: unit tests for merge-on-boot (two passes
  identical) + the tolerant legacy parser; the L1-key deletion is the
  keyed `store.delete` path in main.rs.
- Build closed loop: chunk hash of the fresh Vite bundle greps inside the
  staged release exe; smoke launch green (MainWindowTitle, WS, stderr).
