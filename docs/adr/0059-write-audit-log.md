# ADR-0059: Append-only write audit log (audit.jsonl)

Status: ACCEPTED
- **Date**: 2026-09-04 (round5 T11 / ticket 37 §4 gap (d))

## Context

The three-layer config authority (ARCHITECTURE.md §「配置权威」) guards *what* the
current config is, but nothing recorded *who changed it, when, and from what*. The
whitebox JSON files and the strategy JSON each keep 10 rolling backups (ADR-0054 §B),
which answer "what did the file look like before" but not "which GUI action / rollback
wrote this version, and was the write accepted". Ticket 37 §4 gap (d) requires an
append-only write audit trail that is independent of the backup ring and cheap enough
to write on every L2 mutation.

## Decision

**D1 — One JSONL file, append-only.** The audit log is a single
`app_config_dir()/audit.jsonl` (`resin_core::audit::AUDIT_LOG_FILE`), one JSON object
per line, never rewritten in place. It is a log, not config: it belongs to the storage
locations ledger but to no L1/L2/L3 authority layer — deleting it never affects proxy
behavior.

**D2 — Eight required fields per row (D-31 schema).** Every row carries:
`schema` (always 1), `ts` (ISO-8601 UTC), `audit_id` (UUID v4), `target` (file path or
resource id), `op` (e.g. `put`/`apply`/`rollback`), `actor` (e.g.
`gui:strategy_config_put`), `before_hash`/`after_hash` (SHA-256 of the full written
content, hex), `outcome` (`ok`/`failed`). Optional fields: `reason`, `diff_summary`,
`source_backup` (rollback provenance), `prev_hash`, `app_version`, `bytes_before`,
`bytes_after`.

**D3 — Per-row prev_hash chain.** Each row embeds the SHA-256 of the previous row's
serialized content (first row: `null`), so the chain survives rotation (archives keep
their own tail hash; continuity is row-to-row, not file-to-file). A row's own hash is
the SHA-256 of its deterministic serde serialization + `\n`. The log seeds its chain
tail from the last line of the existing file at startup, so a process restart does not
fork the chain.

**D4 — Capacity: 100 MB per file, 10 archives.** When `audit.jsonl` exceeds 100 MB it
is renamed `audit.jsonl.1` (and `.1→.2 … .9→.10`, the oldest archive is deleted) —
K8s log-backend semantics (D-32). Combined ceiling ≈ 1.1 GB on disk, zero
administrative action required.

**D5 — Two instrumentation points, both in resin-core write entries.** The audit row
is appended where the write is *accepted*:

- `crates/resin-core/src/strategy_service.rs` `FsStrategyStore::store` (strategy JSON
  write entry; default op `put`, actor `gui:strategy_config_put`).
- `crates/resin-core/src/whitebox_config.rs` `WhiteboxConfigStore::write_atomic` (ports
  whitebox write entry; default op `apply`, actor `whitebox:apply`).

Deeper layers (SQLite sync, listener rebuild) derive from these two entries and are
NOT separately audited — one user-visible mutation = one row (no redundancy with the
backup ring, which records file *content*, while the audit records *intent + actor +
outcome*).

**D6 — Rollback hooks via task-local context.** `strategy_rollback` /
`whitebox_rollback` wrap the restore call in
`resin_core::audit::AUDIT_CTX.scope(...)`, so the audited store write inside picks up
`op = "rollback"`, the `source_backup` id, and the GUI actor, instead of being recorded
as a plain `put`/`apply`. The task-local is thread-of-await scoped: writes outside a
rollback scope keep their default op.

**D7 — Audit logging NEVER blocks or fails the app (Argus principle).** The global
audit sink is best-effort: `resin_core::audit::append` never propagates errors — a
failed write (disk full, permission denied, corrupt chain tail) degrades to a
`tracing::warn!` and the caller's write proceeds. Initialization failures at startup
are likewise non-fatal. The inverse also holds: the audit log is not a security
boundary and must not be trusted by code paths (it is user-writable state).

**D8 — Export is a copy, not a move.** Settings > Storage "Export audit log" copies
`audit.jsonl` to a user-chosen path via the native save dialog
(`export_audit_log` command; TS wrapper validates 1..4096 chars, no control chars).
The live log is never truncated by export; the log is only ever rotated by D4.

## Consequences

- Every L2 mutation (GUI edit, external file edit converge, rollback) now leaves a
  tamper-evident trace whose chain can be verified offline with sha256 alone.
- The audit module lives entirely in resin-core (`src/audit.rs`, 8 unit tests) so both
  write entries and future Rust-side writers share one implementation; the Tauri shell
  only initializes it and exposes export.
- No HTTP/IPC read surface for the log yet (beyond export) — if a GUI audit viewer is
  ever added it should paginate the file read-only, never rewrite.
- Zero interaction with the 10-backup ring (D5): backups answer content history,
  audit answers actor history; losing either does not corrupt the other.

## Errata (2026-09-23, r12 wave-d R12-D3)

Corrects and tightens D3 (amend-not-rewrite; no format change):

1. A row's own hash is the SHA-256 of the **stored line bytes** plus the
   `\n` terminator. Verifiers hash the stored bytes directly and MUST NOT
   re-serialize a parsed `AuditEvent` (serialization is deterministic for
   this struct today, but the chain contract binds the bytes on disk).
2. A torn tail (crash-truncated final line) is forensically preserved: the
   adoption walk skips it and the bytes are never truncated, rewritten, or
   removed.
3. Garbage lines mid-stream are a tolerated input: a verifier MUST skip any
   line that does not parse as an `AuditEvent`, and the chain links over it
   (the next appended row's `prev_hash` points at the last complete event
   row, not the garbage).
4. Backward-walk semantics: from EOF toward the start, the first line that
   parses as a complete `AuditEvent` is the adopted chain tail; blank and
   unparseable lines are skipped. Since this ticket the candidate must parse
   as `AuditEvent`, not merely as JSON - a well-formed foreign object is
   garbage, not a tail. The optional single-link back-verify (adopted tail's
   `prev_hash` vs the preceding complete event row's own hash) logs a
   `tracing::warn!` on mismatch but never blocks adoption (D7).
5. Disclosure: after `write_row` creates `audit.jsonl`, the parent
   directory's entry change is not fsynced - a crash could lose the file
   (never a torn file; best-effort per D7). Documented, not fixed.

Single-writer invariant (mirrored in the `audit.rs` module docs): `append`
performs the `prev_hash` read-modify-write across the row I/O in two
separate critical sections; this is safe only because every production
caller funnels through the process-global `AUDIT` Mutex. Any future
multi-writer wiring must move the chain read+write into ONE critical
section.
