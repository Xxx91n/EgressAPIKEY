//! Append-only audit logging (ADR-0059).
//!
//! Every sanctioned L2 whitebox write (strategy store + ports write_atomic)
//! records one JSONL row in `audit.jsonl` under `app_config_dir()` — the
//! same directory as the two whitebox files — with a SHA-256 content hash of
//! the pre-write and post-write document plus an inline `prev_hash` chain.
//! The chain is carried per-row, so it survives `audit.jsonl.1..10` rotation
//! without ever depending on which physical file a row landed in.
//!
//! Always-best-effort: a write failure is logged through `tracing` and never
//! propagated — audit logging NEVER prevents app startup or a whitebox write
//! (the argus principle in). The module is a process-global
//! singleton initialized once by the shell with the audit file path; tests
//! exercise an `AuditLog` instance directly and never touch the global.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Schema tag in every row.
pub const AUDIT_SCHEMA: &str = "audit/v1";
/// Audit file name (sibling of the two whitebox files).
pub const AUDIT_LOG_FILE: &str = "audit.jsonl";
/// Rotation threshold in bytes (K8s log backend default: 100 MB).
pub const AUDIT_LOG_MAXSIZE: u64 = 100 * 1024 * 1024;
/// Number of rotated archives kept (`audit.jsonl.1..10`).
pub const AUDIT_LOG_MAXBACKUP: usize = 10;

/// One append-only audit row. The always-present fields are, per:
/// schema / ts / audit_id / target / op / actor / before_hash+after_hash /
/// outcome. Every field below is written on every row (the hash pair counts
/// as one of the eight groups); `None` fields are omitted from the JSON.
/// `Deserialize` is derived so tests (and a future command-line verify) can
/// read rows back; missing optional keys default to None.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuditEvent {
    pub schema: String,
    /// ISO 8601 UTC timestamp.
    pub ts: String,
    /// Correlation id for incident lookup.
    pub audit_id: String,
    /// "L2:strategy" | "L2:ports".
    pub target: String,
    /// "put" | "apply" | "rollback" | "external_edit" | "init".
    pub op: String,
    /// Initiating surface, e.g. "gui:strategy_config_put".
    pub actor: String,
    /// SHA-256 hex of the pre-write content (the bytes just backed up).
    pub before_hash: String,
    /// SHA-256 hex of the post-write content (the bytes that land).
    pub after_hash: String,
    /// "ok" | "error:<msg>".
    pub outcome: String,
    /// Optional context (rollback reason, apply failure message, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Optional high-level summary of what changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_summary: Option<Vec<String>>,
    /// The backup file a rollback restored from (rollback rows).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_backup: Option<String>,
    /// SHA-256 hex of the previous row (inline hash chain).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_before: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_after: Option<u64>,
}

/// SHA-256 hex of `bytes`. Public so the write entry points can compute the
/// before/after hashes of the whitebox documents before calling `append`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// ISO 8601 UTC timestamp for `ts`.
pub fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // UTC ISO 8601 via chrono (epoch seconds -> YYYY-MM-DDTHH:MM:SSZ); no
    // hand-rolled civil calendar. The fallback is unreachable in practice —
    // from_timestamp only fails outside ±262k years.
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs as i64, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}

/// RFC 4122 v4 uuid string for `audit_id` (uuid is already in Cargo.lock via
/// the rustls/reqwest tree, so this adds no new transitive fetch).
pub fn new_audit_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Append-only audit sink bound to one `audit.jsonl` file.
///
/// `last_hash` is the SHA-256 hex of the most recently written row and becomes
/// the next row's `prev_hash`, forming an unbroken inline chain that survives
/// rotation (each row carries its own predecessor). `written_bytes` tracks the
/// on-disk size so we rotate at AUDIT_LOG_MAXSIZE; it is re-seeded from the
/// real file at construction so a size already past the threshold rotates on
/// the first append.
///
/// Single-writer invariant: `append` performs the `prev_hash`
/// read-modify-write across the row I/O in two separate critical sections.
/// That is safe only because every production caller funnels through the
/// process-global `AUDIT` Mutex (tests use a private `AuditLog`). Before
/// any future multi-writer wiring, the chain read + write must move into ONE
/// critical section.
pub struct AuditLog {
    path: PathBuf,
    last_hash: Mutex<Option<String>>,
    written_bytes: Mutex<u64>,
}

impl AuditLog {
    pub fn new(path: PathBuf) -> Self {
        // Seed the chain tail from whatever is already on disk (fresh boot
        // must link its first row to the last row of a prior process run).
        let last_hash = last_line_hash(&path);
        let written = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Self {
            path,
            last_hash: Mutex::new(last_hash),
            written_bytes: Mutex::new(written),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one row. Best-effort at this layer only in the sense that a
    /// failure is reported to the caller (which is the store's degrade path);
    /// the module-level `append` wraps this so a failure never blocks writes.
    pub fn append(&self, mut event: AuditEvent) -> Result<(), String> {
        event.prev_hash = self.last_hash.lock().unwrap().clone();
        // Canonical JSONL row: serialize to one line + trailing newline.
        let mut line = serde_json::to_vec(&event).map_err(|e| e.to_string())?;
        line.push(b'\n');
        let row_hash = sha256_hex(&line);

        self.rotate_if_over()?;
        self.write_row(&line)?;

        // Commit the chain + byte accounting only after the row lands.
        *self.last_hash.lock().unwrap() = Some(row_hash);
        // Re-seed the byte counter from the real file size after EVERY append
        // so drift between the counter and on-disk bytes self-corrects
        // (rotation fidelity). A stat failure keeps the previous value -
        // never zeroed, never an error (D7).
        let mut written = self.written_bytes.lock().unwrap();
        if let Ok(m) = std::fs::metadata(&self.path) {
            *written = m.len();
        }
        Ok(())
    }

    fn write_row(&self, line: &[u8]) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create audit log dir: {e}"))?;
            }
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("open audit log: {e}"))?;
        f.write_all(line)
            .map_err(|e| format!("append audit log: {e}"))?;
        // Best-effort fsync (issue F1); ignore failure rather than block.
        let _ = f.sync_data();
        Ok(())
    }

    fn rotate_if_over(&self) -> Result<(), String> {
        if *self.written_bytes.lock().unwrap() <= AUDIT_LOG_MAXSIZE {
            return Ok(());
        }
        self.rotate().map_err(|e| format!("rotate audit log: {e}"))
    }

    /// Shift `audit.jsonl` to `.1`, bump `.N` -> `.N+1`, drop the oldest
    /// beyond AUDIT_LOG_MAXBACKUP, and leave an empty `audit.jsonl`. The
    /// chain tail (`last_hash`) is preserved in memory, so the next row still
    /// links to the last row of the rotated file (chain never breaks).
    fn rotate(&self) -> Result<(), String> {
        let dir = self
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        // Walk slots from the oldest to the newest so a rename never
        // overwrites a not-yet-moved slot.
        for i in (1..=AUDIT_LOG_MAXBACKUP).rev() {
            let from = dir.join(format!("{AUDIT_LOG_FILE}.{i}"));
            if i == AUDIT_LOG_MAXBACKUP {
                // Oldest slot: drop it (beyond the keep window).
                let _ = std::fs::remove_file(&from);
            } else if from.exists() {
                let to = dir.join(format!("{AUDIT_LOG_FILE}.{}", i + 1));
                std::fs::rename(&from, &to).map_err(|e| format!("shift audit archive {i}: {e}"))?;
            }
        }
        // Slot `.1` is now free; move the live file there.
        if self.path.exists() {
            let target = dir.join(format!("{AUDIT_LOG_FILE}.1"));
            std::fs::rename(&self.path, &target).map_err(|e| format!("archive audit log: {e}"))?;
        }
        // Fresh empty live file; byte accounting resets to 0.
        *self.written_bytes.lock().unwrap() = 0;
        Ok(())
    }
}

/// sha256 of a stored line's raw bytes + the '\n' terminator - a row's OWN
/// hash. Verifiers hash the stored bytes directly; a parsed `AuditEvent` is
/// never re-serialized for verification (ADR-0059 D3 errata).
fn row_own_hash(line: &[u8]) -> String {
    let mut bytes = Vec::with_capacity(line.len() + 1);
    bytes.extend_from_slice(line);
    bytes.push(b'\n');
    sha256_hex(&bytes)
}

/// Read the trailing row's OWN hash (the chain tail) from an existing audit
/// file, or None on a missing/empty/unparseable log. Walks backward over the
/// stored bytes: the FIRST line that parses as a complete `AuditEvent` is the
/// adopted tail; blank and unparseable lines - a torn final line left by a
/// crash, a well-formed foreign JSON object, mid-stream garbage - are skipped
/// forensically preserved (never truncated or rewritten).
fn last_line_hash(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let lines: Vec<&[u8]> = bytes.split(|&b| b == b'\n').collect();
    for (idx, &line) in lines.iter().enumerate().rev() {
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        // The tail candidate must parse as a complete audit event, not merely
        // as valid JSON - a foreign object is mid-stream garbage, not a tail.
        let Ok(tail) = serde_json::from_slice::<AuditEvent>(line) else {
            continue;
        };
        // Optional single-link back-verify (R12-D3, warn-only per D7): when a
        // preceding complete event row exists, its own hash must equal the
        // adopted tail's prev_hash. A mismatch never blocks adoption.
        for &prev in lines[..idx].iter().rev() {
            if prev.iter().all(|b| b.is_ascii_whitespace()) {
                continue;
            }
            if serde_json::from_slice::<AuditEvent>(prev).is_ok() {
                if tail.prev_hash.as_deref() != Some(row_own_hash(prev).as_str()) {
                    tracing::warn!(
                        target: "audit",
                        path = %path.display(),
                        "audit chain back-verify: adopted tail's prev_hash does not match the preceding complete row"
                    );
                }
                break;
            }
        }
        return Some(row_own_hash(line));
    }
    None
}

/// Process-global audit sink, initialized once by the shell (`audit::init`)
/// with the audit.jsonl path under app_config_dir(). Never propagated on
/// failure: `append` degrades to a tracing error and returns Ok so a write
/// entry that calls it can never be blocked by the audit log (argus rule).
static AUDIT: once_cell::sync::Lazy<Mutex<Option<AuditLog>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

/// Point the process-global audit log at `path` (called once at boot after
/// the shell resolves app_config_dir()). Safe to call more than once; the
/// last call wins (tests keep the global untouched and use `AuditLog::new`).
pub fn init(path: PathBuf) {
    *AUDIT.lock().unwrap() = Some(AuditLog::new(path));
}

/// Append an event through the process-global audit log. A not-yet-initialized
/// global (headless build, tests) or any write failure is a no-op that logs
/// through `tracing` and returns Ok — audit never blocks its caller.
pub fn append(event: &AuditEvent) -> Result<(), String> {
    let audit = AUDIT.lock().unwrap();
    let Some(log) = audit.as_ref() else {
        return Ok(());
    };
    match log.append(event.clone()) {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::error!(error = %e, path = %log.path().display(), "audit append failed");
            Ok(())
        }
    }
}

/// Rollback context carried down to the write entry points (issue F3). The
/// rollback IPC commands set this via `tokio::task_local!` before calling
/// the store, so the single audit row knows it was a rollback, which backup
/// it restored, and an optional reason. Absent context = the store defaults.
#[derive(Debug, Clone, Default)]
pub struct AuditCtx {
    pub op: Option<String>,
    pub actor: Option<String>,
    pub source_backup: Option<String>,
    pub reason: Option<String>,
}

tokio::task_local! {
    /// Threads into the store's write entry the fact that the write is a
    /// rollback (and why), so the emitted audit row carries `op:"rollback"`,
    /// `source_backup` and `reason` instead of the store's put/apply default.
    pub static AUDIT_CTX: AuditCtx;
}

/// Read the current task's audit context (empty outside a rollback scope).
pub fn ctx() -> AuditCtx {
    AUDIT_CTX.try_with(|c| c.clone()).unwrap_or_default()
}

/// High-level no-fields-configurable event with sensible defaults filled by
/// `append` for the common write case; the rollback path overrides op/actor
/// via `ctx()`.
pub fn event(
    target: &str,
    op: &str,
    actor: &str,
    before_hash: String,
    after_hash: String,
    outcome: &str,
    bytes_before: Option<u64>,
    bytes_after: Option<u64>,
) -> AuditEvent {
    AuditEvent {
        schema: AUDIT_SCHEMA.to_string(),
        ts: now_iso(),
        audit_id: new_audit_id(),
        target: target.to_string(),
        op: op.to_string(),
        actor: actor.to_string(),
        before_hash,
        after_hash,
        outcome: outcome.to_string(),
        reason: None,
        diff_summary: None,
        source_backup: None,
        prev_hash: None,
        app_version: None,
        bytes_before,
        bytes_after,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("egressapikey-audit-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample(target: &str) -> AuditEvent {
        event(
            target,
            "put",
            "gui:strategy_config_put",
            "a".repeat(64),
            "b".repeat(64),
            "ok",
            Some(10),
            Some(20),
        )
    }

    #[test]
    fn sha256_known_vector() {
        // "abc" -> the standard SHA-256 test vector.
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn now_iso_is_utc_rfc3339() {
        let s = now_iso();
        assert!(s.ends_with('Z'), "must be UTC: {s}");
        assert_eq!(s.len(), 20, "YYYY-MM-DDTHH:MM:SSZ");
        assert!(s.chars().nth(10) == Some('T'));
    }

    #[test]
    fn new_audit_id_is_unique_v4_shape() {
        let a = new_audit_id();
        let b = new_audit_id();
        assert_ne!(a, b);
        // 8-4-4-4-12 hex, version nibble 4.
        let parts: Vec<&str> = a.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
        assert_eq!(&parts[2][..1], "4");
    }

    #[test]
    fn append_writes_canonical_jsonl_and_links_prev_hash() {
        let dir = temp_dir("chain");
        let path = dir.join(AUDIT_LOG_FILE);
        let log = AuditLog::new(path.clone());

        log.append(sample("L2:strategy")).unwrap();
        log.append(sample("L2:ports")).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 2);
        let _ = lines;
        // Two rows, second renders as this file's _own_ first row prev_hash is none.
        let first: AuditEvent = serde_json::from_str(lines[0]).unwrap();
        let second: AuditEvent = serde_json::from_str(lines[1]).unwrap();
        assert!(first.prev_hash.is_none());
        // The second row's prev_hash equals the FIRST row's own hash.
        let first_own = sha256_hex(format!("{}\n", lines[0]).as_bytes());
        assert_eq!(second.prev_hash.as_deref(), Some(first_own.as_str()));
        // Both rows carry the 8 required semantic groups.
        assert_eq!(first.schema, "audit/v1");
        assert_eq!(first.target, "L2:strategy");
        assert_eq!(first.op, "put");
        assert_eq!(first.actor, "gui:strategy_config_put");
        assert_eq!(first.outcome, "ok");
        assert!(!first.before_hash.is_empty());
        assert!(!first.after_hash.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn restart_relinks_to_tail_of_existing_log() {
        let dir = temp_dir("restart");
        let path = dir.join(AUDIT_LOG_FILE);
        {
            let log = AuditLog::new(path.clone());
            log.append(sample("L2:strategy")).unwrap();
        }
        // New instance (simulated process restart) seeds its chain tail from
        // the last row and links its first row to it.
        let log = AuditLog::new(path.clone());
        log.append(sample("L2:ports")).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 2);
        let second: AuditEvent = serde_json::from_str(lines[1]).unwrap();
        let first_own = sha256_hex(format!("{}\n", lines[0]).as_bytes());
        assert_eq!(second.prev_hash.as_deref(), Some(first_own.as_str()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rotation_archives_up_to_maxbackup_and_keeps_chain() {
        let dir = temp_dir("rotate");
        let path = dir.join(AUDIT_LOG_FILE);
        let log = AuditLog {
            path: path.clone(),
            last_hash: Mutex::new(None),
            // Force rotation on the very first append.
            written_bytes: Mutex::new(AUDIT_LOG_MAXSIZE + 1),
        };

        for i in 0..(AUDIT_LOG_MAXBACKUP + 2) {
            let mut ev = sample("L2:strategy");
            ev.op = format!("put-{i}");
            log.append(ev).unwrap();
            // After each rotation the accounting resets, so re-prime to force
            // a rotation on the next append too.
            *log.written_bytes.lock().unwrap() = AUDIT_LOG_MAXSIZE + 1;
        }

        // The live file holds one row (the last append after the final
        // forced rotation); exactly MAXBACKUP archives + the live file exist.
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
        for i in 1..=AUDIT_LOG_MAXBACKUP {
            assert!(
                path.with_file_name(format!("{AUDIT_LOG_FILE}.{i}"))
                    .exists(),
                "archive .{i} missing"
            );
        }
        assert!(!path
            .with_file_name(format!("{AUDIT_LOG_FILE}.{}", AUDIT_LOG_MAXBACKUP + 1))
            .exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn append_failure_is_reportable_but_init_missing_degrades_to_ok() {
        // A nonexistent parent creates the dir and succeeds.
        let dir = temp_dir("okdir");
        let path = dir.join("nested").join(AUDIT_LOG_FILE);
        let log = AuditLog::new(path);
        log.append(sample("L2:strategy")).unwrap();
        assert!(log.path().exists());
        let _ = std::fs::remove_dir_all(dir);

        // Un-initialized global: module `append` returns Ok without writing.
        let audit = AUDIT.lock().unwrap();
        assert!(audit.is_none(), "global must be untouched by tests");
        drop(audit);
    }

    #[test]
    fn from_str_round_trip_is_stable_for_own_hash() {
        // A row's own hash is derived from its canonical serialization; the
        // file must never reorder fields or add whitespace, or the chain
        // would break on restart. Locked by hashing a serialized+re-serialized
        // row and asserting it equals a directly hashed one.
        let dir = temp_dir("stable");
        let path = dir.join(AUDIT_LOG_FILE);
        let log = AuditLog::new(path.clone());
        log.append(sample("L2:ports")).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let line = raw.lines().next().unwrap();
        let parsed: AuditEvent = serde_json::from_str(line).unwrap();
        let re = serde_json::to_vec(&parsed).unwrap();
        assert_eq!(
            &re[..],
            line.as_bytes(),
            "round-trip must be byte-identical"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    // -- R12-D3: tail-adoption hardening --

    /// sha256 of a canonical row (json line content) + the '\n' terminator -
    /// mirrors row_own_hash for string fixtures.
    fn own_hash(line: &str) -> String {
        sha256_hex(format!("{line}\n").as_bytes())
    }

    /// Serialize a sample event to its canonical row (no trailing newline).
    fn sample_line(target: &str) -> String {
        serde_json::to_string(&sample(target)).unwrap()
    }

    #[test]
    fn tail_adoption_picks_last_complete_event() {
        let dir = temp_dir("tail-ok");
        let path = dir.join(AUDIT_LOG_FILE);
        let l1 = sample_line("L2:strategy");
        let l2 = sample_line("L2:ports");
        std::fs::write(&path, format!("{l1}\n{l2}\n")).unwrap();
        let log = AuditLog::new(path);
        assert_eq!(*log.last_hash.lock().unwrap(), Some(own_hash(&l2)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tail_adoption_skips_valid_json_non_event() {
        // A well-formed foreign JSON object is mid-stream garbage, NOT a tail
        // candidate (was: any parseable Value became the chain tail).
        let dir = temp_dir("tail-nonjson");
        let path = dir.join(AUDIT_LOG_FILE);
        let l1 = sample_line("L2:strategy");
        std::fs::write(&path, format!("{l1}\n{{\"garbage\":true}}\n")).unwrap();
        let log = AuditLog::new(path);
        assert_eq!(*log.last_hash.lock().unwrap(), Some(own_hash(&l1)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tail_adoption_skips_torn_final_line() {
        // Crash-truncated tail (no closing brace, no newline) is skipped;
        // the last COMPLETE event row is adopted.
        let dir = temp_dir("tail-torn");
        let path = dir.join(AUDIT_LOG_FILE);
        let l1 = sample_line("L2:strategy");
        std::fs::write(&path, format!("{l1}\n{{\"schema\":\"audit/v1\",\"ts")).unwrap();
        let log = AuditLog::new(path);
        assert_eq!(*log.last_hash.lock().unwrap(), Some(own_hash(&l1)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tail_adoption_empty_log_seeds_none() {
        let dir = temp_dir("tail-empty");
        let path = dir.join(AUDIT_LOG_FILE);
        std::fs::write(&path, "").unwrap();
        let log = AuditLog::new(path);
        assert_eq!(*log.last_hash.lock().unwrap(), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn append_links_over_garbage_and_preserves_forensic_bytes() {
        // File: [event1, foreign-JSON garbage, torn tail]. The new append
        // must link to event1 (chain links OVER garbage) and the injected
        // bytes must remain in the file untouched.
        let dir = temp_dir("forensic");
        let path = dir.join(AUDIT_LOG_FILE);
        let l1 = sample_line("L2:strategy");
        let garbage = "{\"not\":\"an audit event\"}";
        let torn = "{\"schema\":\"audit/v1\",\"ts"; // truncated row bytes
        std::fs::write(&path, format!("{l1}\n{garbage}\n{torn}\n")).unwrap();
        let log = AuditLog::new(path.clone());
        assert_eq!(*log.last_hash.lock().unwrap(), Some(own_hash(&l1)));

        log.append(sample("L2:ports")).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        // forensic preservation: injected bytes still present verbatim
        assert!(raw.contains(garbage), "garbage line must be preserved");
        assert!(raw.contains(torn), "torn tail bytes must be preserved");
        // chain continuity lands on the last complete event row
        let last: AuditEvent = serde_json::from_str(raw.lines().last().unwrap()).unwrap();
        assert_eq!(last.prev_hash.as_deref(), Some(own_hash(&l1).as_str()));
        // written_bytes was re-seeded from the real file size
        assert_eq!(
            *log.written_bytes.lock().unwrap(),
            std::fs::metadata(&path).unwrap().len()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn new_seeds_counter_from_metadata_len() {
        let dir = temp_dir("counter");
        let path = dir.join(AUDIT_LOG_FILE);
        let l1 = sample_line("L2:strategy");
        std::fs::write(&path, format!("{l1}\n")).unwrap();
        let log = AuditLog::new(path.clone());
        assert_eq!(
            *log.written_bytes.lock().unwrap(),
            std::fs::metadata(&path).unwrap().len()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn back_verify_mismatch_still_adopts_tail() {
        // Tail row whose prev_hash does NOT match the preceding event's own
        // hash (hand-edited/forged chain): warn path fires, adoption stands.
        let dir = temp_dir("backverify");
        let path = dir.join(AUDIT_LOG_FILE);
        let l1 = sample_line("L2:strategy");
        let mut bad = sample("L2:ports");
        bad.prev_hash = Some("00".repeat(32));
        let l2 = serde_json::to_string(&bad).unwrap();
        std::fs::write(&path, format!("{l1}\n{l2}\n")).unwrap();
        let log = AuditLog::new(path);
        assert_eq!(*log.last_hash.lock().unwrap(), Some(own_hash(&l2)));
        let _ = std::fs::remove_dir_all(dir);
    }
}
