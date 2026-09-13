# ADR-0050-bis: backup_create's read-only exception to the L3 private-file rule

Status: ACCEPTED
> Date: 2026-09-03
> Ticket: round5-config-authority T06 (backup-create-l3-private-exception,
> mental-model fractures #2 + #8).
> Extends (reopens none): ADR-0050 (resin-core public-surface shrink — the
> shell-side-support framing this exception lives beside), ADR-0042 S2/S6
> (whitebox truth source + L3 rebuildable), ADR-0036 (whitebox single write
> entry — untouched; this ADR legislates a READ, not a write path).

## Context

The three-layer config authority legislation (ARCHITECTURE.md config
authority, AGENTS.md §Storage locations, §7.6) says shell code reaches L3
(Resin runtime state) exclusively through the ResinClient REST seam, and
"no shell code may read Resin's private `request_logs*.db` files". The
documentation named exactly one exception history: `request_log_tail` used
to read Resin's private DB directly and was retargeted to the REST seam
(architecture-recovery ticket 11).

`src-tauri/src/commands/backup.rs` `backup_create` still reads two L3
private files directly: it `std::fs::read`s `state.db` and `cache.db` from
the per-user Resin state dir and packages them into the user-facing zip
export. This predates the ADR-0042 S2 legislation and was never declared —
so the docs' exception ledger was incomplete: a code reader (or review
gate) could rightly flag `backup_create` as a "direct L3 private-file
read" violation, and the next agent could "fix" it into a broken backup.
Fractures #2 (undeclared exception) and #8 (incomplete exception
declaration) of the round-5 config-authority survey are this single gap.

Retargeting the read through the REST seam was considered and rejected:
Resin v1.2.0 exposes no endpoint that streams its own state.db/cache.db
bytes, the zip needs raw file bytes (not API projections), and the read is
one-shot, user-triggered, and side-effect-free — the seam rule exists to
stop the shell from WRITING or deriving config from L3, not to block a
user-requested export of L3 bytes.

## Decision

`backup_create`'s direct read of `state.db` / `cache.db` is a legislated,
read-only exception to the L3 execute-only rule:

1. **Scope is closed**: only `state.db` and `cache.db`. The
   `request_logs*.db` prohibition (AGENTS.md §7.6) is unchanged and
   `request_logs*.db` never enter backups — request logs can carry
   account headers and payloads; zipping them would leak secrets into a
   user-uploadable artifact.
2. **Read-only, never written**: `backup_create` opens the files for
   reading only and writes nothing back to the Resin state dir. The L3
   write authority stays exclusively the ResinClient REST seam
   (ADR-0042 S6). Backup-time consistency is guarded by the Resin
   sidecar's own SQLite locking; the shell takes no Resin-side lock.
3. **No schema reach**: the shell must never depend on the internal
   schema of these files (no parsing, no queries, no migration). If a
   future feature needs to read or — a fortiori — write Resin's private
   state meaningfully, it goes through the REST seam, and where none
   exists, an upstream Resin change is proposed first. Any widening of
   this exception's file set requires a new ADR.

Enforcement is documentary (this ADR + the ARCHITECTURE.md known-exception
paragraph + the AGENTS.md exception sentence + the CONTEXT.md "Backup
Scope" glossary entry + the pointer comment on `backup_create`); the code
is deliberately unchanged.

## Consequences

- The exception ledger is complete: every direct shell read of L3 private
  files is now either prohibited (`request_logs*.db`) or legislated
  (`state.db` / `cache.db` via `backup_create`), so reviews and agents
  have a single authoritative list to check against.
- `backup_create` behavior is unchanged; no test, wire, or IPC surface
  moves. The comment citing this ADR is the code-side anchor.
- The risk profile is accepted: a mid-write `std::fs::read` could in the
  worst case capture a torn SQLite page; the zip is a best-effort export,
  not a transactional snapshot, and Resin's own WAL/locking makes a torn
  read unlikely. A consistent hot-backup path (VACUUM INTO / backup API
  via a future Resin endpoint) is the known upgrade path if export
  fidelity ever matters.
