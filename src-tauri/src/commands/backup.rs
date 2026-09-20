//! backup domain IPC commands (EgressAPIKEY).
//!
//! Extracted from the former commands/mod.rs monolith
//! pure mechanical move - no behavior, naming, or IPC-surface change.
//!
//! `backup_create` was rewritten (ADR-0070) so the archive actually carries
//! the configuration that decides behaviour (the CONTEXT.md "Backup Scope"
//! ledger), and `backup_restore` adds download -> verify -> import through the
//! EXISTING authoritative write entries. The request-log DBs can never enter a
//! package, enforced on both the packing and the restore side.
use super::common::{map_resin_error, resin_client};
use super::strategy::{reconcile_ports_half, strategy_service};
use crate::sidecar::SidecarHandle;
use resin_core::backup as backup_model;
use resin_core::resolve_id_in;
use resin_core::IpcError;
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_store::StoreExt;

/// L1 store file name. The tauri-plugin-store document is the single L1
/// authoritative write entry (the ADR-0036 discipline applied to the GUI
/// layer), so a restore re-enters it instead of editing the file by hand.
const SETTINGS_STORE: &str = "settings.json";
/// Upper bound on a downloaded package: a hostile or broken WebDAV endpoint
/// must not be able to make the shell allocate without limit.
const MAX_BACKUP_BYTES: u64 = 256 * 1024 * 1024;
/// Upper bound on the total uncompressed member bytes accepted from a package.
const MAX_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
/// Upper bound on the number of members accepted from a package.
const MAX_PACKAGE_MEMBERS: usize = 4096;
/// Cap on whitebox history members packed (two documents x 10 kept + slack).
const MAX_HISTORY_MEMBERS: usize = 32;

/// L1 + L2 authority directory: settings.json, both whitebox documents,
/// egressapikey.db, audit.jsonl and the whitebox backup/ history.
/// CONTEXT.md "Config Authority".
fn cfg_dir(app: &AppHandle) -> Result<PathBuf, IpcError> {
    app.path()
        .app_config_dir()
        .map_err(|e| IpcError::from(e.to_string()))
}

/// L3 runtime root: resin-state/ and resin-cache/. Resin private state is
/// packaged read-only and never written back (ADR-0050-bis).
fn data_dir(app: &AppHandle) -> Result<PathBuf, IpcError> {
    app.path()
        .app_data_dir()
        .map_err(|e| IpcError::from(e.to_string()))
}

/// App-owned directory holding produced archives, the same-volume download
/// target and the SQLite snapshot scratch space.
fn backups_dir(app: &AppHandle) -> Result<PathBuf, IpcError> {
    let dir = data_dir(app)?.join("backups");
    std::fs::create_dir_all(&dir).map_err(|e| IpcError::from(e.to_string()))?;
    Ok(dir)
}

/// Best-effort random suffix for archive and scratch names. The threat model
/// is path guessing on a shared host, not secrecy: the file stays inside the
/// per-user app data directory either way.
fn random_suffix() -> String {
    let mut rand_bytes = [0u8; 8];
    match std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut rand_bytes)) {
        Ok(()) => {}
        Err(_) => {
            // Windows has no /dev/urandom: fall back to time+pid mixing.
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0)
                .wrapping_mul(std::process::id() as u64);
            let mut s = seed;
            for b in rand_bytes.iter_mut() {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                *b = (s >> 56) as u8;
            }
        }
    }
    rand_bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Archive-name timestamp. Keeps the pre-existing egressapikey-backup-* naming
/// so archives produced before and after this change sort together.
fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()
}

/// Add one on-disk file as a package member.
///
/// Absent files are skipped on purpose: a first run has no audit log, no
/// whitebox history and possibly no settings document yet, and config.json
/// still carries the effective configuration either way.
fn push_file(members: &mut Vec<(String, Vec<u8>)>, src: &Path, entry: &str) {
    if let Ok(bytes) = std::fs::read(src) {
        members.push((entry.to_string(), bytes));
    }
}

/// Snapshot a SQLite database consistently.
///
/// A bare file read of a live database is the failure mode sqlite.org names
/// explicitly: the copy can capture a torn page or a half-applied WAL. The
/// snapshot goes through resin_core::db::snapshot_db_readonly, which runs
/// VACUUM INTO on a read-only source handle - a fresh, fully checkpointed
/// database with the source untouched, so the ADR-0050-bis read-only
/// constraint on Resin private state holds.
///
/// When no consistent snapshot can be taken the database is packed as a cold
/// copy together with its -wal/-shm siblings and the degradation is logged, so
/// the database is still represented as evidence instead of silently
/// disappearing from the archive.
fn push_sqlite_snapshot(
    members: &mut Vec<(String, Vec<u8>)>,
    src: &Path,
    entry: &str,
    scratch: &Path,
) {
    if !src.is_file() {
        return;
    }
    let tmp = scratch.join(format!("snapshot-{}.db", random_suffix()));
    if resin_core::db::snapshot_db_readonly(src, &tmp).is_ok() {
        if let Ok(bytes) = std::fs::read(&tmp) {
            members.push((entry.to_string(), bytes));
        }
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    tracing::warn!(
        db = %src.display(),
        "backup: consistent SQLite snapshot unavailable; packing a cold copy (db + -wal/-shm)"
    );
    push_file(members, src, entry);
    for ext in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{ext}", src.display()));
        push_file(members, &side, &format!("{entry}{ext}"));
    }
    let _ = std::fs::remove_file(&tmp);
}

/// Pack the whitebox version history (app_config_dir()/backup/*.bak).
/// CONTEXT.md "Backup Scope" class (a) includes it, so it rides along; the
/// list is name-filtered and truncated so a pathological directory cannot
/// balloon the archive.
fn push_whitebox_history(members: &mut Vec<(String, Vec<u8>)>, cfg: &Path) {
    let dir = cfg.join(resin_core::whitebox_backup::BACKUP_DIR_NAME);
    let Ok(read_dir) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut names: Vec<String> = read_dir
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".bak"))
        .collect();
    names.sort();
    names.truncate(MAX_HISTORY_MEMBERS);
    for n in names {
        let entry = format!("{}/{}", backup_model::WHITEBOX_HISTORY_DIR, n);
        push_file(members, &dir.join(n.as_str()), &entry);
    }
}

/// Create a backup package and return its path.
///
/// ADR-0070: the archive carries exactly the
/// CONTEXT.md "Backup Scope" ledger:
///
///   manifest.json   integrity gate (sha256 + size per member); never written
///   config.json     the ADR-0061 container, whitebox as the source
///   settings.json   L1 GUI preferences
///   l2/*            both whitebox documents, the port-mapping DB, the audit
///                   chain and the whitebox version history
///   l3/*            Resin runtime state, packaged READ-ONLY as evidence
///
/// The previous implementation read app_data_dir() and packed only
/// settings.json plus two Resin databases. On Linux, where app_config_dir()
/// and app_data_dir() are different directories, that silently archived the
/// WRONG root and captured none of the whitebox at all; it also looked for the
/// cache DB under resin-state/, where it never lives (resin-cache/ does).
/// Windows and macOS hid both defects because the two roots coincide there.
///
/// passphrase (non-empty) wraps the finished zip in the AEAD envelope from
/// resin_core::backup; None or empty keeps a plain zip, so a restore can tell
/// the two apart from the package magic instead of from a flag.
#[tauri::command]
pub async fn backup_create(
    app: AppHandle,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    passphrase: Option<String>,
) -> Result<String, IpcError> {
    let cfg = cfg_dir(&app)?;
    let data = data_dir(&app)?;
    let out_dir = backups_dir(&app)?;
    let svc = strategy_service(&app)?;
    backup_create_impl(&cfg, &data, &out_dir, &svc, &whitebox, passphrase).await
}

/// Transport-free body (R11-03): the headless BFF passes its own state roots
/// and StrategyService; the packing logic is identical.
pub async fn backup_create_impl(
    cfg: &Path,
    data: &Path,
    out_dir: &Path,
    svc: &resin_core::StrategyService<resin_core::FsStrategyStore>,
    whitebox: &resin_core::WhiteboxConfigStore,
    passphrase: Option<String>,
) -> Result<String, IpcError> {
    let mut members: Vec<(String, Vec<u8>)> = Vec::new();

    // Config layer: one canonical re-importable container (ADR-0061). This is
    // the member a restore replays, which is how the two backup models stop
    // competing - backup now speaks the same container as config_export and
    // config_import.
    let strategy = svc.get().map_err(IpcError::from)?;
    let ports = whitebox.snapshot();
    let exported_at = chrono::Local::now().to_rfc3339();
    let container = resin_core::build_export_doc(&strategy, &ports, &exported_at);
    let mut config_bytes = serde_json::to_vec_pretty(&container)
        .map_err(|e| IpcError::from(format!("config.json serialize: {e}")))?;
    config_bytes.push(b'\n');
    members.push((backup_model::CONFIG_ENTRY.to_string(), config_bytes));

    // L1 GUI preferences.
    push_file(
        &mut members,
        &cfg.join(SETTINGS_STORE),
        backup_model::SETTINGS_ENTRY,
    );

    // L2 whitebox documents, port-mapping DB, audit chain, version history.
    push_file(
        &mut members,
        &cfg.join("egressapikey-strategy.json"),
        backup_model::STRATEGY_ENTRY,
    );
    push_file(
        &mut members,
        &cfg.join("egressapikey-ports.json"),
        backup_model::PORTS_ENTRY,
    );
    push_sqlite_snapshot(
        &mut members,
        &cfg.join("egressapikey.db"),
        backup_model::PORT_DB_ENTRY,
        &out_dir,
    );
    push_file(
        &mut members,
        &cfg.join(resin_core::audit::AUDIT_LOG_FILE),
        backup_model::AUDIT_ENTRY,
    );
    for i in 1..=resin_core::audit::AUDIT_LOG_MAXBACKUP {
        let archived = format!("{}.{i}", resin_core::audit::AUDIT_LOG_FILE);
        let entry = format!("{}{i}", backup_model::AUDIT_ARCHIVE_PREFIX);
        push_file(&mut members, &cfg.join(&archived), &entry);
    }
    push_whitebox_history(&mut members, &cfg);

    // L3 derived runtime state: read-only evidence, never written back.
    push_sqlite_snapshot(
        &mut members,
        &data.join("resin-state").join("state.db"),
        backup_model::STATE_DB_ENTRY,
        &out_dir,
    );
    push_sqlite_snapshot(
        &mut members,
        &data.join("resin-cache").join("cache.db"),
        backup_model::CACHE_DB_ENTRY,
        &out_dir,
    );

    // Leak line (CONTEXT.md "Backup Scope" class c): the request-log DBs are
    // refused by the manifest gate below, so this assert is the packing-side
    // half of the same rule.
    let names: Vec<String> = members.iter().map(|(n, _)| n.clone()).collect();
    backup_model::assert_no_forbidden(&names).map_err(IpcError::from)?;

    // Manifest: one sha256 + size per member, so a restore can prove the
    // package is intact before a single byte is written back.
    let entries: Vec<backup_model::ManifestEntry> = members
        .iter()
        .map(|(path, bytes)| backup_model::manifest_entry(path, bytes))
        .collect();
    let manifest = backup_model::build_manifest(&exported_at, entries).map_err(IpcError::from)?;
    let manifest_bytes = backup_model::manifest_json(&manifest).map_err(IpcError::from)?;

    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    let opts = zip::write::FileOptions::default();
    zip.start_file(backup_model::MANIFEST_ENTRY, opts)
        .map_err(|e| IpcError::from(e.to_string()))?;
    zip.write_all(&manifest_bytes)
        .map_err(|e| IpcError::from(e.to_string()))?;
    for (path, bytes) in &members {
        zip.start_file(path.as_str(), opts)
            .map_err(|e| IpcError::from(e.to_string()))?;
        zip.write_all(bytes)
            .map_err(|e| IpcError::from(e.to_string()))?;
    }
    let mut package = zip
        .finish()
        .map_err(|e| IpcError::from(e.to_string()))?
        .into_inner();

    if let Some(pass) = passphrase.filter(|p| !p.is_empty()) {
        // PBKDF2-HMAC-SHA256 at 600k iterations is ~hundreds of ms of pure
        // CPU; keep it off the async runtime worker (spawn_blocking is the
        // codebase's offload pattern for blocking work).
        let plain = package;
        package = tokio::task::spawn_blocking(move || backup_model::seal_package(&plain, &pass))
            .await
            .map_err(|e| IpcError::from(format!("seal_package join: {e}")))?
            .map_err(IpcError::from)?;
    }

    let file_name = format!("egressapikey-backup-{}-{}.zip", stamp(), random_suffix());
    let path = out_dir.join(&file_name);
    std::fs::write(&path, &package).map_err(|e| IpcError::from(e.to_string()))?;
    tracing::info!(
        members = members.len(),
        bytes = package.len(),
        encrypted = backup_model::is_encrypted(&package),
        "backup_create complete"
    );
    Ok(path.to_string_lossy().to_string())
}

/// Upload a backup zip to a WebDAV server.
/// url/username/password come from tauri-plugin-store (server-trust, never webview raw).
#[tauri::command]
pub async fn backup_upload(
    app: AppHandle,
    url: String,
    username: String,
    password: String,
    zip_path: String,
) -> Result<(), IpcError> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| IpcError::from(e.to_string()))?;
    let backups_dir = app_data.join("backups");
    backup_upload_impl(&backups_dir, url, username, password, zip_path).await
}

/// Transport-free body (R11-03). The backups_dir confinement root is the
/// caller's responsibility — the shell passes app_data/backups, the headless
/// BFF passes state_root/backups.
pub async fn backup_upload_impl(
    backups_dir: &Path,
    url: String,
    username: String,
    password: String,
    zip_path: String,
) -> Result<(), IpcError> {
    if url.trim().is_empty() {
        return Err(IpcError::from("webdav url must not be empty".to_string()));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(IpcError::from(
            "webdav url must start with http:// or https://".to_string(),
        ));
    }
    if url.len() > 2048 {
        return Err(IpcError::from("webdav url too long".to_string()));
    }

    // Security: confine zip_path to the per-user app_data/backups dir.
    // Canonicalize both and require backups_dir to be a prefix; reject ../
    // escapes and absolute paths outside app data. Prevents a compromised
    // webview from exfiltrating arbitrary files (e.g. the Resin admin token,
    // settings.json, or system files) to an attacker-controlled WebDAV URL.
    std::fs::create_dir_all(backups_dir).map_err(|e| IpcError::from(e.to_string()))?;
    let canon_backup = std::fs::canonicalize(backups_dir)
        .map_err(|e| IpcError::from(format!("backups dir not accessible: {e}")))?;
    let canon_zip = std::fs::canonicalize(&zip_path)
        .map_err(|e| IpcError::from(format!("zip path not accessible: {e}")))?;
    if !canon_zip.starts_with(&canon_backup) {
        return Err(IpcError::from(
            "zip path must be inside the app backups directory".to_string(),
        ));
    }
    if !canon_zip.is_file() {
        return Err(IpcError::from("zip path is not a file".to_string()));
    }

    let data = std::fs::read(&canon_zip).map_err(|e| IpcError::from(e.to_string()))?;
    let zip_name = canon_zip
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "backup.zip".to_string());
    let webdav_url = format!("{}/{}", url.trim_end_matches('/'), zip_name);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| IpcError::from(e.to_string()))?;
    let resp = client
        .put(&webdav_url)
        .basic_auth(&username, Some(&password))
        .body(data)
        .send()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(IpcError::from(format!(
            "webdav upload failed: HTTP {}",
            resp.status()
        )))
    }
}

/// List backups on the WebDAV server (PROPFIND).
#[tauri::command]
pub async fn backup_list(
    url: String,
    username: String,
    password: String,
) -> Result<Vec<String>, IpcError> {
    if url.trim().is_empty() {
        return Err(IpcError::from("webdav url must not be empty".to_string()));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(IpcError::from(
            "webdav url must start with http:// or https://".to_string(),
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| IpcError::from(e.to_string()))?;
    let resp = client
        .request(
            reqwest::Method::from_bytes(b"PROPFIND").unwrap(),
            url.trim_end_matches('/'),
        )
        .basic_auth(&username, Some(&password))
        .header("Depth", "1")
        .header("Content-Type", "application/xml")
        .body(
            r#"<?xml version="1.0"?><propfind xmlns="DAV:"><prop><displayname/></prop></propfind>"#,
        )
        .send()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
    if !resp.status().is_success() {
        return Err(IpcError::from(format!(
            "webdav PROPFIND failed: HTTP {}",
            resp.status()
        )));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
    // Parse <D:href> or <D:displayname> entries
    let mut names = Vec::new();
    for part in body.split("<D:href>").skip(1) {
        if let Some(end) = part.find("</D:href>") {
            let name = &part[..end];
            if name.ends_with(".zip") {
                names.push(name.rsplit('/').next().unwrap_or(name).to_string());
            }
        }
    }
    Ok(names)
}

/// ADR-0061: export the L2 whitebox configuration as JSON.
/// Reads `egressapikey-strategy.json` (via StrategyService, ADR-0036) and
/// `egressapikey-ports.json` (via the WhiteboxConfigStore snapshot) and wraps
/// them verbatim in a versioned container. It does NOT read Resin — Resin is
/// the derived L3 runtime, and the "Include Resin derived" option is deferred
/// (out of scope this round). The exported document re-imports through
/// config_import's schema gate.
#[tauri::command]
pub async fn config_export(
    app: AppHandle,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
) -> Result<serde_json::Value, IpcError> {
    let svc = strategy_service(&app)?;
    config_export_impl(&svc, &whitebox).await
}

/// Transport-free body (R11-03).
pub async fn config_export_impl(
    svc: &resin_core::StrategyService<resin_core::FsStrategyStore>,
    whitebox: &resin_core::WhiteboxConfigStore,
) -> Result<serde_json::Value, IpcError> {
    let strategy = svc.get().map_err(IpcError::from)?;
    let ports = whitebox.snapshot();
    let exported_at = chrono::Local::now().to_rfc3339();
    Ok(resin_core::build_export_doc(
        &strategy,
        &ports,
        &exported_at,
    ))
}

/// ADR-0061: import a config document (from config_export).
/// Parses + validates the two whitebox documents up front (schema + version
/// gate — any violation returns a clear IpcError and writes NOTHING), then
/// persists them through the single sanctioned write entries:
/// `StrategyService::store` (ADR-0036, versions the previous file) and
/// `WhiteboxConfigStore::apply` (validate -> DB -> file backup-before-write
/// -> atomic swap), and finally triggers the one-way reconcile (ADR-0054 §A)
/// so Resin converges from the imported whitebox (diff-then-skip per
/// ADR-0057; no direct Resin PATCH here). Subscriptions are not part of the
/// whitebox config layer and are not touched — the former Resin-derived
/// subscription import is gone.
#[tauri::command]
pub async fn config_import(
    app: AppHandle,
    sidecar: State<'_, SidecarHandle>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    db: State<'_, resin_core::DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    config: serde_json::Value,
) -> Result<serde_json::Value, IpcError> {
    let svc = strategy_service(&app)?;
    config_import_impl(&svc, &sidecar, &whitebox, &db, &forwarder, config).await
}

/// Transport-free body (R11-03).
pub async fn config_import_impl(
    svc: &resin_core::StrategyService<resin_core::FsStrategyStore>,
    sidecar: &SidecarHandle,
    whitebox: &resin_core::WhiteboxConfigStore,
    db: &resin_core::DbPool,
    forwarder: &resin_core::PortForwarder,
    config: serde_json::Value,
) -> Result<serde_json::Value, IpcError> {
    // Cap input size before any parse (AGENTS s7.5: 256KB max).
    let config_str = serde_json::to_string(&config).map_err(|e| IpcError::from(e.to_string()))?;
    if config_str.len() > 262_144 {
        return Err(IpcError::from(
            "config_import: config too large (max 256KB)".to_string(),
        ));
    }

    // ADR-0069 D4 phase 1: parse + validate BOTH whitebox documents through
    // the one shared gate BEFORE either is committed. Any rejection returns a
    // clear error and writes NOTHING - this is what removes the
    // half-imported state (a rejection of the second document can no longer
    // leave the first one already persisted).
    let doc = resin_core::parse_import_doc(&config).map_err(IpcError::from)?;
    resin_core::validate_import_pair(&doc.strategy, &doc.ports).map_err(IpcError::from)?;

    // ADR-0069 D4 phase 2: commit the two documents back-to-back through their
    // sanctioned write entries - the strategy half through ADR-0036 (store
    // bumps generation and returns the landed document), the ports half
    // through ADR-0055. No fallible work sits between the two commits, and
    // each entry is itself a temp-file write followed by an atomic rename
    // (`atomic_write_bytes`), so the pair can only ever land whole. The
    // entries are deliberately NOT bypassed: they own the generation bump
    // (ADR-0058), the backup-before-write (ADR-0054 section B) and the audit
    // row (ADR-0059).
    svc.store(doc.strategy.clone()).map_err(IpcError::from)?;
    whitebox
        .apply(db, forwarder, doc.ports.clone())
        .await
        .map_err(IpcError::from)?;

    // Trigger the one-way reconcile (ADR-0054 §A / F2): strategy apply
    // (diff-then-skip, ADR-0057) then ports re-assert — the SAME path as
    // reconcile_now, fail-fast, whitebox always wins.
    let client = resin_client(sidecar)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut errors: Vec<String> = Vec::new();
    match svc
        .reconcile(&client, resolve_id_in, async {
            reconcile_ports_half(sidecar, whitebox, forwarder, now).await
        })
        .await
    {
        Ok(report) => {
            tracing::info!(
                strategy_platforms = doc.strategy.platforms.len(),
                ports = doc.ports.entry_ports.len(),
                ports_restored = report.ports_restored.len(),
                "config_import complete (whitebox write + reconcile)"
            );
        }
        Err(e) => {
            // The whitebox is already written (it is the truth, ADR-0036);
            // Resin converges on the next apply / boot restore. Surface the
            // reconcile failure in the summary rather than falsely failing
            // the import after a successful write.
            tracing::warn!(error = %e, "config_import: whitebox written; reconcile failed");
            errors.push(format!("reconcile: {e}"));
        }
    }

    Ok(serde_json::json!({
        "platforms_created": doc.strategy.platforms.len(),
        "platforms_skipped": 0,
        "subscriptions_created": 0,
        "subscriptions_skipped": 0,
        "ports_created": doc.ports.entry_ports.len(),
        "errors": errors,
    }))
}

/// Restore a backup package produced by backup_create.
///
/// ADR-0070. Deliberately verify-then-write:
///
///   1. download the package from WebDAV (size-capped)
///   2. open the passphrase envelope when the package magic says it is sealed
///   3. read every member into memory, refusing the leak line, unsafe member
///      paths, and the size / count caps
///   4. parse the manifest and re-verify every member hash BEFORE writing
///      anything - any failure aborts with zero writes
///   5. replay the config half through the EXISTING authoritative entries
///      (StrategyService::store + WhiteboxConfigStore::apply + the one-way
///      reconcile) - the same path config_import uses, so backup/restore and
///      export/import stop being two competing models
///   6. replay the L1 half through the settings store, the L1 write entry
///   7. extract L3 and audit members to a restore-artifacts directory as
///      read-only evidence; they are never written back
///
/// Step 4 is the whole reason this is not a plain unzip: the whitebox is the
/// truth (ADR-0036), so a tampered or truncated package has to be rejected
/// before it can reach it.
#[tauri::command]
pub async fn backup_restore(
    app: AppHandle,
    sidecar: State<'_, SidecarHandle>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    db: State<'_, resin_core::DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    url: String,
    username: String,
    password: String,
    zip_name: String,
    passphrase: Option<String>,
) -> Result<serde_json::Value, IpcError> {
    let data = data_dir(&app)?;
    let svc = strategy_service(&app)?;
    backup_restore_impl(
        &data,
        &svc,
        &sidecar,
        &whitebox,
        &db,
        &forwarder,
        url,
        username,
        password,
        zip_name,
        passphrase,
        |obj| {
            let store = app
                .store(SETTINGS_STORE)
                .map_err(|e| IpcError::from(e.to_string()))?;
            for (key, val) in obj {
                store.set(key.clone(), val.clone());
            }
            store.save().map_err(|e| IpcError::from(e.to_string()))
        },
    )
    .await
}

/// Transport-free body (R11-03). The L1 restore half is a caller-supplied
/// settings writer: the shell writes through tauri-plugin-store, the headless
/// BFF writes through its own settings.json document.
pub async fn backup_restore_impl(
    data: &Path,
    svc: &resin_core::StrategyService<resin_core::FsStrategyStore>,
    sidecar: &SidecarHandle,
    whitebox: &resin_core::WhiteboxConfigStore,
    db: &resin_core::DbPool,
    forwarder: &resin_core::PortForwarder,
    url: String,
    username: String,
    password: String,
    zip_name: String,
    passphrase: Option<String>,
    settings_apply: impl FnOnce(&serde_json::Map<String, serde_json::Value>) -> Result<(), IpcError>,
) -> Result<serde_json::Value, IpcError> {
    // ---- IPC-boundary validation (AGENTS 7.5) ---------------------------
    let base = url.trim().trim_end_matches('/').to_string();
    if base.is_empty() {
        return Err(IpcError::from("webdav url must not be empty".to_string()));
    }
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(IpcError::from(
            "webdav url must start with http:// or https://".to_string(),
        ));
    }
    if base.len() > 2048 {
        return Err(IpcError::from("webdav url too long".to_string()));
    }
    let name = zip_name.trim();
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(IpcError::from(
            "backup name must be a plain file name".to_string(),
        ));
    }
    if !name.ends_with(".zip") {
        return Err(IpcError::from("backup name must end with .zip".to_string()));
    }

    // ---- download ------------------------------------------------------
    let remote = format!("{}/{}", base, name);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| IpcError::from(e.to_string()))?;
    let resp = client
        .get(&remote)
        .basic_auth(&username, Some(&password))
        .send()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
    if !resp.status().is_success() {
        return Err(IpcError::from(format!(
            "webdav download failed: HTTP {}",
            resp.status()
        )));
    }
    if let Some(len) = resp.content_length() {
        if len > MAX_BACKUP_BYTES {
            return Err(IpcError::from(format!(
                "backup package too large: {len} bytes (max {MAX_BACKUP_BYTES})"
            )));
        }
    }
    let mut package = resp
        .bytes()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?
        .to_vec();
    if package.len() as u64 > MAX_BACKUP_BYTES {
        return Err(IpcError::from(format!(
            "backup package too large: {} bytes (max {MAX_BACKUP_BYTES})",
            package.len()
        )));
    }

    // ---- passphrase envelope -------------------------------------------
    if backup_model::is_encrypted(&package) {
        let pass = passphrase.unwrap_or_default();
        if pass.is_empty() {
            return Err(IpcError::from(
                "this backup package is encrypted; a passphrase is required".to_string(),
            ));
        }
        // Same 600k-iteration PBKDF2 as seal_package — offloaded likewise.
        let sealed = package;
        package = tokio::task::spawn_blocking(move || backup_model::open_package(&sealed, &pass))
            .await
            .map_err(|e| IpcError::from(format!("open_package join: {e}")))?
            .map_err(IpcError::from)?;
    }

    // ---- read + screen every member BEFORE any write --------------------
    let mut archive = zip::ZipArchive::new(Cursor::new(package))
        .map_err(|e| IpcError::from(format!("backup package is not a readable zip: {e}")))?;
    if archive.len() > MAX_PACKAGE_MEMBERS {
        return Err(IpcError::from(format!(
            "backup package has too many members: {}",
            archive.len()
        )));
    }
    let mut members: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| IpcError::from(format!("backup member {i}: {e}")))?;
        if entry.is_dir() {
            continue;
        }
        let member_name = entry.name().to_string();
        // Zip-slip defence: a member may only be a plain relative path.
        if member_name.starts_with('/')
            || member_name.contains('\\')
            || member_name
                .split('/')
                .any(|seg| seg.is_empty() || seg == "." || seg == "..")
        {
            return Err(IpcError::from(format!(
                "backup member has an unsafe path: {member_name}"
            )));
        }
        if backup_model::is_forbidden_member(&member_name) {
            return Err(IpcError::from(format!(
                "backup package rejected (request-log leak line): {member_name}"
            )));
        }
        if member_name != backup_model::MANIFEST_ENTRY
            && backup_model::classify_member(&member_name).is_none()
        {
            return Err(IpcError::from(format!(
                "backup package carries an unclassified member: {member_name}"
            )));
        }
        let declared = entry.size();
        if declared > MAX_UNCOMPRESSED_BYTES
            || total.saturating_add(declared) > MAX_UNCOMPRESSED_BYTES
        {
            return Err(IpcError::from(format!(
                "backup package expands beyond {MAX_UNCOMPRESSED_BYTES} bytes"
            )));
        }
        // Pre-allocate from the declared size but never trust it for the cap.
        let mut buf = Vec::with_capacity(declared.min(1 << 20) as usize);
        entry
            .take(MAX_UNCOMPRESSED_BYTES + 1)
            .read_to_end(&mut buf)
            .map_err(|e| IpcError::from(format!("backup member {member_name}: {e}")))?;
        total = total.saturating_add(buf.len() as u64);
        if total > MAX_UNCOMPRESSED_BYTES {
            return Err(IpcError::from(format!(
                "backup package expands beyond {MAX_UNCOMPRESSED_BYTES} bytes"
            )));
        }
        if members.insert(member_name.clone(), buf).is_some() {
            return Err(IpcError::from(format!(
                "backup package lists {member_name} twice"
            )));
        }
    }

    let manifest_bytes = members
        .remove(backup_model::MANIFEST_ENTRY)
        .ok_or_else(|| IpcError::from("backup package has no manifest.json".to_string()))?;
    let manifest = backup_model::parse_manifest(&manifest_bytes).map_err(IpcError::from)?;
    // Nothing rides along unverified: every member present must be listed, and
    // every listed member must hash-match the bytes actually in the archive.
    for path in members.keys() {
        if !manifest.entries.iter().any(|e| &e.path == path) {
            return Err(IpcError::from(format!(
                "backup member is not listed in the manifest: {path}"
            )));
        }
    }
    backup_model::verify_members(&manifest, |p| members.get(p).cloned()).map_err(IpcError::from)?;

    // ---- config half: the EXISTING authoritative write entries -----------
    let config_value: serde_json::Value = serde_json::from_slice(
        members
            .get(backup_model::CONFIG_ENTRY)
            .ok_or_else(|| IpcError::from("backup package has no config.json".to_string()))?,
    )
    .map_err(|e| IpcError::from(format!("config.json parse: {e}")))?;
    let doc = resin_core::parse_import_doc(&config_value).map_err(IpcError::from)?;
    resin_core::validate_import_pair(&doc.strategy, &doc.ports).map_err(IpcError::from)?;

    // The same two commits, in the same order, through the same entries as
    // config_import (ADR-0069 D4): no fallible work between them, and neither
    // entry is bypassed - they own the generation bump (ADR-0058), the
    // backup-before-write (ADR-0054 section B) and the audit row (ADR-0059).
    svc.store(doc.strategy.clone()).map_err(IpcError::from)?;
    whitebox
        .apply(db, forwarder, doc.ports.clone())
        .await
        .map_err(IpcError::from)?;

    // One-way reconcile (ADR-0054 section A): Resin converges from the
    // restored whitebox. A reconcile failure does not fail the restore - the
    // whitebox is already the truth (ADR-0036) and Resin converges on the
    // next apply / boot restore.
    let client = resin_client(sidecar)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut errors: Vec<String> = Vec::new();
    let mut ports_restored: usize = 0;
    match svc
        .reconcile(&client, resolve_id_in, async {
            reconcile_ports_half(sidecar, whitebox, forwarder, now).await
        })
        .await
    {
        Ok(report) => {
            ports_restored = report.ports_restored.len();
            tracing::info!(
                platforms = doc.strategy.platforms.len(),
                ports = doc.ports.entry_ports.len(),
                ports_restored,
                "backup_restore complete (whitebox write + reconcile)"
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, "backup_restore: whitebox written; reconcile failed");
            errors.push(format!("reconcile: {e}"));
        }
    }

    // ---- L1 half: the settings store IS the L1 write entry ---------------
    // Keys present in the package are written over the live ones; keys the
    // package does not know about are left alone, so restoring an older
    // package cannot silently drop preferences a newer build added.
    let mut settings_restored = false;
    if let Some(bytes) = members.get(backup_model::SETTINGS_ENTRY) {
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|e| IpcError::from(format!("settings.json parse: {e}")))?;
        let obj = value
            .as_object()
            .ok_or_else(|| IpcError::from("settings.json must be a JSON object".to_string()))?;
        settings_apply(obj)?;
        settings_restored = true;
    }

    // ---- L3 + audit: read-only evidence, never written back --------------
    // ADR-0070 D6: derived runtime state is not replayed over live leases,
    // and the append-only audit chain is never rewritten. Both are extracted
    // next to the app so a human can inspect them, and the UI says so.
    let evidence_dir =
        data.join("restore-artifacts")
            .join(format!("{}-{}", stamp(), random_suffix()));
    let mut evidence: Vec<String> = Vec::new();
    for (path, bytes) in &members {
        let evidence_only = match backup_model::classify_member(path) {
            Some(class) => {
                backup_model::restore_action(class) == backup_model::RestoreAction::EvidenceOnly
            }
            None => false,
        };
        if !evidence_only {
            continue;
        }
        let mut target = evidence_dir.clone();
        for seg in path.split('/') {
            target.push(seg);
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| IpcError::from(e.to_string()))?;
        }
        std::fs::write(&target, bytes).map_err(|e| IpcError::from(e.to_string()))?;
        evidence.push(path.to_string());
    }
    evidence.sort();

    Ok(serde_json::json!({
        "configRestored": true,
        "settingsRestored": settings_restored,
        "platforms": doc.strategy.platforms.len(),
        "ports": doc.ports.entry_ports.len(),
        "portsRestored": ports_restored,
        "evidence": evidence,
        "evidenceDir": evidence_dir.to_string_lossy().to_string(),
        "errors": errors,
    }))
}
