//! Whitebox versioning \u2014 backup / rotation / rollback for the two L2
//! whitebox stores (architecture-recovery ticket 15; ADR-0054 \u00A7B).
//!
//! Before EVERY atomic whitebox write (both the strategy store and the ports
//! store funnel through here) the current file is copied to a sibling
//! `backup/` directory as `<original>.<unixts>.bak`.
//! Rotation keeps the newest 10 copies per whitebox file; the 11th evicts
//! the oldest. Same-second writes never collide: the second copy at the same
//! Unix second becomes `<original>.<ts>-1.bak`, `-2`, ...
//!
//! Rollback helpers re-enter the SAME validate-before-swap chain as a hand
//! edit: callers parse the backup content through serde and then call the
//! store's own write entry (ADR-0036 strategy store / ADR-0042 ports
//! apply) \u2014 this module never writes a whitebox file itself.
//!
//! Storage discipline: backup/ lives under app_config_dir() (OS AppData at
//! runtime), is runtime data and is never committed to git (no repo path is
//! involved); the whitebox editors' documents note this.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Sibling directory (inside app_config_dir) holding the whitebox backups.
pub const BACKUP_DIR_NAME: &str = "backup";
/// Rotation cap per whitebox file: the newest 10 backups are kept.
pub const WHITEBOX_BACKUP_KEEP: usize = 10;
/// Bound for collision suffix hunting (`<ts>-N.bak`).
const COLLISION_ATTEMPTS: u32 = 1000;

/// One backup copy of a whitebox file (listing entry; newest first in lists).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WhiteboxBackupEntry {
    pub file_name: String,
    pub unix_ts: u64,
    pub size_bytes: u64,
}

/// Production clock (Unix seconds). Pure helpers take `now` as a
/// parameter so tests can drive rotation/collision deterministically.
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// backup/ directory that versions `file_path`.
pub fn backup_dir(file_path: &Path) -> PathBuf {
    let parent = match file_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    parent.join(BACKUP_DIR_NAME)
}

/// Parse a backup file name into its Unix-seconds timestamp. Recognized
/// shapes: `<anything>.<ts>.bak` and `<anything>.<ts>-N.bak`
/// (same-second collision suffix). Anything else (other files in the dir,
/// junk dropped by the user) yields None and is ignored by list/rotate.
pub fn parse_backup_name(name: &str) -> Option<u64> {
    let stem = name.strip_suffix(".bak")?;
    // Collision suffix: strip a trailing -N (all digits) where what remains
    // still ends in .<digits>. rsplit_once('-') on the unsuffixed name
    // returns a non-numeric right part ("strategy.json.1756..." case) and is
    // rejected by the digit checks below.
    let ts_part = match stem.rsplit_once('-') {
        Some((left, suffix))
            if !suffix.is_empty()
                && suffix.bytes().all(|b| b.is_ascii_digit())
                && left
                    .rsplit('.')
                    .next()
                    .is_some_and(|seg| !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_digit())) =>
        {
            left
        }
        _ => stem,
    };
    let ts = ts_part.rsplit('.').next()?;
    if ts.is_empty() || !ts.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    ts.parse::<u64>().ok()
}

/// IPC-input guard (AGENTS 7.5): a rollback target must be a plain backup
/// file name that parses as a timestamped backup \u2014 no separators, no
/// traversal, no control characters. read_backup additionally requires the
/// name to exist in the backup dir listing, so traversal is doubly closed.
pub fn validate_backup_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 200 {
        return Err(format!("backup name length out of range (1..=200)"));
    }
    if !name.ends_with(".bak") {
        return Err("backup name must end with .bak".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("backup name must not contain path separators".to_string());
    }
    if name.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
        return Err("backup name contains control characters".to_string());
    }
    if parse_backup_name(name).is_none() {
        return Err("backup name is not a whitebox backup file".to_string());
    }
    Ok(())
}

/// Copy the CURRENT file to backup/ under a collision-free timestamped name,
/// then rotate. Missing source (first run) = no backup, Ok(None). Returns the
/// backup file name when a copy was made.
pub fn backup_before_write(file_path: &Path, now: u64) -> Result<Option<String>, String> {
    if !file_path.exists() {
        return Ok(None);
    }
    let dir = backup_dir(file_path);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create whitebox backup dir: {e}"))?;
    let original = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "whitebox file name is not valid UTF-8".to_string())?;
    let mut target_name = format!("{original}.{now}.bak");
    for suffix in 0..COLLISION_ATTEMPTS {
        if suffix > 0 {
            target_name = format!("{original}.{now}-{suffix}.bak");
        }
        let target = dir.join(&target_name);
        if !target.exists() {
            std::fs::copy(file_path, &target)
                .map_err(|e| format!("copy whitebox backup: {e}"))?;
            rotate_backups(file_path)?;
            return Ok(Some(target_name));
        }
    }
    Err(format!(
        "no free backup name after {COLLISION_ATTEMPTS} same-second attempts"
    ))
}

/// List the backups of ONE whitebox file, newest first. Entries whose names
/// do not parse (or belong to the other whitebox \u2014 both files share the
/// backup/ dir) are excluded.
pub fn backup_list(file_path: &Path) -> Result<Vec<WhiteboxBackupEntry>, String> {
    let dir = backup_dir(file_path);
    let original = match file_path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n.to_string(),
        None => return Ok(Vec::new()),
    };
    let prefix = format!("{original}.");
    let mut entries = Vec::new();
    let read_dir = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(entries),
        Err(e) => return Err(format!("read whitebox backup dir: {e}")),
    };
    for item in read_dir.flatten() {
        let name = item.file_name().to_string_lossy().to_string();
        if !name.starts_with(&prefix) {
            continue;
        }
        let Some(ts) = parse_backup_name(&name) else {
            continue;
        };
        let size = item
            .metadata()
            .map(|m| m.len())
            .map_err(|e| format!("stat whitebox backup: {e}"))?;
        entries.push(WhiteboxBackupEntry {
            file_name: name,
            unix_ts: ts,
            size_bytes: size,
        });
    }
    entries.sort_by(|a, b| b.unix_ts.cmp(&a.unix_ts).then(b.file_name.cmp(&a.file_name)));
    Ok(entries)
}

/// Evict the oldest backups beyond WHITEBOX_BACKUP_KEEP (per whitebox file).
fn rotate_backups(file_path: &Path) -> Result<(), String> {
    let entries = backup_list(file_path)?;
    if entries.len() <= WHITEBOX_BACKUP_KEEP {
        return Ok(());
    }
    let dir = backup_dir(file_path);
    for entry in entries.iter().skip(WHITEBOX_BACKUP_KEEP) {
        std::fs::remove_file(dir.join(&entry.file_name))
            .map_err(|e| format!("rotate whitebox backup: {e}"))?;
    }
    Ok(())
}

/// Read one backup's raw bytes by its listed file name. Unknown names are
/// rejected against the live listing, so a hostile caller cannot read
/// arbitrary paths through this helper.
pub fn read_backup(file_path: &Path, backup_name: &str) -> Result<Vec<u8>, String> {
    validate_backup_name(backup_name)?;
    if !backup_list(file_path)?
        .iter()
        .any(|e| e.file_name == backup_name)
    {
        return Err(format!("unknown whitebox backup: {backup_name}"));
    }
    std::fs::read(backup_dir(file_path).join(backup_name))
        .map_err(|e| format!("read whitebox backup: {e}"))
}

/// Read + parse one backup as a typed whitebox document. The parse is the
/// first half of validate-before-swap: the caller still routes the value
/// through its store's own write entry (validate + atomic swap + apply).
pub fn read_backup_parsed<T: serde::de::DeserializeOwned>(
    file_path: &Path,
    backup_name: &str,
) -> Result<T, String> {
    let bytes = read_backup(file_path, backup_name)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("whitebox backup content invalid: {e}"))
}

/// Single atomic write used by both whitebox stores: temp file + rename so a
/// crash never leaves a half-written whitebox document.
pub fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "config path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("create config directory: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("write config temp: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("activate config: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::whitebox_config::WhiteboxConfig;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("egressapikey-wbb-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_backup_name_extracts_ts_and_collision_suffix() {
        assert_eq!(
            parse_backup_name("egressapikey-ports.json.1756600000.bak"),
            Some(1_756_600_000)
        );
        assert_eq!(
            parse_backup_name("egressapikey-strategy.json.1756600000-3.bak"),
            Some(1_756_600_000)
        );
        // The unsuffixed strategy name contains a '-' but never a numeric
        // right part, so the collision branch must not swallow it.
        assert_eq!(
            parse_backup_name("egressapikey-strategy.json.42.bak"),
            Some(42)
        );
        assert_eq!(parse_backup_name("notes.txt"), None);
        assert_eq!(parse_backup_name("egressapikey-ports.json.bak"), None);
        assert_eq!(parse_backup_name("egressapikey-ports.json.abc.bak"), None);
    }

    #[test]
    fn backup_before_write_copies_current_content() {
        let dir = temp_dir("copy");
        let file = dir.join("egressapikey-ports.json");
        std::fs::write(&file, br#"{"version":1}"#).unwrap();
        let name = backup_before_write(&file, 1_756_600_000).unwrap().unwrap();
        assert_eq!(name, "egressapikey-ports.json.1756600000.bak");
        let copied = std::fs::read(backup_dir(&file).join(&name)).unwrap();
        assert_eq!(copied, b"{\"version\":1}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_source_file_produces_no_backup() {
        let dir = temp_dir("missing");
        let file = dir.join("egressapikey-ports.json");
        assert_eq!(backup_before_write(&file, 1).unwrap(), None);
        assert!(!backup_dir(&file).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn same_second_writes_do_not_collide() {
        let dir = temp_dir("collide");
        let file = dir.join("egressapikey-ports.json");
        std::fs::write(&file, b"first").unwrap();
        let n1 = backup_before_write(&file, 1_756_600_000).unwrap().unwrap();
        std::fs::write(&file, b"second").unwrap();
        let n2 = backup_before_write(&file, 1_756_600_000).unwrap().unwrap();
        assert_ne!(n1, n2);
        assert_eq!(n1, "egressapikey-ports.json.1756600000.bak");
        assert_eq!(n2, "egressapikey-ports.json.1756600000-1.bak");
        // Both copies survive with their own content.
        assert_eq!(
            std::fs::read(backup_dir(&file).join(&n1)).unwrap(),
            b"first"
        );
        assert_eq!(
            std::fs::read(backup_dir(&file).join(&n2)).unwrap(),
            b"second"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rotation_keeps_only_ten_newest_per_file() {
        let dir = temp_dir("rotate");
        let file = dir.join("egressapikey-ports.json");
        std::fs::write(&file, b"v").unwrap();
        for ts in 1..=12u64 {
            // Rewrite the source so each backup has distinct content; the
            // -1 suffix path also gets exercised by the repeated second.
            std::fs::write(&file, format!("v{ts}")).unwrap();
            backup_before_write(&file, ts * 1_000_000_000).unwrap().unwrap();
        }
        let listed = backup_list(&file).unwrap();
        assert_eq!(listed.len(), WHITEBOX_BACKUP_KEEP);
        assert_eq!(listed[0].unix_ts, 12_000_000_000, "newest first");
        assert_eq!(listed[WHITEBOX_BACKUP_KEEP - 1].unix_ts, 3_000_000_000);
        // Oldest two evicted from disk, not just from the listing.
        assert!(!backup_dir(&file).join("egressapikey-ports.json.1000000000.bak").exists());
        assert!(!backup_dir(&file).join("egressapikey-ports.json.2000000000.bak").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn backup_list_filters_to_requested_file_and_orders_newest_first() {
        let dir = temp_dir("filter");
        let ports = dir.join("egressapikey-ports.json");
        let strategy = dir.join("egressapikey-strategy.json");
        std::fs::write(&ports, b"p").unwrap();
        std::fs::write(&strategy, b"s").unwrap();
        backup_before_write(&ports, 100).unwrap().unwrap();
        backup_before_write(&ports, 300).unwrap().unwrap();
        backup_before_write(&strategy, 200).unwrap().unwrap();
        std::fs::write(dir.join("backup").join("junk.txt"), b"junk").unwrap();
        let listed = backup_list(&ports).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].unix_ts, 300);
        assert_eq!(listed[1].unix_ts, 100);
        assert_eq!(listed[0].size_bytes, 1);
        // Strategy list must not see ports backups or the junk file.
        let strategy_listed = backup_list(&strategy).unwrap();
        assert_eq!(strategy_listed.len(), 1);
        assert_eq!(strategy_listed[0].unix_ts, 200);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn read_backup_rejects_traversal_unknown_and_garbage_names() {
        let dir = temp_dir("read");
        let file = dir.join("egressapikey-ports.json");
        std::fs::write(&file, b"p").unwrap();
        backup_before_write(&file, 100).unwrap().unwrap();
        assert!(read_backup(&file, "../egressapikey-ports.json.100.bak").is_err());
        assert!(read_backup(&file, "strategy.json.100.bak").is_err());
        assert!(read_backup(&file, "egressapikey-ports.json.999.bak").is_err());
        assert!(read_backup(&file, "").is_err());
        // A real listed name reads back the exact bytes.
        let bytes =
            read_backup(&file, "egressapikey-ports.json.100.bak").unwrap();
        assert_eq!(bytes, b"p");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn read_backup_parsed_rejects_invalid_content() {
        let dir = temp_dir("parsed");
        let file = dir.join("egressapikey-ports.json");
        std::fs::write(&file, b"not-json").unwrap();
        backup_before_write(&file, 100).unwrap().unwrap();
        let err = read_backup_parsed::<WhiteboxConfig>(&file, "egressapikey-ports.json.100.bak")
            .unwrap_err();
        assert!(err.contains("invalid"), "parse failure must surface: {err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn validate_backup_name_enforces_ipc_bounds() {
        assert!(validate_backup_name("egressapikey-ports.json.100.bak").is_ok());
        assert!(validate_backup_name("").is_err());
        assert!(validate_backup_name(&"x".repeat(201)).is_err());
        assert!(validate_backup_name("x.json.100.bak/../../etc").is_err());
        assert!(validate_backup_name("x.json.100.bak\\win").is_err());
        assert!(validate_backup_name("x.json.100.bak..bak").is_err());
        assert!(validate_backup_name("no-timestamp.bak").is_err());
        assert!(validate_backup_name("x.json.100.secrets").is_err());
    }

    #[test]
    fn atomic_write_bytes_leaves_no_temp_and_round_trips() {
        let dir = temp_dir("atomic");
        let file = dir.join("egressapikey-ports.json");
        atomic_write_bytes(&file, b"{}").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"{}");
        assert!(!file.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
