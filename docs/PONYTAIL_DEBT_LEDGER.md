# Ponytail Debt Ledger

Generated: 2026-08-07 (2 source-tagged markers)

Standard: every `ponytail:` source marker MUST appear in this ledger so future sessions can audit + either resolve or confirm.

| File | Line | Text | Status | Rationale |
|---|---|---|---|---|
| `src-tauri/src/sidecar.rs` | 595 | do NOT add tauri-plugin-notification just for this | keep | Ghost safety net uses the existing webview event listener to draw a banner; no need to introduce a new native notification crate (volume tag: yagni). |
| `src-tauri/src/sidecar.rs` | 607 | MAX_CRASH_RESTARTS dead code | reserved | Reserved for ADR-0016 Q3 wired crash-restart handler in the SidecarHandle Terminated event. Implements the documented bound (3 retries with exponential backoff 1s/2s/4s then terminal failure). Until the handler lands, the constant is dead but documented. |

## Status legend

- **keep** = permanent stub/dead code deliberately kept; rationalized inline.
- **reserved** = forward-looking stub for a planned (ADR-numbered) upgrade; safe to leave until the upgrade lands.
**remove** when upgrade lands and the marker becomes false.
