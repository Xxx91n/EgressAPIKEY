//! backup domain IPC commands (EgressAPIKEY).
//!
//! Extracted from the former commands/mod.rs monolith by architecture-recovery
//! ticket 08: pure mechanical move - no behavior, naming, or IPC-surface change.
use tauri::{AppHandle, Manager, State};
use crate::sidecar::SidecarHandle;
use resin_core::IpcError;
use super::common::{map_resin_error, resin_client};
use resin_core::resolve_id_in;
use super::strategy::{reconcile_ports_half, strategy_service};

/// Create a zip backup of settings.json + resin state dir, return the temp path.
#[tauri::command]
pub async fn backup_create(app: AppHandle) -> Result<String, IpcError> {
    use std::io::Write;
    let path = app.path();
    let app_data = path.app_data_dir().map_err(|e| IpcError::from(e.to_string()))?;
    let settings_path = app_data.join("settings.json");
    let resin_state = app_data.join("resin-state");
    let now = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let backups_dir = app_data.join("backups");
    std::fs::create_dir_all(&backups_dir).map_err(|e| IpcError::from(e.to_string()))?;
    // Crypto-random suffix: prevents path-guessing on shared hosts and keeps
    // the backup inside the per-user app_data dir (not world-writable /tmp).
    let mut rand_bytes = [0u8; 8];
    use std::io::Read;
    match std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut rand_bytes)) {
        Ok(()) => {}
        Err(_) => {
            // Windows: no /dev/urandom. Fall back to time+pid mixing (best-effort
            // entropy; the threat model here is path-guessing, not crypto).
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
    let suffix: String = rand_bytes.iter().map(|b| format!("{:02x}", b)).collect();
    let zip_name = format!("egressapikey-backup-{}-{}.zip", now, suffix);
    let zip_path = backups_dir.join(&zip_name);

    let zip_file = std::fs::File::create(&zip_path).map_err(|e| IpcError::from(e.to_string()))?;
    let mut zip = zip::ZipWriter::new(zip_file);
    let opts = zip::write::FileOptions::default();

    // Add settings.json if it exists
    if settings_path.is_file() {
        zip.start_file("settings.json", opts)
            .map_err(|e| IpcError::from(e.to_string()))?;
        let data = std::fs::read(&settings_path).map_err(|e| IpcError::from(e.to_string()))?;
        zip.write_all(&data).map_err(|e| IpcError::from(e.to_string()))?;
    }
    // L3 private-file read exception (ADR-0050-bis): the two reads below are
    // the ONLY legislated direct reads of Resin private state — read-only
    // (never written back; L3 writes stay on the ResinClient REST seam,
    // ADR-0042 S6), scoped to state.db/cache.db, never request_logs*.db
    // (AGENTS.md §7.6), schema-blind. Ledger: ARCHITECTURE.md "Known exception".
    // Add resin state DB if it exists
    let state_db = resin_state.join("state.db");
    if state_db.is_file() {
        zip.start_file("resin-state/state.db", opts)
            .map_err(|e| IpcError::from(e.to_string()))?;
        let data = std::fs::read(&state_db).map_err(|e| IpcError::from(e.to_string()))?;
        zip.write_all(&data).map_err(|e| IpcError::from(e.to_string()))?;
    }
    // Add cache DB if it exists
    let cache_db = resin_state.join("cache.db");
    if cache_db.is_file() {
        zip.start_file("resin-state/cache.db", opts)
            .map_err(|e| IpcError::from(e.to_string()))?;
        let data = std::fs::read(&cache_db).map_err(|e| IpcError::from(e.to_string()))?;
        zip.write_all(&data).map_err(|e| IpcError::from(e.to_string()))?;
    }
    zip.finish().map_err(|e| IpcError::from(e.to_string()))?;
    Ok(zip_path.to_string_lossy().to_string())
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
    if url.trim().is_empty() {
        return Err(IpcError::from("webdav url must not be empty".to_string()));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(IpcError::from("webdav url must start with http:// or https://".to_string()));
    }
    if url.len() > 2048 {
        return Err(IpcError::from("webdav url too long".to_string()));
    }

    // Security: confine zip_path to the per-user app_data/backups dir.
    // Canonicalize both and require backups_dir to be a prefix; reject ../
    // escapes and absolute paths outside app data. Prevents a compromised
    // webview from exfiltrating arbitrary files (e.g. the Resin admin token,
    // settings.json, or system files) to an attacker-controlled WebDAV URL.
    let app_data = app.path().app_data_dir().map_err(|e| IpcError::from(e.to_string()))?;
    let backups_dir = app_data.join("backups");
    std::fs::create_dir_all(&backups_dir).map_err(|e| IpcError::from(e.to_string()))?;
    let canon_backup = std::fs::canonicalize(&backups_dir)
        .map_err(|e| IpcError::from(format!("backups dir not accessible: {e}")))?;
    let canon_zip =
        std::fs::canonicalize(&zip_path).map_err(|e| IpcError::from(format!("zip path not accessible: {e}")))?;
    if !canon_zip.starts_with(&canon_backup) {
        return Err(IpcError::from("zip path must be inside the app backups directory".to_string()));
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
        Err(IpcError::from(format!("webdav upload failed: HTTP {}", resp.status())))
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
        return Err(IpcError::from("webdav url must start with http:// or https://".to_string()));
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
        return Err(IpcError::from(format!("webdav PROPFIND failed: HTTP {}", resp.status())));
    }
    let body = resp.text().await.map_err(|e| map_resin_error(&e.to_string()))?;
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

/// Round 5 T07 / ADR-0061: export the L2 whitebox configuration as JSON.
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
    let strategy = svc.get().map_err(IpcError::from)?;
    let ports = whitebox.snapshot();
    let exported_at = chrono::Local::now().to_rfc3339();
    Ok(resin_core::build_export_doc(&strategy, &ports, &exported_at))
}

/// Round 5 T07 / ADR-0061: import a config document (from config_export).
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
    // Cap input size before any parse (AGENTS s7.5: 256KB max).
    let config_str = serde_json::to_string(&config).map_err(|e| IpcError::from(e.to_string()))?;
    if config_str.len() > 262_144 {
        return Err(IpcError::from("config_import: config too large (max 256KB)".to_string()));
    }

    // Parse + validate BOTH whitebox documents up front: any failure returns
    // a clear error and writes nothing (no partial import).
    let doc = resin_core::parse_import_doc(&config).map_err(IpcError::from)?;

    // Write the strategy half through the single sanctioned entry (ADR-0036).
    // T09: store() bumps generation and returns the landed document.
    let svc = strategy_service(&app)?;
    svc.store(doc.strategy.clone()).map_err(IpcError::from)?;

    // Write the ports half through the single sanctioned entry (ADR-0055).
    whitebox
        .apply(&db, &forwarder, doc.ports.clone())
        .await
        .map_err(IpcError::from)?;

    // Trigger the one-way reconcile (ADR-0054 §A / F2): strategy apply
    // (diff-then-skip, ADR-0057) then ports re-assert — the SAME path as
    // reconcile_now, fail-fast, whitebox always wins.
    let client = resin_client(&sidecar)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut errors: Vec<String> = Vec::new();
    match svc
        .reconcile(&client, resolve_id_in, async {
            reconcile_ports_half(sidecar.inner(), whitebox.inner(), now).await
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
