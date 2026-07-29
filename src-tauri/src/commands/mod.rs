//! IPC commands exposed to the React frontend via `tauri::generate_handler!`.
//!
//! Path A (fork Resin Go sidecar): the webview talks to the Resin Go control
//! plane WRAPPED behind Rust IPC. Platform create/list/delete are FORWARDED
//! to Resin via ResinClient; the lane/lease/account/exit_ip IPC surface the old
//! self-implemented resin-core model exposed is DEPRECATED to echo/no-op because
//! the Resin forward proxy owns sticky-session + exit-ip allocation natively.
//! Each command still validates its inputs at the IPC boundary (AGENTS 7.5).

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::sidecar::SidecarHandle;
use resin_core::{ResinClient, MAX_LANES};
use resin_core::platform::Account;

const AUTHORITY_MAX_LEN: usize = 253;
const LATENCY_CAP_MS: u64 = 24 * 60 * 60 * 1000;
const KEY_MAX_LEN: usize = 4096;
const NAME_MAX_LEN: usize = 128;

fn validate_authority(authority: &str) -> Result<(), String> {
    if authority.is_empty() || authority.len() > AUTHORITY_MAX_LEN {
        return Err(format!("authority length out of range (1..={AUTHORITY_MAX_LEN})"));
    }
    if authority.bytes().any(|b| b == 0 || (b < 0x20 && b != 0x09) || b == 0x7f) {
        return Err("authority contains control characters".to_string());
    }
    Ok(())
}

fn validate_short_name(name: &str, field: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > NAME_MAX_LEN {
        return Err(format!("{field} length out of range (1..={NAME_MAX_LEN})"));
    }
    if name.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
        return Err(format!("{field} contains control characters"));
    }
    Ok(())
}

fn validate_ip(ip: &str) -> Result<(), String> {
    if ip.is_empty() || ip.len() > 253 {
        return Err("exit_ip length out of range".to_string());
    }
    if ip.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f || b == b' ') {
        return Err("exit_ip contains control/space characters".to_string());
    }
    Ok(())
}

fn resin_client(h: &SidecarHandle) -> Result<ResinClient, String> {
    ResinClient::new(&h.api_base(), h.admin_token.clone())
        .map_err(|e| format!("sidecar client: {e:?}"))
}

#[derive(Debug, Serialize)]
pub struct ReserveResult {
    pub lane: usize,
    pub lease: Option<u64>,
    pub reason: String,
}

#[tauri::command]
pub async fn gateway_reserve(
    api_key: String,
    account: String,
    authority: String,
    exit_ip: Option<String>,
) -> Result<ReserveResult, String> {
    if api_key.is_empty() || account.is_empty() {
        return Err("api_key and account must be non-empty".to_string());
    }
    if api_key.len() > KEY_MAX_LEN || account.len() > KEY_MAX_LEN {
        return Err(format!("api_key/account length out of range (1..={KEY_MAX_LEN})"));
    }
    validate_authority(&authority)?;
    if let Some(ip) = exit_ip.as_deref() {
        validate_ip(ip)?;
    }
    Ok(ReserveResult { lane: 0, lease: None, reason: "ok".into() })
}

#[tauri::command]
pub async fn gateway_release(_lease: Option<u64>) -> Result<(), String> {
    Ok(())
}

#[tauri::command]
pub async fn gateway_evict_lane(lane: usize) -> Result<(), String> {
    if lane >= MAX_LANES {
        return Err(format!("evict_lane: lane {lane} out of range (max {})", MAX_LANES - 1));
    }
    Ok(())
}

#[tauri::command]
pub async fn gateway_record_latency(authority: String, latency_ms: u64) -> Result<(), String> {
    validate_authority(&authority)?;
    let _capped = latency_ms.min(LATENCY_CAP_MS);
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct SelectResult {
    pub account: Option<String>,
    pub lane: usize,
    pub exit_ip: Option<String>,
    pub reason: String,
}

#[tauri::command]
pub async fn gateway_select_account(
    platform: String,
    api_key: String,
    authority: String,
    _weighted: Option<bool>,
) -> Result<SelectResult, String> {
    validate_short_name(&platform, "platform")?;
    if api_key.is_empty() {
        return Err("api_key must be non-empty".to_string());
    }
    validate_authority(&authority)?;
    Ok(SelectResult { account: Some(platform.clone()), lane: 0, exit_ip: None, reason: platform })
}

#[derive(Debug, Serialize)]
pub struct LaneSnapshot {
    pub lane_count: usize,
    pub busy: usize,
    pub latencies: Vec<(String, f64, u64, i8)>,
}

#[tauri::command]
pub async fn gateway_snapshot(sidecar: State<'_, SidecarHandle>) -> Result<LaneSnapshot, String> {
    let client = resin_client(&sidecar)?;
    let leases = client.active_leases().await.map_err(|e| e.to_string())?;
    let busy = sum_active_leases(&leases);
    Ok(LaneSnapshot { lane_count: MAX_LANES, busy, latencies: Vec::new() })
}

fn sum_active_leases(v: &serde_json::Value) -> usize {
    if let Some(items) = v.get("items").and_then(|i| i.as_array()) {
        return items.iter()
            .filter_map(|it| it.get("active_leases").and_then(|n| n.as_u64()))
            .map(|n| n as usize)
            .sum();
    }
    v.get("active_leases").and_then(|n| n.as_u64()).map(|n| n as usize).unwrap_or(0)
}

#[tauri::command]
pub async fn platform_add(sidecar: State<'_, SidecarHandle>, name: String) -> Result<(), String> {
    validate_short_name(&name, "platform")?;
    let client = resin_client(&sidecar)?;
    client.create_platform_from_name(&name).await.map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn platform_remove(sidecar: State<'_, SidecarHandle>, name: String) -> Result<bool, String> {
    validate_short_name(&name, "platform")?;
    let client = resin_client(&sidecar)?;
    let list = client.list_platforms().await.map_err(|e| e.to_string())?;
    let id = platform_id_for_name(&list, &name).ok_or_else(|| format!("platform not found: {name}"))?;
    client.delete_platform(&id).await.map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub async fn platform_list(sidecar: State<'_, SidecarHandle>) -> Result<Vec<String>, String> {
    let client = resin_client(&sidecar)?;
    let list = client.list_platforms().await.map_err(|e| e.to_string())?;
    Ok(platform_names(&list))
}

#[tauri::command]
pub async fn platform_snapshot(sidecar: State<'_, SidecarHandle>, name: String) -> Result<Vec<Account>, String> {
    validate_short_name(&name, "platform")?;
    let _client = resin_client(&sidecar)?;
    Ok(Vec::new())
}

#[tauri::command]
pub async fn account_add(
    sidecar: State<'_, SidecarHandle>,
    platform: String,
    id: String,
    lane: usize,
) -> Result<(), String> {
    validate_short_name(&platform, "platform")?;
    validate_short_name(&id, "account")?;
    if lane >= MAX_LANES {
        return Err(format!("lane {lane} out of range (max {})", MAX_LANES - 1));
    }
    let _client = resin_client(&sidecar)?;
    Ok(())
}

#[tauri::command]
pub async fn account_bind_ip(
    sidecar: State<'_, SidecarHandle>,
    platform: String,
    account: String,
    ip: String,
) -> Result<bool, String> {
    validate_short_name(&platform, "platform")?;
    validate_short_name(&account, "account")?;
    validate_ip(&ip)?;
    let _client = resin_client(&sidecar)?;
    Ok(true)
}

fn platform_names(v: &serde_json::Value) -> Vec<String> {
    if let Some(arr) = v.as_array() {
        arr.iter().filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(String::from)).collect()
    } else {
        Vec::new()
    }
}

fn platform_id_for_name(v: &serde_json::Value, want: &str) -> Option<String> {
    if let Some(arr) = v.as_array() {
        for p in arr {
            let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name == want {
                let id = p.get("id").and_then(|n| n.as_str()).unwrap_or("");
                if !id.is_empty() {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Subscriptions - FORWARDED to Resin via ResinClient (DESIGN.md /subscriptions).

#[tauri::command]
pub async fn subscription_add(sidecar: State<'_, SidecarHandle>, name: String, url: String) -> Result<(), String> {
    validate_short_name(&name, "subscription")?;
    if url.trim().is_empty() { return Err("subscription url must be non-empty".to_string()); }
    if url.len() > KEY_MAX_LEN { return Err("subscription url out of range".to_string()); }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("subscription url must start with http:// or https://".to_string());
    }
    let client = resin_client(&sidecar)?;
    let body = serde_json::json!({ "name": name, "source_type": "remote", "url": url });
    client.create_subscription(body).await.map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn subscription_remove(sidecar: State<'_, SidecarHandle>, name: String) -> Result<bool, String> {
    validate_short_name(&name, "subscription")?;
    let client = resin_client(&sidecar)?;
    let list = client.list_subscriptions().await.map_err(|e| e.to_string())?;
    let id = subscription_id_for_name(&list, &name).ok_or_else(|| format!("subscription not found: {name}"))?;
    client.delete_subscription(&id).await.map_err(|e| e.to_string())?;
    Ok(true)
}

#[derive(Debug, Serialize)]
pub struct SubscriptionSnapshotEntry {
    pub name: String,
    pub node_count: u64,
}

#[tauri::command]
pub async fn subscription_list(sidecar: State<'_, SidecarHandle>) -> Result<Vec<SubscriptionSnapshotEntry>, String> {
    let client = resin_client(&sidecar)?;
    let list = client.list_subscriptions().await.map_err(|e| e.to_string())?;
    Ok(subscription_snapshot(&list))
}

#[tauri::command]
pub async fn node_pool_snapshot(sidecar: State<'_, SidecarHandle>) -> Result<serde_json::Value, String> {
    let client = resin_client(&sidecar)?;
    client.node_pool_snapshot().await.map_err(|e| e.to_string())
}

fn subscription_snapshot(v: &serde_json::Value) -> Vec<SubscriptionSnapshotEntry> {
    if let Some(arr) = v.as_array() {
        arr.iter().filter_map(|p| {
            let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let node_count = p.get("node_count").and_then(|n| n.as_u64()).unwrap_or(0);
            if name.is_empty() { None } else { Some(SubscriptionSnapshotEntry { name: name.to_string(), node_count }) }
        }).collect()
    } else {
        Vec::new()
    }
}

fn subscription_id_for_name(v: &serde_json::Value, want: &str) -> Option<String> {
    if let Some(arr) = v.as_array() {
        for p in arr {
            let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name == want {
                let id = p.get("id").and_then(|n| n.as_str()).unwrap_or("");
                if !id.is_empty() { return Some(id.to_string()); }
            }
        }
    }
    None
}

#[tauri::command]
pub fn tray_refresh_labels(app: AppHandle) -> Result<(), String> {
    crate::tray::apply_labels(&app).map_err(|e| format!("tray_refresh_labels: {e:?}"))
}

#[tauri::command]
pub fn get_config_dir(app: AppHandle) -> Result<String, String> {
    match app.path().app_config_dir() {
        Ok(p) => Ok(p.to_string_lossy().into_owned()),
        Err(e) => Err(format!("app_config_dir: {e:?}")),
    }
}

#[tauri::command]
pub fn get_log_dir(app: AppHandle) -> Result<String, String> {
    match app.path().app_log_dir() {
        Ok(p) => Ok(p.to_string_lossy().into_owned()),
        Err(e) => Err(format!("app_log_dir: {e:?}")),
    }
}

// --- WebDAV backup (clash-verge-rev pattern: zip config + upload to WebDAV) ---
// Ponytail: no reqwest_dav crate — reqwest does HTTP PUT for WebDAV upload.
// The webview never sees the password; it passes through tauri-plugin-store.
// We validate the URL shape (http(s)://) and length-cap before issuing the PUT.

/// Create a zip backup of settings.json + resin state dir, return the temp path.
#[tauri::command]
pub async fn backup_create(app: AppHandle) -> Result<String, String> {
    use std::io::Write;
    let path = app.path();
    let app_data = path.app_data_dir().map_err(|e| e.to_string())?;
    let settings_path = app_data.join("settings.json");
    let resin_state = app_data.join("resin-state");
    let now = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let zip_name = format!("ai-api-route-backup-{}.zip", now);
    let zip_path = std::env::temp_dir().join(&zip_name);

    let zip_file = std::fs::File::create(&zip_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(zip_file);
    let opts = zip::write::FileOptions::default();

    // Add settings.json if it exists
    if settings_path.is_file() {
        zip.start_file("settings.json", opts).map_err(|e| e.to_string())?;
        let data = std::fs::read(&settings_path).map_err(|e| e.to_string())?;
        zip.write_all(&data).map_err(|e| e.to_string())?;
    }
    // Add resin state DB if it exists
    let state_db = resin_state.join("state.db");
    if state_db.is_file() {
        zip.start_file("resin-state/state.db", opts).map_err(|e| e.to_string())?;
        let data = std::fs::read(&state_db).map_err(|e| e.to_string())?;
        zip.write_all(&data).map_err(|e| e.to_string())?;
    }
    // Add cache DB if it exists
    let cache_db = resin_state.join("cache.db");
    if cache_db.is_file() {
        zip.start_file("resin-state/cache.db", opts).map_err(|e| e.to_string())?;
        let data = std::fs::read(&cache_db).map_err(|e| e.to_string())?;
        zip.write_all(&data).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(zip_path.to_string_lossy().to_string())
}

/// Upload a backup zip to a WebDAV server.
/// url/username/password come from tauri-plugin-store (server-trust, never webview raw).
#[tauri::command]
pub async fn backup_upload(url: String, username: String, password: String, zip_path: String) -> Result<(), String> {
    if url.trim().is_empty() { return Err("webdav url must not be empty".to_string()); }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("webdav url must start with http:// or https://".to_string());
    }
    if url.len() > 2048 { return Err("webdav url too long".to_string()); }

    let data = std::fs::read(&zip_path).map_err(|e| e.to_string())?;
    let zip_name = std::path::Path::new(&zip_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or("backup.zip".to_string());
    let webdav_url = format!("{}/{}", url.trim_end_matches('/'), zip_name);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .put(&webdav_url)
        .basic_auth(&username, Some(&password))
        .body(data)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("webdav upload failed: HTTP {}", resp.status()))
    }
}

/// List backups on the WebDAV server (PROPFIND).
#[tauri::command]
pub async fn backup_list(url: String, username: String, password: String) -> Result<Vec<String>, String> {
    if url.trim().is_empty() { return Err("webdav url must not be empty".to_string()); }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("webdav url must start with http:// or https://".to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), url.trim_end_matches('/'))
        .basic_auth(&username, Some(&password))
        .header("Depth", "1")
        .header("Content-Type", "application/xml")
        .body(r#"<?xml version="1.0"?><propfind xmlns="DAV:"><prop><displayname/></prop></propfind>"#)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("webdav PROPFIND failed: HTTP {}", resp.status()));
    }
    let body = resp.text().await.map_err(|e| e.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_max_len_is_reasonable_cap() {
        assert!(KEY_MAX_LEN >= 64);
        assert!(KEY_MAX_LEN <= 32_768);
    }

    #[test]
    fn validate_authority_accepts_normal_rejects_bad() {
        assert!(validate_authority("api.openai.com").is_ok());
        assert!(validate_authority("").is_err());
        assert!(validate_authority(&"x".repeat(AUTHORITY_MAX_LEN + 1)).is_err());
        assert!(validate_authority("a\x00b").is_err());
        assert!(validate_authority("a\x01b").is_err());
        assert!(validate_authority("a\x7fb").is_err());
        assert!(validate_authority("a\tb").is_ok());
    }

    #[test]
    fn validate_ip_accepts_normal_rejects_bad() {
        assert!(validate_ip("203.0.113.7").is_ok());
        assert!(validate_ip("::1").is_ok());
        assert!(validate_ip("").is_err());
        assert!(validate_ip(&"1".repeat(254)).is_err());
        assert!(validate_ip("127.0.0.1 x").is_err());
    }

    #[test]
    fn validate_short_name_bounds() {
        assert!(validate_short_name("openai", "platform").is_ok());
        assert!(validate_short_name("", "platform").is_err());
        assert!(validate_short_name(&"x".repeat(NAME_MAX_LEN + 1), "account").is_err());
    }

    #[test]
    fn platform_names_projects_name_field() {
        let v = json!([
            { "name": "Default", "id": "00000000-0000-0000-0000-000000000000" },
            { "name": "Platform-A", "id": "11111111-1111-1111-1111-111111111111" }
        ]);
        assert_eq!(platform_names(&v), vec!["Default".to_string(), "Platform-A".to_string()]);
    }

    #[test]
    fn platform_id_for_name_matches() {
        let v = json!([{ "name": "Foo", "id": "uuid-1" }]);
        assert_eq!(platform_id_for_name(&v, "Foo"), Some("uuid-1".to_string()));
        assert_eq!(platform_id_for_name(&v, "Bar"), None);
    }

    #[test]
    fn sum_active_leases_parses_resin_shape() {
        assert_eq!(sum_active_leases(&json!({ "items": [{ "active_leases": 7 }, { "active_leases": 5 }] })), 12);
        assert_eq!(sum_active_leases(&json!({ "active_leases": 4 })), 4);
        assert_eq!(sum_active_leases(&json!({})), 0);
    }
}
