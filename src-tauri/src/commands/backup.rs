//! backup domain IPC commands (EgressAPIKEY).
//!
//! Extracted from the former commands/mod.rs monolith by architecture-recovery
//! ticket 08: pure mechanical move - no behavior, naming, or IPC-surface change.
use tauri::{AppHandle, Manager, State};
use crate::sidecar::SidecarHandle;
use resin_core::IpcError;
use super::common::{items_arr, map_resin_error, resin_client};
use super::platform::{ALLOWED_ALLOCATION_POLICIES, ProcessRouteRule, platform_id_for_name};

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

pub fn process_route_conflict_check(
    existing: &[ProcessRouteRule],
    new_process: &str,
    new_port: u16,
) -> Result<(), String> {
    for r in existing {
        if r.target_port == new_port && new_process.trim() != r.process.trim() {
            return Err(format!(
                "conflict: port {new_port} already bound to process '{}'",
                r.process
            ));
        }
    }
    Ok(())
}

/// Phase R4: export the current platform + subscription config as JSON.
/// This is the whitebox config layer — the user can save this file, edit it,
/// and re-import it to restore or migrate their routing setup. The exported
/// JSON contains the full platform schema (name, regex_filters, region_filters,
/// allocation_policy, sticky_ttl) and subscription references (name, url).
/// It does NOT contain node data (nodes are derived from subscriptions and
/// fetched live by the Resin sidecar).
#[tauri::command]
pub async fn config_export(sidecar: State<'_, SidecarHandle>) -> Result<serde_json::Value, IpcError> {
    let client = resin_client(&sidecar)?;
    let platforms = client.list_platforms().await.map_err(|e| map_resin_error(&e.to_string()))?;
    let subscriptions = client
        .list_subscriptions()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;

    let plat_items: Vec<serde_json::Value> = items_arr(&platforms)
        .iter()
        .filter_map(|p| {
            let name = p.get("name").and_then(|n| n.as_str())?;
            if name.is_empty() { return None; }
            Some(serde_json::json!({
                "name": name,
                "regex_filters": p.get("regex_filters").cloned().unwrap_or(serde_json::Value::Null),
                "region_filters": p.get("region_filters").cloned().unwrap_or(serde_json::Value::Null),
                "allocation_policy": p.get("allocation_policy").and_then(|v| v.as_str()).unwrap_or("BALANCED"),
                "sticky_ttl": p.get("sticky_ttl").and_then(|v| v.as_str()).unwrap_or("0s"),
            }))
        })
        .collect();

    let sub_items: Vec<serde_json::Value> = items_arr(&subscriptions)
        .iter()
        .filter_map(|s| {
            let name = s.get("name").and_then(|n| n.as_str())?;
            if name.is_empty() {
                return None;
            }
            let url = s.get("url").and_then(|u| u.as_str()).unwrap_or("");
            Some(serde_json::json!({ "name": name, "url": url }))
        })
        .collect();

    Ok(serde_json::json!({
        "version": 1,
        "exported_at": chrono::Local::now().to_rfc3339(),
        "platforms": plat_items,
        "subscriptions": sub_items,
    }))
}

/// Phase R4: import a config JSON (from config_export or hand-edited).
/// Validates the structure, auto-creates a backup via backup_create, then
/// re-creates platforms and subscriptions via the Resin API. Existing
/// platforms/subscriptions with the same name are skipped (idempotent).
/// Returns a summary of what was created.
#[tauri::command]
pub async fn config_import(
    app: AppHandle,
    sidecar: State<'_, SidecarHandle>,
    config: serde_json::Value,
) -> Result<serde_json::Value, IpcError> {
    // Validate top-level structure
    let platforms = config
        .get("platforms")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "config_import: missing 'platforms' array".to_string())?;
    let subscriptions = config
        .get("subscriptions")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "config_import: missing 'subscriptions' array".to_string())?;

    // Cap input size to prevent abuse (AGENTS s7.5: 256KB max)
    let config_str = serde_json::to_string(&config).map_err(|e| IpcError::from(e.to_string()))?;
    if config_str.len() > 262_144 {
        return Err(IpcError::from("config_import: config too large (max 256KB)".to_string()));
    }

    // Auto-backup before applying (防呆: always backup before destructive change)
    let backup_path = backup_create(app.clone()).await?;

    let client = resin_client(&sidecar)?;

    // Get existing names to skip duplicates (idempotent import)
    let existing_plats = client.list_platforms().await.map_err(|e| map_resin_error(&e.to_string()))?;
    let existing_plat_names: std::collections::HashSet<String> = items_arr(&existing_plats)
        .iter()
        .filter_map(|p| {
            p.get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
        })
        .collect();

    let existing_subs = client
        .list_subscriptions()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
    let existing_sub_names: std::collections::HashSet<String> = items_arr(&existing_subs)
        .iter()
        .filter_map(|s| {
            s.get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
        })
        .collect();

    let mut platforms_created = 0u32;
    let mut platforms_skipped = 0u32;
    let mut subscriptions_created = 0u32;
    let mut subscriptions_skipped = 0u32;
    let mut errors: Vec<String> = Vec::new();

    // Create platforms
    for plat in platforms {
        let name = plat.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if name.is_empty() || name.len() > 128 {
            errors.push(format!("platform name invalid: {name}"));
            continue;
        }
        if existing_plat_names.contains(name) {
            platforms_skipped += 1;
            continue;
        }
        match client.create_platform_from_name(name).await {
            Ok(_) => {
                platforms_created += 1;
                // PATCH the platform with imported fields if any
                let mut body = serde_json::Map::new();
                if let Some(policy) = plat.get("allocation_policy").and_then(|v| v.as_str()) {
                    if ALLOWED_ALLOCATION_POLICIES.contains(&policy) {
                        body.insert(
                            "allocation_policy".to_string(),
                            serde_json::Value::String(policy.to_string()),
                        );
                    }
                }
                if let Some(filters) = plat.get("regex_filters").and_then(|v| v.as_array()) {
                    if filters.len() <= 64 {
                        let valid: Vec<serde_json::Value> = filters
                            .iter()
                            .filter(|f| {
                                f.as_str().map_or(false, |s| {
                                    s.len() <= 253
                                        && !s.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f)
                                })
                            })
                            .cloned()
                            .collect();
                        body.insert("regex_filters".to_string(), serde_json::Value::Array(valid));
                    }
                }
                if let Some(filters) = plat.get("region_filters").and_then(|v| v.as_array()) {
                    if filters.len() <= 64 {
                        let valid: Vec<serde_json::Value> = filters
                            .iter()
                            .filter(|f| {
                                f.as_str().map_or(false, |s| {
                                    s.len() <= 16
                                        && !s
                                            .bytes()
                                            .any(|b| b == 0 || b < 0x20 || b == 0x7f || b == b' ')
                                })
                            })
                            .cloned()
                            .collect();
                        body.insert(
                            "region_filters".to_string(),
                            serde_json::Value::Array(valid),
                        );
                    }
                }
                if let Some(ttl) = plat.get("sticky_ttl").and_then(|v| v.as_str()) {
                    if ttl.len() <= 32 && !ttl.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
                        body.insert(
                            "sticky_ttl".to_string(),
                            serde_json::Value::String(ttl.to_string()),
                        );
                    }
                }
                if !body.is_empty() {
                    // Resolve name->id and PATCH
                    let list = client.list_platforms().await.map_err(|e| map_resin_error(&e.to_string()))?;
                    if let Some(id) = platform_id_for_name(&list, name) {
                        if let Err(e) = client
                            .update_platform(&id, serde_json::Value::Object(body))
                            .await
                        {
                            tracing::warn!(platform = %name, error = %e.to_string(), "config_import: PATCH platform fields failed");
                            errors.push(format!("platform {name}: PATCH failed: {e}"));
                        }
                    }
                }
            }
            Err(e) => errors.push(format!("platform {name}: {e}")),
        }
    }

    // Create subscriptions (by URL — the Resin sidecar fetches nodes)
    for sub in subscriptions {
        let name = sub.get("name").and_then(|n| n.as_str()).unwrap_or("");
        let url = sub.get("url").and_then(|u| u.as_str()).unwrap_or("");
        if name.is_empty() || name.len() > 128 {
            errors.push(format!("subscription name invalid: {name}"));
            continue;
        }
        if existing_sub_names.contains(name) {
            subscriptions_skipped += 1;
            continue;
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            errors.push(format!(
                "subscription {name}: url must start with http(s)://"
            ));
            continue;
        }
        // T20-P3: POST source_type=remote directly — Resin fetches nodes via its own clash.meta UA.
        let body = serde_json::json!({
            "name": name,
            "source_type": "remote",
            "url": url,
            "update_interval": "30s",
        });
        match client.create_subscription(body).await {
            Ok(_) => subscriptions_created += 1,
            Err(e) => errors.push(format!("subscription {name}: {e}")),
        }
    }

    tracing::info!(
        platforms_created,
        platforms_skipped,
        subscriptions_created,
        subscriptions_skipped,
        error_count = errors.len(),
        "config_import complete; backup at {}",
        backup_path
    );

    Ok(serde_json::json!({
        "backup_path": backup_path,
        "platforms_created": platforms_created,
        "platforms_skipped": platforms_skipped,
        "subscriptions_created": subscriptions_created,
        "subscriptions_skipped": subscriptions_skipped,
        "errors": errors,
    }))
}
