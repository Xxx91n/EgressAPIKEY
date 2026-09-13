# ADR-0070: Backup/restore unification - full authority coverage, consistent snapshots, entry-preserving restore, envelope encryption

Status: ACCEPTED (2026-09-14, architecture-recovery ticket 14)
> Extends (reopens none): ADR-0050-bis (retires its accepted torn-page risk and closes the upgrade path it declared), ADR-0061 (config export/import container - restore reuses the same parse/validate/write path), ADR-0042 S2/S6 (whitebox is truth; L3 is rebuildable), ADR-0036 + ADR-0055 (the two L2 write entries), ADR-0058 (generation bump), ADR-0059 (append-only audit chain), ADR-0069 D4 (atomic config import).
> Research basis: atomcode, three rounds - SQLite online-backup guidance (sqlite.org/backup.html: copying a live database file is an official anti-pattern; `VACUUM INTO` and the online backup API are the sanctioned paths), restic design (per-file SHA-256 manifest, verify-before-restore), the OneUptime backup-verification model (verify an artifact before applying it), OWASP password-storage guidance (PBKDF2-HMAC-SHA256 iteration counts), and the zip-encryption literature (ZipCrypto is broken; AE-2 has an attack history) - which is why the envelope in D7 is deliberately NOT zip encryption.

## Context

`backup_create` was written before the three-class Backup Scope ledger (CONTEXT.md "Backup Scope") was legislated, and it was never reconciled with it. Six facts define the gap.

- **D1 - the config layer is absent.** The package carried `settings.json` and the two L3 databases, but neither whitebox document (`egressapikey-strategy.json`, `egressapikey-ports.json`), nor `egressapikey.db`, nor `audit.jsonl`, nor the sibling `backup/` history. A backup that omits the desired state cannot restore a configuration - which makes the feature misleading rather than merely incomplete.
- **D2 - the L3 cache path is wrong.** `cache.db` was resolved against `app_config_dir()` instead of `app_data_dir()`, i.e. against a path that does not exist on a normal install: the member was silently absent from every package, and a stray same-named file in the config directory could have been packaged in its place.
- **D3 - there is no restore.** The desktop app could create (`backup_create`), upload (`backup_upload`) and list (`backup_list`) packages, but not apply one; recovery meant manual file surgery. `strategy_rollback` / `whitebox_rollback` cover single-document rollback only, not whole-config recovery.
- **D4 - L3 is read with a raw file copy.** `std::fs::read` on `state.db` / `cache.db`. ADR-0050-bis legislated this as a read-only exception AND named its torn-page risk in the same breath, closing with "a consistent hot-backup path (VACUUM INTO / backup API via a future Resin endpoint) is the known upgrade path if export fidelity ever matters."
- **D5 - no integrity verification.** No manifest and no hashes: a truncated or corrupted upload is discovered only after it has been applied.
- **D6 - the package is plaintext.** It carries the L1 WebDAV credentials and the third-party IP-reputation API keys (ipQualityScore / abuseIpDb), and the whole point of the artifact is that it is uploaded to a third-party WebDAV share.

## Decision

### D1. Scope is exactly the legislated three classes

The package covers the Backup Scope ledger and nothing else: (a) the config layer - both whitebox documents, `egressapikey.db`, `audit.jsonl` plus its rotated archives, and the sibling `backup/` history; (b) the L3 derived layer - `state.db` and `cache.db`, packaged read-only; (c) `request_logs*.db` - never, in any class. Two pure functions turn that ledger into an invariant rather than a review habit: `is_forbidden_member` rejects the leak-prone prefix, and `classify_member` must map every packaged member onto a `BackupClass` (`Config` / `L1` / `L3` / `Audit`) - an unclassified member is a hard error, so the ledger cannot drift silently when someone adds a file to the archive. `assert_no_forbidden` runs over the full member list before the archive is sealed.

### D2. The L3 paths are resolved against the data dir

`l3/resin-state/state.db` is read from `app_data_dir()/resin-state/state.db` and `l3/resin-cache/cache.db` from `app_data_dir()/resin-cache/cache.db` - the same roots the rest of the shell uses. A missing L3 file is recorded as absent rather than silently substituted with a config-dir path, so D2's failure mode cannot recur.

### D3. L3 is packaged as a consistent SQLite snapshot (supersedes ADR-0050-bis's raw-copy clause)

L3 databases go through `snapshot_db_readonly(src, dst)`: open the source `SQLITE_OPEN_READ_ONLY`, `VACUUM INTO` a temporary destination, then reopen the copy read-only and assert `PRAGMA quick_check` returns `ok` before the copy is admitted to the package. `VACUUM INTO` is the sqlite.org-sanctioned way to snapshot a running database (a raw file copy is an official anti-pattern - exactly the torn-page risk ADR-0050-bis accepted), the read-only flag keeps the shell on the read side of the L3 rule, and the copy-self-check means a corrupt snapshot is never admitted to the package. This closes the upgrade path ADR-0050-bis declared - export fidelity now matters, because restore exists.

The snapshot is the primary path, not the only one. When no consistent snapshot can be taken (for example a future Resin holding an exclusive lock), `push_sqlite_snapshot` logs a warning and packs a cold copy of the database together with its `-wal` / `-shm` siblings, so the database is still represented in the archive as evidence rather than silently disappearing from it. The degradation is visible in the log and the archive still goes through the same restore-time verification; what it must never do is present a cold copy as a consistent snapshot.

### D4. Manifest and per-member verification, all of it before any write

The archive carries `manifest.json`: `{format: "egressapikey-backup", version: 1, created_at, entries: [{path, sha256, size}]}`. Restore parses the manifest, rejects any archive member the manifest does not list, rejects any listed member that is absent, and re-verifies every member's SHA-256 - all before a single byte is written to disk. A truncated or tampered package therefore fails with zero side effects. Scope of the guarantee: SHA-256 detects corruption and tampering but is not a signature; an attacker who can rewrite the archive can rewrite the manifest. Authenticity comes from the envelope in D7 when a passphrase is set - an unencrypted package is integrity-checked, not authenticated.

### D5. Restore re-enters the authoritative write entries

The config half of a restore is parsed with ADR-0061's `parse_import_doc` + `validate_import_pair` and applied through the same two write entries `config_import` uses - `StrategyService::store` (ADR-0036) and `WhiteboxConfigStore::apply` (ADR-0055) - followed by the ports half of reconcile. Restore is `config_import` behind a different front door; there is no "swap the file on disk" path. The L1 half merges keys through the `settings.json` store one key at a time, so a restore does not silently drop keys the incoming document predates. `restore_action` encodes the mapping as data: `WriteThroughL2Entry` / `WriteThroughL1Entry` / `EvidenceOnly`.

### D6. L3 and audit are restored as evidence, never written back

Per the research verdict: L3 state is derived and rebuildable (ADR-0042 S6), so it is NOT written back - it is unpacked read-only into `restore-artifacts/` for inspection. `audit.jsonl` is an append-only chain (ADR-0059), so an incoming chain never overwrites the active one; it is unpacked as read-only evidence too. Both are reported in the restore result (`evidence[]` + `evidenceDir`) and surfaced in the UI. This is a deliberate non-goal: a restore that rewrote Resin's private state would violate L3 write authority, and one that rewound the audit chain would break its immutability.

### D7. Optional package encryption - an envelope, not zip encryption

`backup_create` accepts an optional passphrase. When present the zip is sealed with an envelope: a random 32-byte file key encrypts the package with ChaCha20-Poly1305, and the file key is wrapped by a KEK derived from the passphrase with PBKDF2-HMAC-SHA256 (600,000 iterations, 16-byte random salt); the 44-byte header carries magic `EGBKPKG1`, envelope version, KDF id, iteration count, salt and the wrapped key, and is authenticated as AEAD associated data, so a tampered header fails to open. `is_encrypted` sniffs the magic, so an unencrypted package keeps working unchanged (backward compatible with `backup_upload` / `backup_list`). Zip's built-in encryption is explicitly rejected: ZipCrypto is broken and AE-2 has an attack history. Primitives come from `ring`, already present in `Cargo.lock` transitively via rustls and promoted to a direct dependency - the feature adds zero new crates, which keeps it inside the ticket's "no new third-party dependency" constraint. Landing encryption in this round rather than a separate one is deliberate: the artifact exists to be uploaded to a third-party WebDAV share and it carries WebDAV credentials plus third-party API keys, so shipping the upload path without it would be the leak the scope ledger exists to prevent.

### D8. Bounds and boundary validation

Restore enforces a download cap (256 MiB), a total-uncompressed cap (512 MiB), a member-count cap (4096) and a history-member cap (32), zip-slip path validation (no absolute paths, no `..`, no backslash escapes), duplicate-member rejection, and a `zip_name` / URL check at the IPC edge (AGENTS.md 7.5). The caps exist so a hostile or corrupt upload fails closed instead of exhausting memory.

## Consequences

- `backup_create` is now genuinely restorable: the config layer is inside the package, and a package round-trips through the same validated write entries the GUI uses, so a restored install converges rather than merely "looks restored".
- ADR-0050-bis's exception ledger narrows without widening: the L3 read remains a legislated read-only exception, but it is no longer a raw `std::fs::read` - it is a `VACUUM INTO` snapshot with a self-check, so the torn-page risk it accepted is retired. Its file set (`state.db` / `cache.db`) is unchanged and its "widening this set needs a new ADR" clause stands.
- Restore is intentionally not a time machine. L3 and audit come back as evidence; a user who wants Resin's runtime state back rebuilds it by reconciling (ADR-0054), which is the same path as any other drift.
- Encryption is opt-in and the passphrase is unrecoverable by design (no escrow, no recovery key); the UI hint says so.
- New IPC surface: `backup_restore` (manifest 78 -> 79). No new config keys; `backup_create` gains an optional parameter, which is wire-compatible.
- Settings gains a passphrase field, a backup picker and a restore action; the `backup.restore*` i18n keys - which previously existed in all 18 locales with no backend command behind them (dangling keys) - are now live.
- Residual risks, stated plainly: (1) the manifest provides integrity, not authenticity (D4); (2) `VACUUM INTO` against a live WAL-mode database on a read-only connection was verified empirically during this ticket, but a future Resin holding an exclusive lock could still make the snapshot fail - the package then degrades to a cold copy (db + `-wal` / `-shm`) with a logged warning (D3), which is degradation rather than failure; (3) restore applies the config half through validated writes but is not transactional across L2 and L1, so a mid-way failure leaves the config half applied and reports it in `errors[]` rather than pretending to have rolled back.

## References

- SQLite: sqlite.org/backup.html (online backup API, `VACUUM INTO`), sqlite.org/lang_vacuum.html
- restic: design.rst (per-file hashing, verify-on-restore)
- OneUptime: backup verification model (2026-01-25)
- OWASP: Password Storage Cheat Sheet (PBKDF2-HMAC-SHA256 iteration guidance)
- ring: `ring::aead` (ChaCha20-Poly1305), `ring::pbkdf2`, `ring::rand`
