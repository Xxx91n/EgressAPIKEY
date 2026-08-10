# Ponytail Debt Ledger

Re-verified: 2026-08-09 (post-T4 audit pass). Source-tagged markers drive this ledger so future sessions can audit + either resolve or confirm. The only current `ponytail:` in-tree source tag is at sidecar.rs L655.

| File | Reference | Status | Note |
|---|---|---|---|
| `src-tauri/src/sidecar.rs` L655 | `// ponytail: do NOT add tauri-plugin-notification just for this` | keep | Ghost safety-net draws the sidecar-status banner in the React shell via `STATUS_EVENT` (apps emit "healthy"/"unhealthy"/"restarting"/"terminated"). Yagni: no native notification crate; deliberately deferred. |
| `src-tauri/src/sidecar.rs` L666 (`MAX_CRASH_RESTARTS`) + L696 (`crash_backoff_ms`) | no live `ponytail:` source tag (the dead-code marker was removed once the wiring landed) | done | ADR-0016 Q3 wiring landed in spawn_health_poll: crash_count + exponential 1s/2s/4s backoff, emits "restarting" + "terminated" status events, transitions RunningMode to Terminated past the ceiling. Tests at sidecar.rs L580-598 pin the 1s/2s/4s + MAX=3. |
| `src-tauri/src/sidecar.rs` spawn_health_poll re-spawn | no source tag | deferred | The poll emits the right status events but does NOT actually call `kill()` + `.spawn()` again at the backoff boundary to respawn. Re-spawn requires extracting a regenerate path out of boot_resin (binary lookup, sidecar pickup, port conflict, child Mutex<Option<Child>> swap). Until then the poll surfaces "terminated" so the user sees a banner to restart the app manually. |
| `crates/resin-core/src/lane.rs` `route_id`/`normalize_auth` | no source tag | deferred-dead-code | ADR-0014 authorized deletion of interceptor.rs / route_id / observed_keys but the two functions remained in lane.rs with zero callers (no import anywhere under `crates/` nor `src-tauri/` nor `src/`). Pure dead code awaiting a cleanup commit. Not blocking; not touched in the T4 audit pass because it is code deletion (not destructive-flow review). |

## Status legend
- **keep** = permanent stub/dead code deliberately kept; rationalized inline.
- **done** = previously-tagged shortcut resolved; status retained for audit trail.
- **deferred** = planned follow-up; safe to leave until the upgrade lands.
- **deferred-dead-code** = harvestable dead code; not blocking, slated for a future cleanup commit.| `crates/resin-core/src/strategy_engine.rs` L241 | `account_for_bclass` + `account_for_fixed` have tests but zero production callers | no-caller, wire-into-port_upsert-when-IpcError-refactor-done | IpcError full refactor connects B-class account tag to port_upsert |
| `src/lib/ipc.ts` L562 | `extractIpcErr` + `ipcErrI18nKey` have tests but zero GUI callers | no-gui-caller, wire-into-catch-blocks-when-IpcError-refactor-done | GUI catch blocks switch from raw string to extractIpcErr variant narrowing |

