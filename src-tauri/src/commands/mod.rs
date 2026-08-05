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
use resin_core::DbPool;
use resin_core::{ResinClient, MAX_LANES, fetch_clash_subscription, clash_yaml_to_proxies_block};

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

/// Phase R2: return the full platform objects (not just names) so the topology
/// canvas can render regex_filters, region_filters, allocation_policy,
/// routable_node_count. Returns raw JSON; the frontend parses it.
#[tauri::command]
pub async fn platform_list_full(sidecar: State<'_, SidecarHandle>) -> Result<serde_json::Value, String> {
    let client = resin_client(&sidecar)?;
    client.list_platforms().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn platform_snapshot(sidecar: State<'_, SidecarHandle>, name: String) -> Result<serde_json::Value, String> {
    validate_short_name(&name, "platform")?;
    let client = resin_client(&sidecar)?;
    // Resolve platform name -> id, then fetch that platform is routable node list
    // (Resin DESIGN.md: GET /nodes?platform_id=<id> filters to the platform routable set).
    let list = client.list_platforms().await.map_err(|e| e.to_string())?;
    match platform_id_for_name(&list, &name) {
        Some(id) => client.list_nodes_for_platform(&id).await.map_err(|e| e.to_string()),
        None => Ok(serde_json::json!({"items":[], "total":0, "limit":500, "offset":0})),
    }
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
    pub healthy_node_count: u64,
    /// Resin `last_error` (empty string when fetch succeeded). Surfaced so
    /// the GUI can show WHY node_count is 0 instead of a mute zero.
    pub last_error: String,
    /// Resin `last_checked` RFC3339 timestamp (empty when never checked).
    pub last_checked: String,
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

/// PATCH a platform's fields (allocation_policy, regex_filters, region_filters, sticky_ttl).
/// The webview identifies the platform by NAME; we resolve name->id then
/// PATCH. Only the provided fields are sent; null/absent fields are omitted
/// so Resin keeps its current value.
#[tauri::command]
pub async fn platform_update(
    sidecar: State<'_, SidecarHandle>,
    name: String,
    allocation_policy: Option<String>,
    regex_filters: Option<Vec<String>>,
    region_filters: Option<Vec<String>>,
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
    // region_filters: lowercase ISO 3166-1 alpha-2 codes ("hk","us","jp") or
    // negation ("!hk"). These select which node regions the platform routes to
    // — this is the B->C binding mechanism for the topology canvas.
    if let Some(ref filters) = region_filters {
        if filters.len() > 64 {
            return Err("region_filters: too many entries (max 64)".to_string());
        }
        let arr: Vec<serde_json::Value> = filters
            .iter()
            .map(|f| {
                if f.len() > 16 || f.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f || b == b' ') {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String(f.clone())
                }
            })
            .filter(|v| !v.is_null())
            .collect();
        body.insert("region_filters".to_string(), serde_json::Value::Array(arr));
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

/// POST /api/v1/platforms with the full create schema (P21 Milestone B).
/// The webview identifies the platform by a fully-formed body; we validate the
/// name (required) and any obviously-hostile fields at the IPC boundary per
/// AGENTS s7.5. allocation_policy is validated against the live-probed enum.
/// The Rust side never guesses missing fields - Resin applies defaults per
/// RESIN_DEFAULT_PLATFORM_* env when a field is omitted.
#[tauri::command]
pub async fn platform_create_with_fields(
    sidecar: State<'_, SidecarHandle>,
    body: serde_json::Value,
) -> Result<serde_json::Value, String> {
    // body must be a JSON object with a non-empty "name".
    let obj = body.as_object()
        .ok_or("platform_create_with_fields: body must be a JSON object")?;
    let name = obj.get("name").and_then(|v| v.as_str())
        .ok_or("platform_create_with_fields: missing 'name' field")?;
    validate_short_name(name, "platform")?;
    // If allocation_policy is present, must be one of the allowed enum.
    if let Some(policy) = obj.get("allocation_policy").and_then(|v| v.as_str()) {
        if !ALLOWED_ALLOCATION_POLICIES.contains(&policy) {
            return Err(format!(
                "allocation_policy must be one of {:?}",
                ALLOWED_ALLOCATION_POLICIES
            ));
        }
    }
    // If regex_filters present, cap count + per-entry length (mirrors platform_update).
    if let Some(arr) = obj.get("regex_filters").and_then(|v| v.as_array()) {
        if arr.len() > 64 {
            return Err("regex_filters: too many entries (max 64)".to_string());
        }
        for f in arr {
            if let Some(s) = f.as_str() {
                if s.len() > 253 || s.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
                    return Err("regex_filters: entry invalid (max 253 chars, no control)".to_string());
                }
            }
        }
    }
    // If region_filters present, cap each at 16 chars (ISO 3166-1 alpha-2 + negation).
    if let Some(arr) = obj.get("region_filters").and_then(|v| v.as_array()) {
        if arr.len() > 64 {
            return Err("region_filters: too many entries (max 64)".to_string());
        }
        for r in arr {
            if let Some(s) = r.as_str() {
                if s.len() > 16 || s.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f || b == b' ') {
                    return Err("region_filter invalid (max 16, no control/space)".to_string());
                }
            }
        }
    }
    // If sticky_ttl present, cap at 32 chars + no control (mirrors platform_update).
    if let Some(ttl) = obj.get("sticky_ttl").and_then(|v| v.as_str()) {
        if ttl.len() > 32 || ttl.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
            return Err("sticky_ttl: invalid (max 32 chars, no control)".to_string());
        }
    }
    let client = resin_client(&sidecar)?;
    client.create_platform_with_fields(body).await.map_err(|e| e.to_string())
}

/// GET /api/v1/platforms/{id}/leases - the live leases on a platform, used by
/// the Milestone B right pane to show which accounts are already bound to an
/// exit IP on each platform. We resolve the platform name -> id (the webview
/// only knows the user-visible name) and forward to ResinClient.
#[tauri::command]
pub async fn platform_leases(
    sidecar: State<'_, SidecarHandle>,
    name: String,
) -> Result<serde_json::Value, String> {
    validate_short_name(&name, "platform")?;
    let client = resin_client(&sidecar)?;
    let list = client.list_platforms().await.map_err(|e| e.to_string())?;
    let id = platform_id_for_name(&list, &name)
        .ok_or_else(|| format!("platform not found: {name}"))?;
    client.platform_leases(&id).await.map_err(|e| e.to_string())
}

fn subscription_snapshot(v: &serde_json::Value) -> Vec<SubscriptionSnapshotEntry> {
    items_arr(v)
        .iter()
        .filter_map(|p| {
            let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let node_count = p.get("node_count").and_then(|n| n.as_u64()).unwrap_or(0);
            if name.is_empty() { None } else {
                let healthy_node_count = p.get("healthy_node_count").and_then(|n| n.as_u64()).unwrap_or(0);
                let last_error = p.get("last_error").and_then(|n| n.as_str()).unwrap_or("").to_string();
                let last_checked = p.get("last_checked").and_then(|n| n.as_str()).unwrap_or("").to_string();
                Some(SubscriptionSnapshotEntry { name: name.to_string(), node_count, healthy_node_count, last_error, last_checked })
            }
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
    let zip_name = format!("egressapikey-backup-{}-{}.zip", now, suffix);
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


/// Phase R4: export the current platform + subscription config as JSON.
/// This is the whitebox config layer — the user can save this file, edit it,
/// and re-import it to restore or migrate their routing setup. The exported
/// JSON contains the full platform schema (name, regex_filters, region_filters,
/// allocation_policy, sticky_ttl) and subscription references (name, url).
/// It does NOT contain node data (nodes are derived from subscriptions and
/// fetched live by the Resin sidecar).
#[tauri::command]
pub async fn config_export(sidecar: State<'_, SidecarHandle>) -> Result<serde_json::Value, String> {
    let client = resin_client(&sidecar)?;
    let platforms = client.list_platforms().await.map_err(|e| e.to_string())?;
    let subscriptions = client.list_subscriptions().await.map_err(|e| e.to_string())?;

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
                "sticky_ttl": p.get("sticky_ttl").and_then(|v| v.as_str()).unwrap_or("168h0m0s"),
            }))
        })
        .collect();

    let sub_items: Vec<serde_json::Value> = items_arr(&subscriptions)
        .iter()
        .filter_map(|s| {
            let name = s.get("name").and_then(|n| n.as_str())?;
            if name.is_empty() { return None; }
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
) -> Result<serde_json::Value, String> {
    // Validate top-level structure
    let platforms = config.get("platforms")
        .and_then(|v| v.as_array())
        .ok_or("config_import: missing 'platforms' array")?;
    let subscriptions = config.get("subscriptions")
        .and_then(|v| v.as_array())
        .ok_or("config_import: missing 'subscriptions' array")?;

    // Cap input size to prevent abuse (AGENTS s7.5: 256KB max)
    let config_str = serde_json::to_string(&config).map_err(|e| e.to_string())?;
    if config_str.len() > 262_144 {
        return Err("config_import: config too large (max 256KB)".to_string());
    }

    // Auto-backup before applying (防呆: always backup before destructive change)
    let backup_path = backup_create(app.clone()).await?;

    let client = resin_client(&sidecar)?;

    // Get existing names to skip duplicates (idempotent import)
    let existing_plats = client.list_platforms().await.map_err(|e| e.to_string())?;
    let existing_plat_names: std::collections::HashSet<String> = items_arr(&existing_plats)
        .iter()
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
        .collect();

    let existing_subs = client.list_subscriptions().await.map_err(|e| e.to_string())?;
    let existing_sub_names: std::collections::HashSet<String> = items_arr(&existing_subs)
        .iter()
        .filter_map(|s| s.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
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
                        body.insert("allocation_policy".to_string(), serde_json::Value::String(policy.to_string()));
                    }
                }
                if let Some(filters) = plat.get("regex_filters").and_then(|v| v.as_array()) {
                    if filters.len() <= 64 {
                        let valid: Vec<serde_json::Value> = filters.iter()
                            .filter(|f| f.as_str().map_or(false, |s| s.len() <= 253 && !s.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f)))
                            .cloned()
                            .collect();
                        body.insert("regex_filters".to_string(), serde_json::Value::Array(valid));
                    }
                }
                if let Some(filters) = plat.get("region_filters").and_then(|v| v.as_array()) {
                    if filters.len() <= 64 {
                        let valid: Vec<serde_json::Value> = filters.iter()
                            .filter(|f| f.as_str().map_or(false, |s| s.len() <= 16 && !s.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f || b == b' ')))
                            .cloned()
                            .collect();
                        body.insert("region_filters".to_string(), serde_json::Value::Array(valid));
                    }
                }
                if let Some(ttl) = plat.get("sticky_ttl").and_then(|v| v.as_str()) {
                    if ttl.len() <= 32 && !ttl.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
                        body.insert("sticky_ttl".to_string(), serde_json::Value::String(ttl.to_string()));
                    }
                }
                if !body.is_empty() {
                    // Resolve name->id and PATCH
                    let list = client.list_platforms().await.map_err(|e| e.to_string())?;
                    if let Some(id) = platform_id_for_name(&list, name) {
                        let _ = client.update_platform(&id, serde_json::Value::Object(body)).await;
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
            errors.push(format!("subscription {name}: url must start with http(s)://"));
            continue;
        }
        // Use the same local-fetch path as subscription_add
        match fetch_clash_subscription(url).await {
            Ok(yaml) => {
                match clash_yaml_to_proxies_block(&yaml) {
                    Ok(block) => {
                        let body = serde_json::json!({
                            "name": name,
                            "source_type": "local",
                            "content": block,
                            "url": url,
                            "update_interval": "30s",
                        });
                        match client.create_subscription(body).await {
                            Ok(_) => subscriptions_created += 1,
                            Err(e) => errors.push(format!("subscription {name}: {e}")),
                        }
                    }
                    Err(e) => errors.push(format!("subscription {name} convert: {e}")),
                }
            }
            Err(e) => errors.push(format!("subscription {name} fetch: {e}")),
        }
    }

    tracing::info!(
        platforms_created, platforms_skipped, subscriptions_created, subscriptions_skipped,
        error_count = errors.len(),
        "config_import complete; backup at {}", backup_path
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

/// One active lease row from Resin /api/v1/metrics/realtime/leases, projected
/// for the GUI Topology lease panel. Mirrors the items-wrapper Resin returns.
#[derive(Debug, Clone, Serialize)]
pub struct LeaseEntry {
    /// Platform UUID (Resin's internal id). Empty = Default platform.
    pub platform_id: String,
    /// The X-Resin-Account the interceptor injected (the A4-3 identity).
    pub account: String,
    /// Brand fields Resin surfaces per-lease when it has them.
    pub egress_ip: String,
    pub node_tag: String,
    pub target_domain: String,
    pub ts: String,
}

/// Live active lease map from the Resin sidecar. The GUI polls this alongside
/// platform_list + node_list in the Topology sync loop and renders a per-platform
/// lease chip showing "(account short): egress_ip". Used by A4-3 to prove the
/// (key, endpoint) -> distinct egress IP contract is live in the GUI, not just
/// prose.
#[tauri::command]
pub async fn lease_map(sidecar: State<'_, SidecarHandle>) -> Result<Vec<LeaseEntry>, String> {
    let client = resin_client(&sidecar)?;
    let raw = client.active_leases().await.map_err(|e| e.to_string())?;
    // Resin returns {"items":[{active_leases:N,"ts":"...","platform_id":""}]}
    // or a bare array. We use the shared items_arr helper to be robust.
    let items = items_arr(&raw);
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        let platform_id = it.get("platform_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let account = it.get("account")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let egress_ip = it.get("egress_ip")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let node_tag = it.get("node_tag")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let target_domain = it.get("target_domain")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let ts = it.get("ts")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        out.push(LeaseEntry {
            platform_id,
            account,
            egress_ip,
            node_tag,
            target_domain,
            ts,
        });
    }
    Ok(out)
}


/// Validate an entry-port mapping before touching DB / listeners.
fn validate_port_mapping(port: u16, protocol: &str, platform_name: &str, account: &str, label: &str) -> Result<(), String> {
    use resin_core::{MAX_ENTRY_PORTS, MIN_USER_PORT};
    if port < MIN_USER_PORT {
        return Err(format!("port {port} is privileged (< {MIN_USER_PORT})"));
    }
    let proto = protocol.trim().to_ascii_lowercase();
    if proto != "socks5" && proto != "http" {
        return Err("protocol must be socks5 or http".into());
    }
    validate_short_name(platform_name, "platform_name")?;
    // account + label optional but length/control capped
    if account.len() > NAME_MAX_LEN || account.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
        return Err("account invalid".into());
    }
    if label.len() > NAME_MAX_LEN || label.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
        return Err("label invalid".into());
    }
    // Resin Platform.Account forbids these chars in either side
    let forbidden = |s: &str| s.chars().any(|ch| ".:/\\@?#%~ ".contains(ch));
    if forbidden(platform_name) {
        return Err("platform_name contains Resin-forbidden chars".into());
    }
    if !account.is_empty() && forbidden(account) {
        return Err("account contains Resin-forbidden chars".into());
    }
    let _ = MAX_ENTRY_PORTS; // capacity enforced at reload
    Ok(())
}

#[tauri::command]
pub async fn port_list(db: State<'_, DbPool>) -> Result<Vec<resin_core::PortMapping>, String> {
    db.list_ports()
}

#[tauri::command]
pub async fn port_upsert(
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    port: u16,
    protocol: String,
    platform_name: String,
    account: String,
    label: String,
    enabled: bool,
) -> Result<resin_core::PortMapping, String> {
    validate_port_mapping(port, &protocol, &platform_name, &account, &label)?;
    let acct = if account.trim().is_empty() {
        format!("port-{port}")
    } else {
        account
    };
    let m = resin_core::PortMapping {
        port,
        protocol: protocol.trim().to_ascii_lowercase(),
        platform_name,
        account: acct,
        label,
        enabled,
    };
    db.upsert_port(&m)?;
    // Hot-apply listeners (ADR-0012). Failure surfaces to GUI.
    forwarder.reload().await?;
    Ok(m)
}

#[tauri::command]
pub async fn port_remove(
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    port: u16,
) -> Result<bool, String> {
    if port < resin_core::MIN_USER_PORT {
        return Err(format!("port {port} is privileged"));
    }
    db.delete_port(port)?;
    forwarder.reload().await?;
    Ok(true)
}

#[tauri::command]
pub async fn port_running(forwarder: State<'_, resin_core::PortForwarder>) -> Result<Vec<u16>, String> {
    Ok(forwarder.running_ports())
}

#[tauri::command]
pub async fn port_reload(forwarder: State<'_, resin_core::PortForwarder>) -> Result<usize, String> {
    forwarder.reload().await
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
                { "id": "s2", "name": "sub-b", "node_count": 0, "healthy_node_count": 0, "last_error": "downloader: unexpected status 403", "last_checked": "2026-08-02T10:54:41.0883134Z" },
            ],
            "total": 2, "limit": 50, "offset": 0,
        });
        let snap = subscription_snapshot(&v);
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].name, "sub-a");
        assert_eq!(snap[0].node_count, 33);
        // Item 2 / Option B: default fields when Resin omits them.
        assert_eq!(snap[0].healthy_node_count, 0);
        assert_eq!(snap[0].last_error, "");
        assert_eq!(snap[0].last_checked, "");
        assert_eq!(snap[1].node_count, 0);
        // Item 2 / Option B: surface last_error/last_checked/healthy so a
        // fetch 403 is not a mute zero in the GUI.
        assert_eq!(snap[1].healthy_node_count, 0);
        assert_eq!(snap[1].last_error, "downloader: unexpected status 403");
        assert_eq!(snap[1].last_checked, "2026-08-02T10:54:41.0883134Z");
    }

    /// Item 2 / Option B: ensure the projection does not panic when Resin
    /// returns last_error as a null (some Go encoders emit null instead of
    /// empty for an unset string pointer). unwrap_or("") must handle both.
    #[test]
    fn subscription_snapshot_handles_null_last_error_and_missing_healthy() {
        let v = json!({
            "items": [
                { "id": "s3", "name": "sub-c", "node_count": 12, "healthy_node_count": 8, "last_error": null, "last_checked": "2026-08-02T11:00:00Z" },
                { "id": "s4", "name": "sub-d", "node_count": 5 },
            ],
            "total": 2,
        });
        let snap = subscription_snapshot(&v);
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].name, "sub-c");
        assert_eq!(snap[0].node_count, 12);
        assert_eq!(snap[0].healthy_node_count, 8);
        assert_eq!(snap[0].last_error, ""); // null collapses to empty string
        assert_eq!(snap[0].last_checked, "2026-08-02T11:00:00Z");
        assert_eq!(snap[1].name, "sub-d");
        assert_eq!(snap[1].node_count, 5);
        assert_eq!(snap[1].healthy_node_count, 0);
        assert_eq!(snap[1].last_error, "");
        assert_eq!(snap[1].last_checked, "");
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
    #[test]
    fn validate_port_mapping_rejects_privileged_and_bad_proto() {
        assert!(validate_port_mapping(80, "socks5", "OpenAI", "a", "").is_err());
        assert!(validate_port_mapping(17990, "ftp", "OpenAI", "a", "").is_err());
        assert!(validate_port_mapping(17990, "socks5", "Open.AI", "a", "").is_err());
        assert!(validate_port_mapping(17990, "http", "OpenAI", "port-17990", "k").is_ok());
    }

    #[test]
    fn validate_port_mapping_rejects_control_in_label() {
        assert!(validate_port_mapping(17990, "socks5", "OpenAI", "a", "bad\n").is_err());
    }

}
