//! platform domain IPC commands (EgressAPIKEY).
//!
//! Extracted from the former commands/mod.rs monolith by architecture-recovery
//! ticket 08: pure mechanical move - no behavior, naming, or IPC-surface change.
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_store::StoreExt;
use crate::sidecar::SidecarHandle;
use resin_core::{DbPool, IpcError};
use resin_core::{MAX_LANES, ReputationClient, ReputationProvider, ReputationSnapshot, parse_public_ips};
use super::common::{KEY_MAX_LEN, items_arr, map_resin_error, resin_client, validate_ip, validate_short_name};

#[tauri::command]
pub async fn platform_add(sidecar: State<'_, SidecarHandle>, name: String) -> Result<(), IpcError> {
    validate_short_name(&name, "platform")?;
    let client = resin_client(&sidecar)?;
    client
        .create_platform_from_name(&name)
        .await
        .map(|_| ())
        .map_err(|e| map_resin_error(&e.to_string()))
}

#[tauri::command]
pub async fn platform_remove(
    sidecar: State<'_, SidecarHandle>,
    name: String,
) -> Result<bool, IpcError> {
    validate_short_name(&name, "platform")?;
    let client = resin_client(&sidecar)?;
    let list = client.list_platforms().await.map_err(|e| map_resin_error(&e.to_string()))?;
    let id =
        platform_id_for_name(&list, &name).ok_or_else(|| format!("platform not found: {name}"))?;
    client
        .delete_platform(&id)
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
    Ok(true)
}

#[tauri::command]
pub async fn platform_list(sidecar: State<'_, SidecarHandle>) -> Result<Vec<String>, IpcError> {
    let client = resin_client(&sidecar)?;
    let list = client.list_platforms().await.map_err(|e| map_resin_error(&e.to_string()))?;
    Ok(platform_names(&list))
}

/// Phase R2: return the full platform objects (not just names) so the topology
/// canvas can render regex_filters, region_filters, allocation_policy,
/// routable_node_count. Returns raw JSON; the frontend parses it.
#[tauri::command]
pub async fn platform_list_full(
    sidecar: State<'_, SidecarHandle>,
) -> Result<serde_json::Value, IpcError> {
    let client = resin_client(&sidecar)?;
    client.list_platforms().await.map_err(|e| map_resin_error(&e.to_string()))
}

#[tauri::command]
pub async fn account_add(
    sidecar: State<'_, SidecarHandle>,
    platform: String,
    id: String,
    lane: usize,
) -> Result<(), IpcError> {
    validate_short_name(&platform, "platform")?;
    validate_short_name(&id, "account")?;
    if lane >= MAX_LANES {
        return Err(IpcError::from(format!("lane {lane} out of range (max {})", MAX_LANES - 1)));
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
) -> Result<bool, IpcError> {
    validate_short_name(&platform, "platform")?;
    validate_short_name(&account, "account")?;
    validate_ip(&ip)?;
    let _client = resin_client(&sidecar)?;
    Ok(true)
}

pub fn platform_names(v: &serde_json::Value) -> Vec<String> {
    items_arr(v)
        .iter()
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(String::from))
        .collect()
}

pub fn platform_id_for_name(v: &serde_json::Value, want: &str) -> Option<String> {
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

/// Ticket 17 / ADR-0055 D2: the ONLY write entry for process routes. The
/// rule family lives in the L2 whitebox (egressapikey-ports.json
/// process_routes field); writes commit through WhiteboxConfigStore::apply
/// (validate -> DB/listeners -> versioned file swap, ADR-0042 entry, the
/// same chain as port_upsert). The legacy L1 settings.json path is deleted.
#[tauri::command]
pub async fn process_route_add(
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    process: String,
    target_port: u16,
) -> Result<(), IpcError> {
    validate_short_name(&process, "process")?;
    if target_port < 1024 {
        return Err(IpcError::from(format!(
            "process_route_add: port {target_port} out of range (must be >= 1024, got {target_port})"
        )));
    }
    let mut next = whitebox.snapshot();
    if let Some(slot) = next
        .process_routes
        .iter_mut()
        .find(|r| r.process.trim().eq_ignore_ascii_case(process.trim()))
    {
        slot.target_port = target_port;
    } else {
        next.process_routes.push(resin_core::ProcessRouteRule {
            process: process.trim().to_string(),
            target_port,
        });
    }
    whitebox.apply(&db, &forwarder, next).await?;
    Ok(())
}

/// Ticket 17 / ADR-0055 D2: remove by process name (case-insensitive).
/// Returns false when no rule matched (nothing was written).
#[tauri::command]
pub async fn process_route_remove(
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    process: String,
) -> Result<bool, IpcError> {
    validate_short_name(&process, "process")?;
    let mut next = whitebox.snapshot();
    let before = next.process_routes.len();
    next.process_routes
        .retain(|r| !r.process.trim().eq_ignore_ascii_case(process.trim()));
    if next.process_routes.len() == before {
        return Ok(false);
    }
    whitebox.apply(&db, &forwarder, next).await?;
    Ok(true)
}

/// Ticket 17 / ADR-0055 D2: read the whitebox route family.
#[tauri::command]
pub async fn process_route_list(
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
) -> Result<Vec<resin_core::ProcessRouteRule>, IpcError> {
    Ok(whitebox.snapshot().process_routes)
}

#[tauri::command]
pub async fn subscription_add(
    sidecar: State<'_, SidecarHandle>,
    name: String,
    url: String,
    update_interval: Option<String>,
) -> Result<(), IpcError> {
    validate_short_name(&name, "subscription")?;
    if url.trim().is_empty() {
        return Err(IpcError::from("subscription url must be non-empty".to_string()));
    }
    if url.len() > KEY_MAX_LEN {
        return Err(IpcError::from("subscription url out of range".to_string()));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(IpcError::from("subscription url must start with http:// or https://".to_string()));
    }
    // T8-5: validate update_interval Go duration format (default 30s).
    let update_interval = update_interval.unwrap_or_else(|| "30s".to_string());
    if update_interval.len() > 10 || update_interval.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
        return Err(IpcError::from("update_interval: invalid (max 10 chars, no control)".to_string()));
    }
    let client = resin_client(&sidecar)?;

    // T20-P3: POST source_type=remote directly (ADR-0045). Resin's Scheduler
    // re-pulls the remote URL via its own clash.meta UA fetcher
    // (cmd/resin/main.go const downloadUserAgent); the shell no longer
    // re-fetches or converts the Clash YAML itself (P13 B4 chain deleted).
    // update_interval default 30s — Resin has no public force-refresh
    // endpoint, the 30s tick lands the first background fetch within seconds.
    tracing::info!(subscription = %name, url = %url, "subscription_add: POST source_type=remote");
    let body = serde_json::json!({
        "name": name,
        "source_type": "remote",
        "url": url,
        "update_interval": update_interval,
    });
    match client.create_subscription(body).await {
        Ok(v) => {
            tracing::info!(?v, "subscription_add: Resin accepted subscription");
            Ok(())
        }
        Err(e) => {
            tracing::warn!(error = ?e, "subscription_add: Resin POST failed");
            Err(IpcError::from(e.to_string()))
        }
    }
}

#[tauri::command]
pub async fn subscription_remove(
    sidecar: State<'_, SidecarHandle>,
    name: String,
) -> Result<bool, IpcError> {
    validate_short_name(&name, "subscription")?;
    let client = resin_client(&sidecar)?;
    let list = client
        .list_subscriptions()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
    let id = subscription_id_for_name(&list, &name)
        .ok_or_else(|| format!("subscription not found: {name}"))?;
    client
        .delete_subscription(&id)
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
    Ok(true)
}

/// T20-P2: trigger Resin-native subscription refresh by POST /actions/refresh.
/// source_type must be "remote" (ADR-0045); a local-source subscription re-parses
/// in-memory content on /actions/refresh (no HTTP, no new upstream nodes).
/// The paired subscription_add migrated to source_type=remote so refresh
/// actually pulls new nodes. Resin's Scheduler re-fetches via its own clash.meta
/// UA fetcher (cmd/resin/main.go const downloadUserAgent); the shell no longer
/// re-fetches or converts the Clash YAML itself (P13 B4 chain deleted).
/// Returns the post-refresh node_count so the UI can show a toast without a
/// second list round trip.
#[tauri::command]
pub async fn subscription_refresh(
    sidecar: State<'_, SidecarHandle>,
    name: String,
) -> Result<u64, IpcError> {
    validate_short_name(&name, "subscription")?;
    let client = resin_client(&sidecar)?;
    let list = client
        .list_subscriptions()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
    let items = items_arr(&list);
    let id = subscription_id_for_name(&list, &name)
        .ok_or_else(|| IpcError::from(format!("subscription not found: {name}")))?;
    tracing::info!(subscription = %name, id = %id, "subscription_refresh: POST /actions/refresh");
    client
        .refresh_subscription_native(&id)
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
    // Return the post-refresh node_count so the UI can show a toast.
    // The /actions/refresh response body is empty on success; fall back to
    // the existing list snapshot so the toast stays informative.
    let node_count = items
        .iter()
        .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
        .and_then(|p| p.get("node_count").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    tracing::info!(subscription = %name, node_count, "subscription_refresh: action accepted");
    Ok(node_count)
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
pub async fn subscription_list(
    sidecar: State<'_, SidecarHandle>,
) -> Result<Vec<SubscriptionSnapshotEntry>, IpcError> {
    let client = resin_client(&sidecar)?;
    let list = client
        .list_subscriptions()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
    Ok(subscription_snapshot(&list))
}

#[tauri::command]
pub async fn node_pool_snapshot(
    sidecar: State<'_, SidecarHandle>,
) -> Result<serde_json::Value, IpcError> {
    let client = resin_client(&sidecar)?;
    client.node_pool_snapshot().await.map_err(|e| map_resin_error(&e.to_string()))
}

/// The allocation_policy values Resin v1.1.2 actually accepts (probed
/// 2026-07-31). The IPC layer rejects anything else before reaching Resin.
pub const ALLOWED_ALLOCATION_POLICIES: &[&str] = &["BALANCED", "PREFER_LOW_LATENCY", "PREFER_IDLE_IP"];

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
    passive_circuit_breaker_disabled: Option<bool>,
) -> Result<serde_json::Value, IpcError> {
    validate_short_name(&name, "platform")?;
    let client = resin_client(&sidecar)?;
    // Resolve name -> id (same pattern as platform_remove).
    let list = client.list_platforms().await.map_err(|e| map_resin_error(&e.to_string()))?;
    let id =
        platform_id_for_name(&list, &name).ok_or_else(|| format!("platform not found: {name}"))?;

    // Build the PATCH body with only the fields the caller provided. Validate
    // each at the IPC boundary (AGENTS 7.5) so a hostile webview cannot send
    // an unsupported policy or an oversized filter to Resin.
    let mut body = serde_json::Map::new();
    if let Some(ref policy) = allocation_policy {
        if !ALLOWED_ALLOCATION_POLICIES.contains(&policy.as_str()) {
            return Err(IpcError::from(format!(
                "allocation_policy must be one of {:?}",
                ALLOWED_ALLOCATION_POLICIES
            )));
        }
        body.insert(
            "allocation_policy".to_string(),
            serde_json::Value::String(policy.clone()),
        );
    }
    if let Some(ref filters) = regex_filters {
        if filters.len() > 64 {
            return Err(IpcError::from("regex_filters: too many entries (max 64)".to_string()));
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
            return Err(IpcError::from("region_filters: too many entries (max 64)".to_string()));
        }
        let arr: Vec<serde_json::Value> = filters
            .iter()
            .map(|f| {
                if f.len() > 16
                    || f.bytes()
                        .any(|b| b == 0 || b < 0x20 || b == 0x7f || b == b' ')
                {
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
            return Err(IpcError::from("sticky_ttl: invalid (max 32 chars, no control)".to_string()));
        }
        body.insert(
            "sticky_ttl".to_string(),
            serde_json::Value::String(ttl.clone()),
        );
    }
    // T8-1: passive_circuit_breaker_disabled — platform-level boolean.
    // When false (default) the circuit breaker is ENABLED: nodes with
    // consecutive failures (threshold set by system max_consecutive_failures)
    // are auto-isolated. When true, the circuit breaker is disabled for this
    // platform and all nodes are always eligible.
    if let Some(disabled) = passive_circuit_breaker_disabled {
        body.insert(
            "passive_circuit_breaker_disabled".to_string(),
            serde_json::Value::Bool(disabled),
        );
    }
    if body.is_empty() {
        return Err(IpcError::from("platform_update: no fields to update".to_string()));
    }
    client
        .update_platform(&id, serde_json::Value::Object(body))
        .await
        .map_err(|e| map_resin_error(&e.to_string()))
}

/// GET /api/v1/nodes - return the full node list (the "C category" ip/ip
/// channels) as raw JSON. The frontend renders egress IPs, health, protocol.
#[tauri::command]
pub async fn node_list(sidecar: State<'_, SidecarHandle>) -> Result<serde_json::Value, IpcError> {
    let client = resin_client(&sidecar)?;
    client.list_nodes().await.map_err(|e| map_resin_error(&e.to_string()))
}

/// T19-P3: pure input validation for node_probe. Exposed as a module-level
/// free fn so the command body and the unit tests share one implementation
/// without a Tauri runtime. Returns an owned String for ergonomic mapping
/// to IpcError::from in the command body.
pub fn validate_node_probe_inputs(node_hash: &str, kind: &str) -> Result<(), String> {
    if node_hash.is_empty() || node_hash.len() > 128 {
        return Err("node_hash length out of range (1..=128)".to_string());
    }
    if node_hash.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
        return Err("node_hash contains control characters".to_string());
    }
    if kind != "egress" && kind != "latency" {
        return Err("kind must be egress or latency".to_string());
    }
    Ok(())
}

/// T19-P3: on-demand node probe. Forwards to Resin's native
/// POST /api/v1/nodes/{hash}/actions/probe-egress or .../probe-latency
/// (v1.2.0 HandleProbeEgress/HandleProbeLatency). The shell validates the
/// node_hash (length <= 128, no control chars) and kind (in {"egress",
/// "latency"}) at the IPC boundary so a hostile webview can't POST to an
/// arbitrary hash path. Resin returns {egress_ip, region, latency_ewma_ms}
/// for egress and {latency_ewma_ms} for latency; the shell forwards the JSON
/// body verbatim to the webview.
#[tauri::command]
pub async fn node_probe(
    sidecar: State<'_, SidecarHandle>,
    node_hash: String,
    kind: String,
) -> Result<serde_json::Value, IpcError> {
    validate_node_probe_inputs(&node_hash, &kind)
        .map_err(IpcError::from)?;
    let client = resin_client(&sidecar)?;
    let result = if kind == "egress" {
        client.probe_node_egress(&node_hash).await
    } else {
        client.probe_node_latency(&node_hash).await
    };
    result.map_err(|e| map_resin_error(&e.to_string()))
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
) -> Result<serde_json::Value, IpcError> {
    // body must be a JSON object with a non-empty "name".
    let obj = body
        .as_object()
        .ok_or_else(|| "platform_create_with_fields: body must be a JSON object".to_string())?;
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "platform_create_with_fields: missing 'name' field".to_string())?;
    validate_short_name(name, "platform")?;
    // If allocation_policy is present, must be one of the allowed enum.
    if let Some(policy) = obj.get("allocation_policy").and_then(|v| v.as_str()) {
        if !ALLOWED_ALLOCATION_POLICIES.contains(&policy) {
            return Err(IpcError::from(format!(
                "allocation_policy must be one of {:?}",
                ALLOWED_ALLOCATION_POLICIES
            )));
        }
    }
    // If regex_filters present, cap count + per-entry length (mirrors platform_update).
    if let Some(arr) = obj.get("regex_filters").and_then(|v| v.as_array()) {
        if arr.len() > 64 {
            return Err(IpcError::from("regex_filters: too many entries (max 64)".to_string()));
        }
        for f in arr {
            if let Some(s) = f.as_str() {
                if s.len() > 253 || s.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
                    return Err(IpcError::from(
                        "regex_filters: entry invalid (max 253 chars, no control)".to_string()
                    ));
                }
            }
        }
    }
    // If region_filters present, cap each at 16 chars (ISO 3166-1 alpha-2 + negation).
    if let Some(arr) = obj.get("region_filters").and_then(|v| v.as_array()) {
        if arr.len() > 64 {
            return Err(IpcError::from("region_filters: too many entries (max 64)".to_string()));
        }
        for r in arr {
            if let Some(s) = r.as_str() {
                if s.len() > 16
                    || s.bytes()
                        .any(|b| b == 0 || b < 0x20 || b == 0x7f || b == b' ')
                {
                    return Err(IpcError::from("region_filter invalid (max 16, no control/space)".to_string()));
                }
            }
        }
    }
    // If sticky_ttl present, cap at 32 chars + no control (mirrors platform_update).
    if let Some(ttl) = obj.get("sticky_ttl").and_then(|v| v.as_str()) {
        if ttl.len() > 32 || ttl.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
            return Err(IpcError::from("sticky_ttl: invalid (max 32 chars, no control)".to_string()));
        }
    }
    let client = resin_client(&sidecar)?;
    client
        .create_platform_with_fields(body)
        .await
        .map_err(|e| map_resin_error(&e.to_string()))
}

/// GET /api/v1/platforms/{id}/leases - the live leases on a platform, used by
/// the Milestone B right pane to show which accounts are already bound to an
/// exit IP on each platform. We resolve the platform name -> id (the webview
/// only knows the user-visible name) and forward to ResinClient.
#[tauri::command]
pub async fn platform_leases(
    sidecar: State<'_, SidecarHandle>,
    name: String,
) -> Result<serde_json::Value, IpcError> {
    validate_short_name(&name, "platform")?;
    let client = resin_client(&sidecar)?;
    let list = client.list_platforms().await.map_err(|e| map_resin_error(&e.to_string()))?;
    let id =
        platform_id_for_name(&list, &name).ok_or_else(|| format!("platform not found: {name}"))?;
    client.platform_leases(&id).await.map_err(|e| map_resin_error(&e.to_string()))
}

pub fn subscription_snapshot(v: &serde_json::Value) -> Vec<SubscriptionSnapshotEntry> {
    items_arr(v)
        .iter()
        .filter_map(|p| {
            let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let node_count = p.get("node_count").and_then(|n| n.as_u64()).unwrap_or(0);
            if name.is_empty() {
                None
            } else {
                let healthy_node_count = p
                    .get("healthy_node_count")
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                let last_error = p
                    .get("last_error")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let last_checked = p
                    .get("last_checked")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(SubscriptionSnapshotEntry {
                    name: name.to_string(),
                    node_count,
                    healthy_node_count,
                    last_error,
                    last_checked,
                })
            }
        })
        .collect()
}

pub fn subscription_id_for_name(v: &serde_json::Value, want: &str) -> Option<String> {
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

/// One active lease row from Resin /api/v1/metrics/realtime/leases, projected
/// for the GUI Topology lease panel. Mirrors the items-wrapper Resin returns.
#[derive(Debug, Clone, Serialize)]
pub struct LeaseEntry {
    /// Platform UUID (Resin's internal id). Empty = Default platform.
    pub platform_id: String,
    /// Account string Resin binds to this lease (the upstream business identity).
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
pub async fn lease_map(sidecar: State<'_, SidecarHandle>) -> Result<Vec<LeaseEntry>, IpcError> {
    let client = resin_client(&sidecar)?;
    let raw = client.active_leases().await.map_err(|e| map_resin_error(&e.to_string()))?;
    // Resin returns {"items":[{active_leases:N,"ts":"...","platform_id":""}]}
    // or a bare array. We use the shared items_arr helper to be robust.
    let items = items_arr(&raw);
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        let platform_id = it
            .get("platform_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let account = it
            .get("account")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let egress_ip = it
            .get("egress_ip")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let node_tag = it
            .get("node_tag")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let target_domain = it
            .get("target_domain")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let ts = it
            .get("ts")
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

/// Reputation only queries public egress IPs already reported by the local Resin
/// lease API. Provider credentials stay in the Rust-side settings store and are
/// never returned to the webview.
#[tauri::command]
pub async fn ip_reputation_snapshot(
    app: AppHandle,
    sidecar: State<'_, SidecarHandle>,
) -> Result<ReputationSnapshot, IpcError> {
    let store = app
        .store("settings.json")
        .map_err(|e| IpcError::from(format!("settings store: {e}")))?;
    let provider_name = store
        .get("ipReputationProvider")
        .and_then(|v| v.as_str().map(str::to_string));
    let Some(provider) = provider_name.as_deref().and_then(ReputationProvider::parse) else {
        return Ok(ReputationSnapshot {
            provider: None,
            status: "disabled".into(),
            entries: Vec::new(),
        });
    };
    let api_key = if provider.requires_key() {
        store
            .get(provider.key_name())
            .and_then(|v| v.as_str().map(str::to_string))
    } else {
        None
    };
    if provider.requires_key() && api_key.as_deref().unwrap_or("").trim().is_empty() {
        return Ok(ReputationSnapshot {
            provider: Some(provider),
            status: "not_configured".into(),
            entries: Vec::new(),
        });
    }
    let client = resin_client(&sidecar)?;
    let raw = client.active_leases().await.map_err(|e| map_resin_error(&e.to_string()))?;
    let ips = parse_public_ips(
        items_arr(&raw)
            .iter()
            .filter_map(|v| {
                v.get("egress_ip")
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
            })
            .collect::<Vec<_>>(),
        50,
    );
    let reputation = ReputationClient::new().map_err(|e| IpcError::from(e.to_string()))?;
    let mut entries = Vec::with_capacity(ips.len());
    for ip in ips {
        match reputation.lookup(provider, api_key.as_deref(), ip).await {
            Ok(entry) => entries.push(entry),
            Err(e) => tracing::warn!(error = %e, "ip reputation lookup failed"),
        }
    }
    Ok(ReputationSnapshot {
        provider: Some(provider),
        status: "ok".into(),
        entries,
    })
}
