//! IPC commands exposed to the React frontend via `tauri::generate_handler!`.
//!
//! Path A (fork Resin Go sidecar): the webview talks to the Resin Go control
//! plane WRAPPED behind Rust IPC. Platform create/list/delete are FORWARDED
//! to Resin via ResinClient; the lane/lease/account/exit_ip IPC surface the old
//! self-implemented resin-core model exposed is DEPRECATED to echo/no-op because
//! the Resin forward proxy owns sticky-session + exit-ip allocation natively.
//! Each command still validates its inputs at the IPC boundary (AGENTS 7.5).

use serde::{Serialize, Deserialize};
use tauri::{AppHandle, Manager, State};

use crate::sidecar::SidecarHandle;
use resin_core::{ResinClient, MAX_LANES, fetch_clash_subscription, clash_yaml_to_proxies_block};
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

/// Extract the array from a Resin list response. Resin wraps paginated
/// collections as `{"items":[...], "total", "limit", "offset"}`; a few
/// legacy endpoints still return a bare array. Accept both so a future
/// Resin API tightening cannot silently empty the UI (P13 root cause).
fn items_arr<'a>(v: &'a serde_json::Value) -> &'a [serde_json::Value] {
    if let Some(arr) = v.get("items").and_then(|i| i.as_array()) {
        return arr.as_slice();
    }
    if let Some(arr) = v.as_array() {
        return arr.as_slice();
    }
    &[]
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
    /// Per-platform active lease counts (platform name, active_count).
    /// The TS side uses this to render entry boxes with the real active
    /// lease occupancy instead of a placeholder 0. Resin is the source.
    pub per_platform_active: Vec<(String, usize)>,
}

#[tauri::command]
pub async fn gateway_snapshot(sidecar: State<'_, SidecarHandle>) -> Result<LaneSnapshot, String> {
    let client = resin_client(&sidecar)?;
    let leases = client.active_leases().await.map_err(|e| e.to_string())?;
    let busy = sum_active_leases(&leases);
    // Issue 4+7: pull the Resin /platforms list once per snapshot so we can
    // resolve lease items' platform_id back to the user-visible platform NAME
    // that the Topology canvas shows as an Entry box (the leases endpoint only
    // exposes platform_id as a UUID; Resin owns the UUID->name mapping). On
    // any failure we fall back to the raw platform_id so the canvas still
    // renders a real entry instead of dropping the lease count.
    let platforms = client.list_platforms().await.unwrap_or_else(|_| serde_json::json!([]));
    let per_platform_active = per_platform_active_from_leases(&leases, &platforms);
    Ok(LaneSnapshot {
        lane_count: MAX_LANES,
        busy,
        latencies: Vec::new(),
        per_platform_active,
    })
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

/// Build per-platform (name, active_count) from a /metrics/realtime/leases
/// response by joining each lease item's platform_id to the platforms list.
/// If a lease item has no platform_id (Resin's "Default" platform emits
/// the empty string instead of the Default platform UUID), we attribute
/// the active count to the Default platform if it exists in the platforms
/// list; otherwise we surface it under the raw id so the count is not lost.
fn per_platform_active_from_leases(
    leases: &serde_json::Value,
    platforms: &serde_json::Value,
) -> Vec<(String, usize)> {
    let parr = items_arr(platforms);
    let id_to_name: std::collections::HashMap<String, String> = parr
        .iter()
        .filter_map(|p| {
            let id = p.get("id").and_then(|i| i.as_str())?.trim().to_string();
            let name = p.get("name").and_then(|n| n.as_str())?.trim().to_string();
            if id.is_empty() || name.is_empty() { None } else { Some((id, name)) }
        })
        .collect();
    let default_name = parr
        .iter()
        .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("Default"))
        .and_then(|p| p.get("name").and_then(|n| n.as_str()).map(String::from));
    let items = match leases.get("items").and_then(|i| i.as_array()) {
        Some(arr) => arr,
        None => return Vec::new(),
    };
    let mut acc: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for it in items {
        let raw_id = it.get("platform_id").and_then(|i| i.as_str()).unwrap_or("").trim().to_string();
        let active = it.get("active_leases").and_then(|n| n.as_u64()).map(|n| n as usize).unwrap_or(0);
        let resolved = if raw_id.is_empty() {
            // Resin emits "Default" platform leases with platform_id = ""
            default_name.clone()
        } else {
            id_to_name.get(&raw_id).cloned().or(Some(raw_id.clone()))
        };
        if let Some(name) = resolved {
            *acc.entry(name).or_insert(0) += active;
        }
    }
    acc.into_iter().collect()
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
    items_arr(v)
        .iter()
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(String::from))
        .collect()
}

fn platform_id_for_name(v: &serde_json::Value, want: &str) -> Option<String> {
    for p in items_arr(v) {
        let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if name == want {
            let id = p.get("id").and_then(|n| n.as_str()).unwrap_or("");
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Subscriptions - FORWARDED to Resin via ResinClient (DESIGN.md /subscriptions).

// ---- Process routing (issue 3+10) ----
// Per-process -> lane routing rules stored server-side in tauri-plugin-store.
// The Rust side rejects lane collisions (the same target lane already bound
// to a different process in a live rule) BEFORE we record the rule. This is the
// user-visible "conflict detect + refuse + clear toast" surface. The auth
// boundary stays on the OS side (server-trusted store); per-request auth lives
// in the Resin sidecar proxy. We only own the routing rule registry here.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessRouteRule {
    pub process: String,
    pub target_lane: usize,
}

#[tauri::command]
pub async fn process_route_add(app: AppHandle, process: String, target_lane: usize) -> Result<(), String> {
    validate_short_name(&process, "process")?;
    if target_lane >= MAX_LANES {
        return Err(format!("process_route_add: lane {target_lane} out of range (max {})", MAX_LANES - 1));
    }
    let store = tauri_plugin_store::StoreExt::store(&app, "settings.json").map_err(|e| format!("store: {e:?}"))?;
    let mut rules: Vec<ProcessRouteRule> = store.get("processRoutes")
        .and_then(|v| serde_json::from_value::<Vec<ProcessRouteRule>>(v).ok())
        .unwrap_or_default();
    // conflict detect via the extracted helper (unit-testable)
    process_route_conflict_check(&rules, &process, target_lane)?;
    if let Some(slot) = rules.iter_mut().find(|r| r.process.trim() == process.trim()) {
        slot.target_lane = target_lane;
    } else {
        rules.push(ProcessRouteRule { process: process.trim().to_string(), target_lane });
    }
    store.set("processRoutes", serde_json::to_value(&rules).map_err(|e| format!("serialize: {e}"))?);
    store.save().map_err(|e| format!("store save: {e:?}"))?;
    Ok(())
}

#[tauri::command]
pub async fn process_route_remove(app: AppHandle, process: String) -> Result<bool, String> {
    validate_short_name(&process, "process")?;
    let store = tauri_plugin_store::StoreExt::store(&app, "settings.json").map_err(|e| format!("store: {e:?}"))?;
    let mut rules: Vec<ProcessRouteRule> = store.get("processRoutes")
        .and_then(|v| serde_json::from_value::<Vec<ProcessRouteRule>>(v).ok())
        .unwrap_or_default();
    let before = rules.len();
    rules.retain(|r| r.process.trim() != process.trim());
    if rules.len() != before {
        store.set("processRoutes", serde_json::to_value(&rules).map_err(|e| format!("serialize: {e}"))?);
        store.save().map_err(|e| format!("store save: {e:?}"))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
pub async fn process_route_list(app: AppHandle) -> Result<Vec<ProcessRouteRule>, String> {
    let store = tauri_plugin_store::StoreExt::store(&app, "settings.json").map_err(|e| format!("store: {e:?}"))?;
    Ok(store.get("processRoutes")
        .and_then(|v| serde_json::from_value::<Vec<ProcessRouteRule>>(v).ok())
        .unwrap_or_default())
}

#[tauri::command]
pub async fn subscription_add(sidecar: State<'_, SidecarHandle>, name: String, url: String) -> Result<(), String> {
    validate_short_name(&name, "subscription")?;
    if url.trim().is_empty() { return Err("subscription url must be non-empty".to_string()); }
    if url.len() > KEY_MAX_LEN { return Err("subscription url out of range".to_string()); }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("subscription url must start with http:// or https://".to_string());
    }
    let client = resin_client(&sidecar)?;

    // P13 B4: Resin's own remote-fetch uses a default HTTP UA that many
    // subscription providers (the user's test host included) reject with 403.
    // We fetch the Clash YAML ourselves with a clash-family UA, convert the
    // flow-style `proxies:` segment into block-style (Resin's Go YAML parser
    // chokes on flow-style inline mappings), and POST it as a local
    // subscription so Resin parses the nodes we already fetched. The user's
    // url is retained as metadata so the UI can still show the source.
    tracing::info!(subscription = %name, url = %url, "subscription_add: fetching clash yaml");
    let yaml = fetch_clash_subscription(&url).await
        .map_err(|e| { tracing::warn!(error = ?e, "subscription_add: fetch failed"); e.to_string() })?;
    tracing::info!(bytes = yaml.len(), "subscription_add: fetched yaml, converting to proxies-only block");
    let block = clash_yaml_to_proxies_block(&yaml)
        .map_err(|e| { tracing::warn!(error = ?e, "subscription_add: convert failed"); e.to_string() })?;
    tracing::info!(block_bytes = block.len(), "subscription_add: posting local subscription to Resin");

    // 30s update_interval so Resin's scheduler parses the local content on the
    // first tick (seconds, not the default 5m). Resin does not expose a
    // force-refresh endpoint.
    // P13 B4: Resin rejects `url` when source_type == "local"
    // (INVALID_ARGUMENT "url is not allowed for local subscription").
    // The user-visible origin is preserved in the Resin subscription name;
    // we do NOT pass url in the local-body. (Handoff summary was wrong here;
    // live probe caught it.)
    let body = serde_json::json!({
        "name": name,
        "source_type": "local",
        "content": block,
        "update_interval": "30s",
    });
    match client.create_subscription(body).await {
        Ok(v) => {
            tracing::info!(?v, "subscription_add: Resin accepted subscription");
            Ok(())
        }
        Err(e) => {
            tracing::warn!(error = ?e, "subscription_add: Resin POST failed");
            Err(e.to_string())
        }
    }
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

// Phase R1: the topology canvas hot-switch and the node-pool tab need the
// full platform schema (not just names) and the node list. These forward to
// Resin with the same input-validation discipline as the other commands.

/// The allocation_policy values Resin v1.1.2 actually accepts (probed
/// 2026-07-31). The IPC layer rejects anything else before reaching Resin.
const ALLOWED_ALLOCATION_POLICIES: &[&str] =
    &["BALANCED", "PREFER_LOW_LATENCY", "PREFER_IDLE_IP"];

/// PATCH a platform's fields (allocation_policy, regex_filters, sticky_ttl).
/// The webview identifies the platform by NAME; we resolve name->id then
/// PATCH. Only the provided fields are sent; null/absent fields are omitted
/// so Resin keeps its current value.
#[tauri::command]
pub async fn platform_update(
    sidecar: State<'_, SidecarHandle>,
    name: String,
    allocation_policy: Option<String>,
    regex_filters: Option<Vec<String>>,
    sticky_ttl: Option<String>,
) -> Result<serde_json::Value, String> {
    validate_short_name(&name, "platform")?;
    let client = resin_client(&sidecar)?;
    // Resolve name -> id (same pattern as platform_remove).
    let list = client.list_platforms().await.map_err(|e| e.to_string())?;
    let id = platform_id_for_name(&list, &name)
        .ok_or_else(|| format!("platform not found: {name}"))?;

    // Build the PATCH body with only the fields the caller provided. Validate
    // each at the IPC boundary (AGENTS 7.5) so a hostile webview cannot send
    // an unsupported policy or an oversized filter to Resin.
    let mut body = serde_json::Map::new();
    if let Some(ref policy) = allocation_policy {
        if !ALLOWED_ALLOCATION_POLICIES.contains(&policy.as_str()) {
            return Err(format!(
                "allocation_policy must be one of {:?}",
                ALLOWED_ALLOCATION_POLICIES
            ));
        }
        body.insert("allocation_policy".to_string(), serde_json::Value::String(policy.clone()));
    }
    if let Some(ref filters) = regex_filters {
        if filters.len() > 64 {
            return Err("regex_filters: too many entries (max 64)".to_string());
        }
        let arr: Vec<serde_json::Value> = filters
            .iter()
            .map(|f| {
                // Cap each filter at 253 chars (DNS-host scale) and reject
               // control chars / NUL.
                if f.len() > 253 || f.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
                   serde_json::Value::Null
                } else {
                    serde_json::Value::String(f.clone())
                }
            })
            .filter(|v| !v.is_null())
            .collect();
        body.insert("regex_filters".to_string(), serde_json::Value::Array(arr));
    }
    if let Some(ref ttl) = sticky_ttl {
       // Go duration string; cap length to prevent abuse.
        if ttl.len() > 32 || ttl.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
            return Err("sticky_ttl: invalid (max 32 chars, no control)".to_string());
        }
        body.insert("sticky_ttl".to_string(), serde_json::Value::String(ttl.clone()));
    }
    if body.is_empty() {
        return Err("platform_update: no fields to update".to_string());
    }
    client
        .update_platform(&id, serde_json::Value::Object(body))
        .await
        .map_err(|e| e.to_string())
}

/// GET /api/v1/nodes - return the full node list (the "C category" ip/ip
/// channels) as raw JSON. The frontend renders egress IPs, health, protocol.
#[tauri::command]
pub async fn node_list(sidecar: State<'_, SidecarHandle>) -> Result<serde_json::Value, String> {
    let client = resin_client(&sidecar)?;
    client.list_nodes().await.map_err(|e| e.to_string())
}

fn subscription_snapshot(v: &serde_json::Value) -> Vec<SubscriptionSnapshotEntry> {
    items_arr(v)
        .iter()
        .filter_map(|p| {
            let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let node_count = p.get("node_count").and_then(|n| n.as_u64()).unwrap_or(0);
            if name.is_empty() { None } else { Some(SubscriptionSnapshotEntry { name: name.to_string(), node_count }) }
        })
        .collect()
}

fn subscription_id_for_name(v: &serde_json::Value, want: &str) -> Option<String> {
    for p in items_arr(v) {
        let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if name == want {
            let id = p.get("id").and_then(|n| n.as_str()).unwrap_or("");
            if !id.is_empty() { return Some(id.to_string()); }
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
    let backups_dir = app_data.join("backups");
    std::fs::create_dir_all(&backups_dir).map_err(|e| e.to_string())?;
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
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                *b = (s >> 56) as u8;
            }
        }
    }
    let suffix: String = rand_bytes.iter().map(|b| format!("{:02x}", b)).collect();
    let zip_name = format!("ai-api-route-backup-{}-{}.zip", now, suffix);
    let zip_path = backups_dir.join(&zip_name);

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
pub async fn backup_upload(app: AppHandle, url: String, username: String, password: String, zip_path: String) -> Result<(), String> {
    if url.trim().is_empty() { return Err("webdav url must not be empty".to_string()); }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("webdav url must start with http:// or https://".to_string());
    }
    if url.len() > 2048 { return Err("webdav url too long".to_string()); }

    // Security: confine zip_path to the per-user app_data/backups dir.
    // Canonicalize both and require backups_dir to be a prefix; reject ../
    // escapes and absolute paths outside app data. Prevents a compromised
    // webview from exfiltrating arbitrary files (e.g. the Resin admin token,
    // settings.json, or system files) to an attacker-controlled WebDAV URL.
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let backups_dir = app_data.join("backups");
    std::fs::create_dir_all(&backups_dir).map_err(|e| e.to_string())?;
    let canon_backup = std::fs::canonicalize(&backups_dir)
        .map_err(|e| format!("backups dir not accessible: {e}"))?;
    let canon_zip = std::fs::canonicalize(&zip_path)
        .map_err(|e| format!("zip path not accessible: {e}"))?;
    if !canon_zip.starts_with(&canon_backup) {
        return Err("zip path must be inside the app backups directory".to_string());
    }
    if !canon_zip.is_file() {
        return Err("zip path is not a file".to_string());
    }

    let data = std::fs::read(&canon_zip).map_err(|e| e.to_string())?;
    let zip_name = canon_zip
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "backup.zip".to_string());
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


// Pure helper: returns Err(msg) if adding {process, target_lane} would
// conflict with an existing rule (same target lane, different process).
// Extracted for unit testing without an AppHandle.
pub fn process_route_conflict_check(
    existing: &[ProcessRouteRule],
    new_process: &str,
    new_lane: usize,
) -> Result<(), String> {
    for r in existing {
        if r.target_lane == new_lane && new_process.trim() != r.process.trim() {
            return Err(format!("conflict: lane {new_lane} already bound to process '{}'", r.process));
        }
    }
    Ok(())
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
    #[test]
    fn process_route_conflict_rejects_same_lane_different_process() {
        let existing = vec![
            ProcessRouteRule { process: "ollama".to_string(), target_lane: 3 },
        ];
        // same process + same lane -> ok (update path)
        assert!(process_route_conflict_check(&existing, "ollama", 3).is_ok());
        // different process, same lane -> conflict
        assert!(process_route_conflict_check(&existing, "openai", 3).is_err());
        // different process, different lane -> ok
        assert!(process_route_conflict_check(&existing, "openai", 4).is_ok());
    }

    #[test]
    fn per_platform_active_resolves_uuid_to_name() {
        // Default platform + one custom platform; Default emits platform_id = ""
        let platforms = json!([
            { "name": "Default", "id": "00000000-0000-0000-0000-000000000000" },
            { "name": "OpenAI",   "id": "11111111-1111-1111-1111-111111111111" },
        ]);
        let leases = json!({
            "items": [
                { "platform_id": "", "active_leases": 4, "ts": "x" },
                { "platform_id": "11111111-1111-1111-1111-111111111111", "active_leases": 2, "ts": "x" },
            ],
            "step_seconds": 5,
        });
        let got = per_platform_active_from_leases(&leases, &platforms);
        let map: std::collections::HashMap<String, usize> = got.into_iter().collect();
        assert_eq!(map.get("Default"), Some(&4));
        assert_eq!(map.get("OpenAI"), Some(&2));
    }

    #[test]
    fn per_platform_active_falls_back_to_raw_id_when_unknown() {
        // Lease for a UUID not present in the platforms list; we surface the raw
        // UUID string instead of dropping the count so the canvas still renders.
        let platforms = json!([ { "name": "Default", "id": "00000000-0000-0000-0000-000000000000" } ]);
        let leases = json!({
            "items": [ { "platform_id": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "active_leases": 7 } ],
        });
        let got = per_platform_active_from_leases(&leases, &platforms);
        assert!(got.iter().any(|(n, c)| n == "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa" && *c == 7));
    }

    /// P13 B6/B4: Resin wraps list responses as `{"items":[...]}`. The old
    /// code used `v.as_array()` which always returned None for that shape,
    /// so platform_list / subscription_list returned empty even with live data.
    /// These tests pin both the bare-array back-compat path and the items path.
    #[test]
    fn items_arr_accepts_resin_items_wrapper() {
        let v = json!({ "items": [{ "name": "a" }, { "name": "b" }], "total": 2 });
        let arr = items_arr(&v);
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["name"], "a");
    }

    #[test]
    fn items_arr_accepts_bare_array_back_compat() {
        let v = json!([{ "name": "x" }]);
        assert_eq!(items_arr(&v).len(), 1);
    }

    #[test]
    fn items_arr_returns_empty_for_non_list_object() {
        let v = json!({ "active_leases": 4 });
        assert!(items_arr(&v).is_empty());
    }

    #[test]
    fn platform_names_reads_resin_items_wrapper() {
        let v = json!({
            "items": [
                { "id": "uuid-1", "name": "Default" },
                { "id": "uuid-2", "name": "OpenAI" },
            ],
            "total": 2, "limit": 50, "offset": 0,
        });
        assert_eq!(platform_names(&v), vec!["Default".to_string(), "OpenAI".to_string()]);
    }

    #[test]
    fn platform_id_for_name_reads_resin_items_wrapper() {
        let v = json!({
            "items": [ { "id": "uuid-9", "name": "Anthropic" } ],
            "total": 1,
        });
        assert_eq!(platform_id_for_name(&v, "Anthropic"), Some("uuid-9".to_string()));
        assert_eq!(platform_id_for_name(&v, "Missing"), None);
    }

    #[test]
    fn subscription_snapshot_reads_resin_items_wrapper_with_node_count() {
        let v = json!({
            "items": [
                { "id": "s1", "name": "sub-a", "node_count": 33 },
                { "id": "s2", "name": "sub-b", "node_count": 0 },
            ],
            "total": 2, "limit": 50, "offset": 0,
        });
        let snap = subscription_snapshot(&v);
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].name, "sub-a");
        assert_eq!(snap[0].node_count, 33);
        assert_eq!(snap[1].node_count, 0);
    }

    #[test]
    fn subscription_id_for_name_reads_resin_items_wrapper() {
        let v = json!({
            "items": [ { "id": "sub-uuid-1", "name": "main" } ],
            "total": 1,
        });
        assert_eq!(subscription_id_for_name(&v, "main"), Some("sub-uuid-1".to_string()));
        assert_eq!(subscription_id_for_name(&v, "nope"), None);
    }
}
