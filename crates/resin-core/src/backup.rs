//! Backup package model (ADR-0070).
//!
//! Pure logic for the user-facing backup archive. It owns four things and
//! nothing else:
//!
//! 1. **The member whitelist.** CONTEXT.md "Backup Scope" legislates a
//!    three-class ledger (config layer / L3-derived / never-in-a-backup).
//!    `classify_member` turns a package path into that class and
//!    `restore_action` turns the class into what restore is allowed to do
//!    with it. The request-log leak-prevention line is `is_forbidden_member`.
//! 2. **The per-file SHA-256 manifest.** Generated at backup time, re-verified
//!    before restore writes anything, so "this backup is restorable" is an
//!    assertable claim rather than a hope.
//! 3. **The optional passphrase envelope.** A random 256-bit file key wraps
//!    the package; the file key itself is wrapped by a PBKDF2-HMAC-SHA256 key
//!    derived from the user passphrase. AEAD is ChaCha20-Poly1305.
//! 4. **The restore disposition policy.** Which classes are written through
//!    an authoritative write entry, and which are evidence-only.
//!
//! This module never touches the filesystem, the zip container, Tauri, or
//! Resin: callers hand it member bytes and paths, so every rule here is
//! unit-testable without a runtime.

use std::num::NonZeroU32;

use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::pbkdf2;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};

use crate::audit::sha256_hex;

// ---------------------------------------------------------------------------
// Package layout
// ---------------------------------------------------------------------------

/// Container tag in the manifest.
pub const BACKUP_FORMAT: &str = "egressapikey-backup";
/// Container format version. Bumped when the member layout changes shape.
pub const BACKUP_FORMAT_VERSION: u8 = 1;

/// Manifest member (itself excluded from its own entry list).
pub const MANIFEST_ENTRY: &str = "manifest.json";
/// The ADR-0061 config-transfer container, built from the two whitebox
/// documents. Sharing that container is what makes backup and
/// `config_export` ONE model instead of two.
pub const CONFIG_ENTRY: &str = "config.json";
/// L1 GUI preferences (L1 authoritative write entry: tauri-plugin-store).
pub const SETTINGS_ENTRY: &str = "settings.json";
/// L2 strategy whitebox (ADR-0036 write entry).
pub const STRATEGY_ENTRY: &str = "l2/egressapikey-strategy.json";
/// L2 entry-port whitebox (ADR-0042 write entry).
pub const PORTS_ENTRY: &str = "l2/egressapikey-ports.json";
/// L2 port-mapping SQLite store (derived from the ports whitebox on apply).
pub const PORT_DB_ENTRY: &str = "l2/egressapikey.db";
/// L2 append-only write-audit log (ADR-0059).
pub const AUDIT_ENTRY: &str = "l2/audit.jsonl";
/// Directory prefix holding the rotated audit archives (audit.jsonl.1..10).
pub const AUDIT_ARCHIVE_PREFIX: &str = "l2/audit.jsonl.";
/// Directory holding the whitebox version history (ADR-0054 section B).
pub const WHITEBOX_HISTORY_DIR: &str = "l2/backup";
/// L3 Resin strong-persistence store (read-only in, never written back).
pub const STATE_DB_ENTRY: &str = "l3/resin-state/state.db";
/// L3 Resin weak-persistence store (read-only in, never written back).
pub const CACHE_DB_ENTRY: &str = "l3/resin-cache/cache.db";
/// L3 root prefix.
pub const L3_PREFIX: &str = "l3/";
/// L2 root prefix.
pub const L2_PREFIX: &str = "l2/";
/// Resin request-log DBs are named request_logs-<unix_ms>.db and live under
/// the Resin LOG dir. They never enter a backup, in any class (leak line).
pub const REQUEST_LOG_PREFIX: &str = "request_logs";

// ---------------------------------------------------------------------------
// Member classification
// ---------------------------------------------------------------------------

/// The three-class ledger of CONTEXT.md "Backup Scope".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupClass {
    /// Configuration layer: the whitebox documents, their version history,
    /// the port-mapping DB, and the shared ADR-0061 config container.
    Config,
    /// L1 GUI preferences.
    L1,
    /// L3-derived Resin runtime state, packaged read-only.
    L3,
    /// Append-only audit evidence.
    Audit,
}

/// What restore may do with a member of a given class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreAction {
    /// Re-enter the L2 authoritative write entry (validate before swap).
    WriteThroughL2Entry,
    /// Re-enter the L1 authoritative write entry (the settings store).
    WriteThroughL1Entry,
    /// Extract as read-only evidence; NEVER written back.
    EvidenceOnly,
}

/// True when a package member must never be produced or accepted: the Resin
/// request-log DBs (CONTEXT.md "Backup Scope" class c). Matched on the file
/// name so any directory form is caught.
pub fn is_forbidden_member(path: &str) -> bool {
    let base = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    base.starts_with(REQUEST_LOG_PREFIX)
}

/// Classify one package member. `None` means the member is not part of any
/// legislated class and must be rejected by the manifest gate.
pub fn classify_member(path: &str) -> Option<BackupClass> {
    if path == MANIFEST_ENTRY || is_forbidden_member(path) {
        return None;
    }
    if path == SETTINGS_ENTRY {
        return Some(BackupClass::L1);
    }
    if path == AUDIT_ENTRY || path.starts_with(AUDIT_ARCHIVE_PREFIX) {
        return Some(BackupClass::Audit);
    }
    if path == CONFIG_ENTRY || path.starts_with(L2_PREFIX) {
        return Some(BackupClass::Config);
    }
    if path.starts_with(L3_PREFIX) {
        return Some(BackupClass::L3);
    }
    None
}

/// The restore disposition for a class. Encodes the researched rule: only the
/// authoritative expectation state is written back (through its own write
/// entry); derived runtime state is left alone; append-only evidence is
/// extracted, never replayed over the live chain.
pub fn restore_action(class: BackupClass) -> RestoreAction {
    match class {
        BackupClass::Config => RestoreAction::WriteThroughL2Entry,
        BackupClass::L1 => RestoreAction::WriteThroughL1Entry,
        BackupClass::L3 | BackupClass::Audit => RestoreAction::EvidenceOnly,
    }
}

/// Reject a member list containing any forbidden member.
pub fn assert_no_forbidden(paths: &[String]) -> Result<(), String> {
    for p in paths {
        if is_forbidden_member(p) {
            return Err(format!("backup member rejected (request-log leak line): {p}"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

/// One package member as recorded in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Member path inside the archive (forward slashes).
    pub path: String,
    /// Lowercase hex SHA-256 of the member bytes.
    pub sha256: String,
    /// Member byte length.
    pub size: u64,
}

/// The package manifest: what the archive claims to contain, and the hashes
/// that make the claim checkable before anything is written back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupManifest {
    pub format: String,
    pub version: u8,
    pub created_at: String,
    pub entries: Vec<ManifestEntry>,
}

/// Build one manifest entry from member bytes.
pub fn manifest_entry(path: &str, bytes: &[u8]) -> ManifestEntry {
    ManifestEntry {
        path: path.to_string(),
        sha256: sha256_hex(bytes),
        size: bytes.len() as u64,
    }
}

/// Assemble a manifest. Entries are sorted by path so the document is
/// byte-deterministic for a given member set. Rejects forbidden members and
/// duplicate paths.
pub fn build_manifest(
    created_at: &str,
    mut entries: Vec<ManifestEntry>,
) -> Result<BackupManifest, String> {
    let paths: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
    assert_no_forbidden(&paths)?;
    for e in &entries {
        if classify_member(&e.path).is_none() {
            return Err(format!("backup member is not part of any legislated class: {}", e.path));
        }
        if !is_sha256_hex(&e.sha256) {
            return Err(format!("manifest entry has a malformed sha256: {}", e.path));
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    for w in entries.windows(2) {
        if w[0].path == w[1].path {
            return Err(format!("duplicate manifest entry: {}", w[0].path));
        }
    }
    Ok(BackupManifest {
        format: BACKUP_FORMAT.to_string(),
        version: BACKUP_FORMAT_VERSION,
        created_at: created_at.to_string(),
        entries,
    })
}

/// Serialize a manifest as pretty JSON with a trailing newline.
pub fn manifest_json(m: &BackupManifest) -> Result<Vec<u8>, String> {
    let mut s = serde_json::to_string_pretty(m).map_err(|e| format!("manifest serialize: {e}"))?;
    s.push('\n');
    Ok(s.into_bytes())
}

/// Parse and validate a manifest. Every gate that guards a restore lives here:
/// container tag, format version, forbidden members, class membership, hash
/// shape, duplicate paths.
pub fn parse_manifest(bytes: &[u8]) -> Result<BackupManifest, String> {
    let m: BackupManifest =
        serde_json::from_slice(bytes).map_err(|e| format!("manifest parse: {e}"))?;
    if m.format != BACKUP_FORMAT {
        return Err(format!("unsupported backup format: {}", m.format));
    }
    if m.version > BACKUP_FORMAT_VERSION {
        return Err(format!(
            "unsupported backup format version {} (this build understands up to {})",
            m.version, BACKUP_FORMAT_VERSION
        ));
    }
    let paths: Vec<String> = m.entries.iter().map(|e| e.path.clone()).collect();
    assert_no_forbidden(&paths)?;
    for e in &m.entries {
        if classify_member(&e.path).is_none() {
            return Err(format!("manifest lists an unclassified member: {}", e.path));
        }
        if !is_sha256_hex(&e.sha256) {
            return Err(format!("manifest entry has a malformed sha256: {}", e.path));
        }
    }
    for w in m.entries.windows(2) {
        if w[0].path == w[1].path {
            return Err(format!("duplicate manifest entry: {}", w[0].path));
        }
    }
    Ok(m)
}

/// Re-verify every manifest entry against the bytes actually present in the
/// package. `read` returns None for a missing member. The first mismatch
/// aborts, so a tampered or truncated package never reaches a write entry.
pub fn verify_members<F>(m: &BackupManifest, mut read: F) -> Result<(), String>
where
    F: FnMut(&str) -> Option<Vec<u8>>,
{
    for e in &m.entries {
        let Some(bytes) = read(&e.path) else {
            return Err(format!("manifest lists a missing member: {}", e.path));
        };
        if bytes.len() as u64 != e.size {
            return Err(format!(
                "member size mismatch for {}: manifest {} vs actual {}",
                e.path,
                e.size,
                bytes.len()
            ));
        }
        let actual = sha256_hex(&bytes);
        if actual != e.sha256 {
            return Err(format!("member hash mismatch for {} (package corrupted or tampered)", e.path));
        }
    }
    Ok(())
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// ---------------------------------------------------------------------------
// Passphrase envelope
// ---------------------------------------------------------------------------

/// Magic prefix of an encrypted package.
pub const ENVELOPE_MAGIC: [u8; 8] = *b"EGBKPKG1";
/// Envelope format version.
pub const ENVELOPE_VERSION: u8 = 1;
/// KDF id 1 = PBKDF2-HMAC-SHA256.
pub const KDF_PBKDF2_HMAC_SHA256: u8 = 1;
/// KDF iteration count (OWASP password-storage guidance for PBKDF2-HMAC-SHA256).
pub const KDF_ITERATIONS: u32 = 600_000;
/// Salt length in bytes.
pub const SALT_LEN: usize = 16;
/// Random file-key length in bytes.
pub const FILE_KEY_LEN: usize = 32;
/// AEAD tag length in bytes.
pub const TAG_LEN: usize = 16;
/// Wrapped file key length (key + tag).
pub const WRAPPED_KEY_LEN: usize = FILE_KEY_LEN + TAG_LEN;
/// Fixed header length: magic + version + kdf id + iterations + salt +
/// wrap nonce + wrapped-key length.
pub const HEADER_LEN: usize = 8 + 1 + 1 + 4 + SALT_LEN + aead::NONCE_LEN + 2;

/// True when `bytes` starts with the encrypted-package magic.
pub fn is_encrypted(bytes: &[u8]) -> bool {
    bytes.len() >= ENVELOPE_MAGIC.len() && bytes[..ENVELOPE_MAGIC.len()] == ENVELOPE_MAGIC
}

/// Wrap `plaintext` (the finished zip bytes) in a passphrase envelope.
///
/// Layout: header(44) | wrapped_key(48) | content_nonce(12) | ciphertext+tag.
/// The header is authenticated as AAD for the key wrap; header plus
/// wrapped key plus content nonce are authenticated as AAD for the payload,
/// so no header field can be edited without failing the open.
pub fn seal_package(plaintext: &[u8], passphrase: &str) -> Result<Vec<u8>, String> {
    if passphrase.is_empty() {
        return Err("backup passphrase must not be empty".to_string());
    }
    let salt = random_bytes(SALT_LEN)?;
    let file_key = random_bytes(FILE_KEY_LEN)?;
    let wrap_nonce = random_bytes(aead::NONCE_LEN)?;
    let content_nonce = random_bytes(aead::NONCE_LEN)?;

    let kek = derive_kek(passphrase, &salt, KDF_ITERATIONS)?;
    let kek_key = aead_key(&kek)?;

    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(&ENVELOPE_MAGIC);
    header.push(ENVELOPE_VERSION);
    header.push(KDF_PBKDF2_HMAC_SHA256);
    header.extend_from_slice(&KDF_ITERATIONS.to_le_bytes());
    header.extend_from_slice(&salt);
    header.extend_from_slice(&wrap_nonce);
    header.extend_from_slice(&(WRAPPED_KEY_LEN as u16).to_le_bytes());
    debug_assert_eq!(header.len(), HEADER_LEN);

    let mut wrapped = file_key.clone();
    kek_key
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce_array(&wrap_nonce)?),
            Aad::from(header.as_slice()),
            &mut wrapped,
        )
        .map_err(|_| "wrap file key failed".to_string())?;

    let mut content_aad = Vec::with_capacity(HEADER_LEN + WRAPPED_KEY_LEN + aead::NONCE_LEN);
    content_aad.extend_from_slice(&header);
    content_aad.extend_from_slice(&wrapped);
    content_aad.extend_from_slice(&content_nonce);

    let file_key_obj = aead_key(&file_key)?;
    let mut out = plaintext.to_vec();
    file_key_obj
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce_array(&content_nonce)?),
            Aad::from(content_aad.as_slice()),
            &mut out,
        )
        .map_err(|_| "seal package failed".to_string())?;

    let mut result = Vec::with_capacity(HEADER_LEN + WRAPPED_KEY_LEN + aead::NONCE_LEN + out.len());
    result.extend_from_slice(&header);
    result.extend_from_slice(&wrapped);
    result.extend_from_slice(&content_nonce);
    result.extend_from_slice(&out);
    Ok(result)
}

/// Open a passphrase envelope, returning the plaintext package bytes.
/// A wrong passphrase and a tampered package are indistinguishable by design
/// (both fail the AEAD open), which is the honest failure shape.
pub fn open_package(bytes: &[u8], passphrase: &str) -> Result<Vec<u8>, String> {
    if !is_encrypted(bytes) {
        return Err("not an encrypted backup package".to_string());
    }
    let min_len = HEADER_LEN + WRAPPED_KEY_LEN + aead::NONCE_LEN + TAG_LEN;
    if bytes.len() < min_len {
        return Err("encrypted package is truncated".to_string());
    }
    if bytes[8] != ENVELOPE_VERSION {
        return Err(format!("unsupported envelope version {}", bytes[8]));
    }
    if bytes[9] != KDF_PBKDF2_HMAC_SHA256 {
        return Err(format!("unsupported kdf id {}", bytes[9]));
    }
    let iterations = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
    let salt = &bytes[14..14 + SALT_LEN];
    let wrap_nonce = &bytes[14 + SALT_LEN..14 + SALT_LEN + aead::NONCE_LEN];
    let wkl_at = 14 + SALT_LEN + aead::NONCE_LEN;
    let wrapped_key_len = u16::from_le_bytes([bytes[wkl_at], bytes[wkl_at + 1]]) as usize;
    if wrapped_key_len != WRAPPED_KEY_LEN {
        return Err(format!("unsupported wrapped key length {wrapped_key_len}"));
    }
    let header = &bytes[..HEADER_LEN];
    let wrapped = &bytes[HEADER_LEN..HEADER_LEN + WRAPPED_KEY_LEN];
    let nonce_at = HEADER_LEN + WRAPPED_KEY_LEN;
    let content_nonce = &bytes[nonce_at..nonce_at + aead::NONCE_LEN];
    let ciphertext = &bytes[nonce_at + aead::NONCE_LEN..];

    let kek = derive_kek(passphrase, salt, iterations)?;
    let kek_key = aead_key(&kek)?;

    let mut file_key = [0u8; FILE_KEY_LEN];
    {
        let mut buf = wrapped.to_vec();
        let plain = kek_key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce_array(wrap_nonce)?),
                Aad::from(header),
                &mut buf,
            )
            .map_err(|_| "wrong passphrase or corrupted package".to_string())?;
        if plain.len() != FILE_KEY_LEN {
            return Err("unwrapped file key has the wrong length".to_string());
        }
        file_key.copy_from_slice(plain);
    }

    let mut content_aad = Vec::with_capacity(HEADER_LEN + WRAPPED_KEY_LEN + aead::NONCE_LEN);
    content_aad.extend_from_slice(header);
    content_aad.extend_from_slice(wrapped);
    content_aad.extend_from_slice(content_nonce);

    let file_key_obj = aead_key(&file_key)?;
    let mut out = ciphertext.to_vec();
    let plain = file_key_obj
        .open_in_place(
            Nonce::assume_unique_for_key(nonce_array(content_nonce)?),
            Aad::from(content_aad.as_slice()),
            &mut out,
        )
        .map_err(|_| "package authentication failed (wrong passphrase or tampering)".to_string())?;
    Ok(plain.to_vec())
}

fn random_bytes(n: usize) -> Result<Vec<u8>, String> {
    let rng = SystemRandom::new();
    let mut buf = vec![0u8; n];
    rng.fill(&mut buf)
        .map_err(|_| "system CSPRNG unavailable".to_string())?;
    Ok(buf)
}

fn derive_kek(passphrase: &str, salt: &[u8], iterations: u32) -> Result<[u8; 32], String> {
    let iters = NonZeroU32::new(iterations)
        .ok_or_else(|| "kdf iterations must be non-zero".to_string())?;
    let mut kek = [0u8; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        iters,
        salt,
        passphrase.as_bytes(),
        &mut kek,
    );
    Ok(kek)
}

fn aead_key(bytes: &[u8]) -> Result<LessSafeKey, String> {
    let unbound = UnboundKey::new(&aead::CHACHA20_POLY1305, bytes)
        .map_err(|_| "invalid AEAD key length".to_string())?;
    Ok(LessSafeKey::new(unbound))
}

fn nonce_array(raw: &[u8]) -> Result<[u8; aead::NONCE_LEN], String> {
    if raw.len() != aead::NONCE_LEN {
        return Err(format!(
            "nonce must be {} bytes, got {}",
            aead::NONCE_LEN,
            raw.len()
        ));
    }
    let mut a = [0u8; aead::NONCE_LEN];
    a.copy_from_slice(raw);
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, body: &str) -> ManifestEntry {
        manifest_entry(path, body.as_bytes())
    }

    fn sample_manifest() -> BackupManifest {
        build_manifest(
            "2026-09-14T00:00:00Z",
            vec![
                entry(CONFIG_ENTRY, r#"{"format":"egressapikey-config"}"#),
                entry(STRATEGY_ENTRY, r#"{"version":1}"#),
                entry(PORTS_ENTRY, r#"{"version":2}"#),
                entry(PORT_DB_ENTRY, "SQLite format 3"),
                entry(AUDIT_ENTRY, r#"{"schema":"audit/v1"}"#),
                entry(SETTINGS_ENTRY, r#"{"lang":"en"}"#),
                entry(STATE_DB_ENTRY, "SQLite format 3"),
                entry(CACHE_DB_ENTRY, "SQLite format 3"),
            ],
        )
        .unwrap()
    }

    // -- classification ----------------------------------------------------

    #[test]
    fn classify_member_maps_every_legislated_class() {
        assert_eq!(classify_member(CONFIG_ENTRY), Some(BackupClass::Config));
        assert_eq!(classify_member(STRATEGY_ENTRY), Some(BackupClass::Config));
        assert_eq!(classify_member(PORTS_ENTRY), Some(BackupClass::Config));
        assert_eq!(classify_member(PORT_DB_ENTRY), Some(BackupClass::Config));
        assert_eq!(classify_member(SETTINGS_ENTRY), Some(BackupClass::L1));
        assert_eq!(classify_member(AUDIT_ENTRY), Some(BackupClass::Audit));
        assert_eq!(classify_member(STATE_DB_ENTRY), Some(BackupClass::L3));
        assert_eq!(classify_member(CACHE_DB_ENTRY), Some(BackupClass::L3));
    }

    #[test]
    fn whitebox_history_and_audit_archives_classify() {
        assert_eq!(
            classify_member("l2/backup/egressapikey-ports.json.1756600000.bak"),
            Some(BackupClass::Config)
        );
        assert_eq!(
            classify_member("l2/audit.jsonl.3"),
            Some(BackupClass::Audit)
        );
    }

    #[test]
    fn request_log_members_are_forbidden_in_any_directory_form() {
        for p in [
            "request_logs-1756600000000.db",
            "l3/resin-state/request_logs-1.db",
            "l3/resin-logs/Request_Logs-9.DB",
        ] {
            assert!(is_forbidden_member(p), "{p} must be rejected");
            assert_eq!(classify_member(p), None, "{p} has no class");
        }
        assert!(!is_forbidden_member(STATE_DB_ENTRY));
    }

    #[test]
    fn restore_action_keeps_derived_state_and_evidence_out_of_write_back() {
        assert_eq!(
            restore_action(BackupClass::Config),
            RestoreAction::WriteThroughL2Entry
        );
        assert_eq!(
            restore_action(BackupClass::L1),
            RestoreAction::WriteThroughL1Entry
        );
        assert_eq!(restore_action(BackupClass::L3), RestoreAction::EvidenceOnly);
        assert_eq!(restore_action(BackupClass::Audit), RestoreAction::EvidenceOnly);
    }

    // -- manifest ---------------------------------------------------------

    #[test]
    fn manifest_round_trips_through_json() {
        let m = sample_manifest();
        let bytes = manifest_json(&m).unwrap();
        assert_eq!(bytes.last(), Some(&b'\n'));
        assert_eq!(parse_manifest(&bytes).unwrap(), m);
    }

    #[test]
    fn manifest_entries_are_sorted_by_path() {
        let m = sample_manifest();
        let mut sorted = m.entries.clone();
        sorted.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(m.entries, sorted);
    }

    #[test]
    fn build_manifest_rejects_forbidden_unclassified_and_duplicate_members() {
        assert!(build_manifest("t", vec![entry("request_logs-1.db", "x")])
            .unwrap_err()
            .contains("request-log leak line"));
        assert!(build_manifest("t", vec![entry("notes.txt", "x")])
            .unwrap_err()
            .contains("not part of any legislated class"));
        assert!(build_manifest(
            "t",
            vec![entry(AUDIT_ENTRY, "a"), entry(AUDIT_ENTRY, "b")]
        )
        .unwrap_err()
        .contains("duplicate manifest entry"));
    }

    #[test]
    fn parse_manifest_rejects_wrong_tag_future_version_and_bad_hash() {
        let m = sample_manifest();
        let mut v: serde_json::Value =
            serde_json::from_slice(&manifest_json(&m).unwrap()).unwrap();

        let mut wrong = v.clone();
        wrong["format"] = serde_json::json!("someone-elses-backup");
        assert!(parse_manifest(wrong.to_string().as_bytes())
            .unwrap_err()
            .contains("unsupported backup format"));

        let mut future = v.clone();
        future["version"] = serde_json::json!(99);
        assert!(parse_manifest(future.to_string().as_bytes())
            .unwrap_err()
            .contains("unsupported backup format version"));

        v["entries"][0]["sha256"] = serde_json::json!("not-a-hash");
        assert!(parse_manifest(v.to_string().as_bytes())
            .unwrap_err()
            .contains("malformed sha256"));
    }

    #[test]
    fn parse_manifest_rejects_a_request_log_entry() {
        let m = sample_manifest();
        let mut v: serde_json::Value =
            serde_json::from_slice(&manifest_json(&m).unwrap()).unwrap();
        v["entries"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "path": "l3/resin-state/request_logs-1.db",
                "sha256": "0".repeat(64),
                "size": 1
            }));
        assert!(parse_manifest(v.to_string().as_bytes())
            .unwrap_err()
            .contains("request-log leak line"));
    }

    #[test]
    fn verify_members_accepts_intact_and_detects_tamper_and_missing() {
        let m = sample_manifest();
        let mut bodies: Vec<(String, Vec<u8>)> = Vec::new();
        for (i, e) in m.entries.iter().enumerate() {
            bodies.push((e.path.clone(), format!("member-{i}").into_bytes()));
        }
        // Rebuild the manifest so the recorded hashes match these bodies.
        let m2 = build_manifest(
            "2026-09-14T00:00:00Z",
            bodies
                .iter()
                .map(|(p, b)| manifest_entry(p, b))
                .collect(),
        )
        .unwrap();
        let lookup = |path: &str| -> Option<Vec<u8>> {
            m2.entries
                .iter()
                .position(|e| e.path == path)
                .map(|i| format!("member-{i}").into_bytes())
        };
        assert!(verify_members(&m2, lookup).is_ok());

        let tampered = |path: &str| -> Option<Vec<u8>> {
            if path == AUDIT_ENTRY {
                Some(b"tampered".to_vec())
            } else {
                lookup(path)
            }
        };
        assert!(verify_members(&m2, tampered).unwrap_err().contains("mismatch"));

        let missing = |path: &str| -> Option<Vec<u8>> {
            if path == CACHE_DB_ENTRY {
                None
            } else {
                lookup(path)
            }
        };
        assert!(verify_members(&m2, missing)
            .unwrap_err()
            .contains("missing member"));
    }

    // -- envelope ---------------------------------------------------------

    #[test]
    fn envelope_round_trips_and_is_detectable_by_magic() {
        let plain = b"PK\x03\x04 pretend zip bytes".to_vec();
        let sealed = seal_package(&plain, "correct horse battery staple").unwrap();
        assert!(is_encrypted(&sealed));
        assert!(!is_encrypted(&plain));
        assert_eq!(sealed[8], ENVELOPE_VERSION);
        assert_eq!(sealed[9], KDF_PBKDF2_HMAC_SHA256);
        assert_eq!(open_package(&sealed, "correct horse battery staple").unwrap(), plain);
    }

    #[test]
    fn envelope_rejects_empty_passphrase_and_plaintext_input() {
        assert!(seal_package(b"x", "").unwrap_err().contains("must not be empty"));
        assert!(open_package(b"not-an-envelope", "pw")
            .unwrap_err()
            .contains("not an encrypted backup package"));
    }

    #[test]
    fn envelope_rejects_wrong_passphrase_and_any_tampering() {
        let plain = b"package payload".to_vec();
        let sealed = seal_package(&plain, "right-passphrase").unwrap();

        assert!(open_package(&sealed, "wrong-passphrase")
            .unwrap_err()
            .contains("wrong passphrase or corrupted package"));

        // Flip a byte in the ciphertext body.
        let mut body = sealed.clone();
        let last = body.len() - 1;
        body[last] ^= 0x01;
        assert!(open_package(&body, "right-passphrase").is_err());

        // Flip a byte in the authenticated header (salt).
        let mut head = sealed.clone();
        head[14] ^= 0x01;
        assert!(open_package(&head, "right-passphrase").is_err());

        // Flip a byte in the wrapped file key.
        let mut wrap = sealed.clone();
        wrap[HEADER_LEN] ^= 0x01;
        assert!(open_package(&wrap, "right-passphrase").is_err());

        // Truncate below the minimum envelope length.
        assert!(open_package(&sealed[..HEADER_LEN], "right-passphrase")
            .unwrap_err()
            .contains("truncated"));
    }

    #[test]
    fn envelope_rejects_unknown_version_and_kdf_id() {
        let sealed = seal_package(b"payload", "pw").unwrap();

        let mut v = sealed.clone();
        v[8] = 9;
        assert!(open_package(&v, "pw")
            .unwrap_err()
            .contains("unsupported envelope version"));

        let mut k = sealed.clone();
        k[9] = 7;
        assert!(open_package(&k, "pw")
            .unwrap_err()
            .contains("unsupported kdf id"));
    }

    #[test]
    fn two_seals_of_the_same_plaintext_differ() {
        let plain = b"same bytes".to_vec();
        let a = seal_package(&plain, "pw").unwrap();
        let b = seal_package(&plain, "pw").unwrap();
        assert_ne!(a, b, "fresh salt/nonce/file key per seal");
        assert_eq!(open_package(&a, "pw").unwrap(), plain);
        assert_eq!(open_package(&b, "pw").unwrap(), plain);
    }
}
