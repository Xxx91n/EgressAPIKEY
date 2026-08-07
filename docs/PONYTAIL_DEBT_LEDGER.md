# Ponytail Debt Ledger

Generated: 2026-08-07 (2 source-tagged markers)

Standard: every `ponytail:` source marker MUST appear in this ledger so future sessions can audit + either resolve or confirm.

| File | Line | Text | Status | Rationale |
|---|---|---|---|---|
| `src-tauri/src/sidecar.rs` | 595 | do NOT add tauri-plugin-notification just for this | keep | Ghost safety net uses the existing webview event listener to draw a banner; no need to introduce a new native notification crate (volume tag: yagni). |
| `src-tauri/src/sidecar.rs` | 665 | MAX_CRASH_RESTARTS wiring | done (commit 8961948) | ADR-0016 Q3 wiring landed in spawn_health_poll: tracks crash_count, sleeps crash_backoff_ms(i) emitting 'restarting'/'terminated' status events, transitions RunningMode to Terminated past the ceiling. Removed the dead-code marker + #[allow(dead_code)] on crash_backoff_ms. |
| `src-tauri/src/sidecar.rs` | 704 | crash restarter real respawn | deferred | Reserving a follow-up: spawn_health_poll now emits the right status events but does NOT actually call kill() + .spawn() again at the backoff boundary. Re-spawn requires extracting a regenerate path out of boot_resin (binary lookup, sidecar pickup, port conflict, child Mutex<Option<Child>> swap). Until then, the poll surfaces a 'terminated' status to the GUI so the user can restart the app manually. |

## Status legend

- **keep** = permanent stub/dead code deliberately kept; rationalized inline.
- **reserved** = forward-looking stub for a planned (ADR-numbered) upgrade; safe to leave until the upgrade lands.
**remove** when upgrade lands and the marker becomes false.
