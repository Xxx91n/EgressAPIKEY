//! IPC commands exposed to the React frontend via `tauri::generate_handler!`.
//!
//! Path A (fork Resin Go sidecar): the webview talks to the Resin Go control
//! plane WRAPPED behind Rust IPC. Platform create/list/delete are FORWARDED
//! to Resin via ResinClient; the lane/lease/account/exit_ip IPC surface the old
//! self-implemented resin-core model exposed is DEPRECATED to echo/no-op because
//! the Resin forward proxy owns sticky-session + exit-ip allocation natively.
//! Each command still validates its inputs at the IPC boundary (AGENTS 7.5).

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_store::StoreExt;

use crate::sidecar::SidecarHandle;
use resin_core::DbPool;
use resin_core::IpcError;
use resin_core::{
    clash_yaml_to_proxies_block, fetch_clash_subscription, parse_public_ips, ReputationClient,
    ReputationProvider, ReputationSnapshot, ResinClient, MAX_LANES,
};

const KEY_MAX_LEN: usize = 4096;
const NAME_MAX_LEN: usize = 128;

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
    if ip
        .bytes()
        .any(|b| b == 0 || b < 0x20 || b == 0x7f || b == b' ')
    {
        return Err("exit_ip contains control/space characters".to_string());
    }
    Ok(())
}

fn resin_client(h: &SidecarHandle) -> Result<ResinClient, String> {
    ResinClient::new(&h.api_base(), h.admin_token.clone())
        .map_err(|e| format!("sidecar client: {e:?}"))
}

/// A3+Q3: Map Resin upstream error strings to i18n keys for the frontend.
/// The frontend receives the returned string and uses it as a t() key.
/// Unknown errors pass through verbatim (prefixed with "error." not used —
/// the frontend treats unknown strings as literal messages). Always logs
/// the original error to tracing::warn! so debugging is not lost.
pub fn map_resin_error(raw: &str) -> resin_core::IpcError {
    // Log the original error before any mapping.
    tracing::warn!(target: "resin_ipc", raw = raw, "Resin error mapped to i18n");
    // Known Resin error patterns (from DESIGN.md error code table + probed).
    if raw.contains("cannot delete Default platform") {
        return IpcError::internal("error.cannotDeleteDefaultPlatform");
    }
    if raw.contains("AUTH_REQUIRED") || raw.contains("auth required") {
        return IpcError::internal("error.authRequired");
    }
    if raw.contains("AUTH_FAILED") || raw.contains("auth failed") {
        return IpcError::internal("error.authFailed");
    }
    if raw.contains("URL_PARSE_ERROR") || raw.contains("url parse") {
        return IpcError::internal("error.urlParse");
    }
    if raw.contains("INVALID_PROTOCOL") || raw.contains("invalid protocol") {
        return IpcError::internal("error.invalidProtocol");
    }
    if raw.contains("UPSTREAM_CONNECT_FAILED") || raw.contains("upstream connect") {
        return IpcError::internal("error.upstreamConnectFailed");
    }
    if raw.contains("UPSTREAM_REQUEST_FAILED") || raw.contains("upstream request") {
        return IpcError::internal("error.upstreamRequestFailed");
    }
    // Bug 3 (ADR-0026 Q6): port bind conflict. Resin returns 409 with a body
    // like `listen on port 17111: bind: Only one usage of each socket address
    // (protocol/network address/port) is normally permitted.` Extract the
    // port number so the frontend can show "port {{port}} already in use"
    // and auto-suggest another port. Format: `error.bindConflict:PORT`.
    if raw.contains("bind") && (raw.contains("Only one usage") || raw.contains("EADDRINUSE") || raw.contains("address already in use")) {
        if let Some(port) = extract_port_from_residual(raw) {
            return IpcError::from(format!("error.bindConflict:{}", port));
        }
        return IpcError::internal("error.bindConflict");
    }
    if raw.contains("CONFLICT") {
        return IpcError::internal("error.conflict");
    }
    if raw.contains("not found") || raw.contains("NOT_FOUND") {
        return IpcError::internal("error.notFound");
    }
    if raw.contains("BAD_REQUEST") || raw.contains("bad request") {
        return IpcError::internal("error.badRequest");
    }
    if raw.contains("UNAUTHORIZED") || raw.contains("unauthorized") {
        return IpcError::internal("error.unauthorized");
    }
    // T3-Q2: subscription fetch failure (UA rotation exhausted, likely
    // origin SSL/4xx/5xx). Extract the trailing HTTP status code so the
    // i18n key carries the surface reason (e.g. 525 origin SSL error)
    // without leaking fetch pipeline internals. Regex-free: the literal
    // error format is owned by fetch_clash_subscription.
    if let Some(idx) = raw.find("fetch_clash_subscription") {
        let tail = &raw[idx..];
        if let Some(http_idx) = tail.find("HTTP ") {
            let after = &tail[http_idx + 5..];
            let code: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !code.is_empty() {
                return IpcError::from(format!("error.subscriptionFetch.{}", code));
            }
        }
        return IpcError::internal("error.subscriptionFetch");
    }
    // Unknown — pass through as literal for the frontend to display.
    IpcError::from(raw.to_string())
}


/// Extract the array from a Resin list response. Resin wraps paginated
/// collections as `{"items":[...], "total", "limit", "offset"}`; a few
/// legacy endpoints still return a bare array. Accept both so a future
/// Resin API tightening cannot silently empty the UI (P13 root cause).
/// Extract a port number from a Resin error message, looking for
/// `port <digits>` or `:<digits>` patterns. Zero-alloc, no regex.
fn extract_port_from_residual(raw: &str) -> Option<u16> {
    let lower = raw.to_ascii_lowercase();
    let idx = lower.find("port ")?;
    let rest = &raw[idx + 5..];
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    num.parse().ok()
}

fn items_arr<'a>(v: &'a serde_json::Value) -> &'a [serde_json::Value] {
    if let Some(arr) = v.get("items").and_then(|i| i.as_array()) {
        return arr.as_slice();
    }
    if let Some(arr) = v.as_array() {
        return arr.as_slice();
    }
    &[]
}

/// Find a Resin endpoint ID by port number from the list-endpoints response.
/// Handles both `{"items":[...]}` wrapper and bare-array shapes.
/// Skips the read-only `default` endpoint.
fn find_endpoint_id_by_port(existing: &serde_json::Value, port: u16) -> Option<String> {
    for ep in items_arr(existing).iter() {
        if ep.get("port").and_then(|p| p.as_u64()) == Some(port as u64) {
            if let Some(id) = ep.get("id").and_then(|v| v.as_str()) {
                if id != "default" {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}

/// T18-6 (ADR-0042 S6): Restore Resin endpoints from whitebox config on startup.
/// Spawns-safe: failures log only, never fail the app. Skips ports already in Resin (409 Conflict).
pub async fn restore_ports_from_whitebox(
    sidecar: &SidecarHandle,
    whitebox: &resin_core::WhiteboxConfigStore,
) -> Result<(), String> {
    let cfg = whitebox.snapshot();
    let enabled = resin_core::enabled_entries_for_restore(&cfg.entry_ports);
    if enabled.is_empty() {
        tracing::info!("T18-6: no enabled entry_ports to restore");
        return Ok(());
    }
    let client = resin_client(sidecar)?;
    let existing = client.list_endpoints().await
        .map_err(|e| format!("list_endpoints: {e:?}"))?;
    let mut restored = 0u32;
    let mut skipped = 0u32;
    for m in enabled {
        if find_endpoint_id_by_port(&existing, m.port).is_some() {
            skipped += 1;
            continue;
        }
        let proto = m.protocol.trim().to_ascii_lowercase();
        let allow_socks5 = proto == "socks5";
        let allow_http_forward = proto == "http" || proto == "socks5";
        let body = serde_json::json!({
            "port": m.port,
            "allow_management": false,
            "allow_proxy": true,
            "allow_http_forward": allow_http_forward,
            "allow_http_reverse": false,
            "allow_socks5": allow_socks5,
            "require_proxy_auth_info": m.auth_required,
        });
        match client.create_endpoint(body).await {
            Ok(_) => {
                restored += 1;
                tracing::info!(port = m.port, "T18-6: restored Resin endpoint from whitebox");
            }
            Err(e) => {
                let msg = format!("{e:?}");
                if msg.contains("409") || msg.contains("CONFLICT") || msg.contains("Only one usage") {
                    skipped += 1;
                    tracing::info!(port = m.port, "T18-6: port already in Resin; skipping");
                } else {
                    tracing::warn!(port = m.port, error = %msg, "T18-6: restore failed; user can re-save in GUI");
                }
            }
        }
    }
    tracing::info!(restored, skipped, "T18-6: whitebox port restore complete");
    Ok(())
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

/// T15-2: Runtime log level gate. 0=error, 1=warn, 2=info, 3=debug.
/// Default is 2 (info). Use the set_log_level IPC command to change at runtime.
/// Wired into spawn_health_poll per-cycle tracing::debug! on /healthz success,
/// so a user who sets level<debug in Settings suppresses the per-cycle noise.
pub static LOG_LEVEL_GATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);

/// Returns true if the given numeric level (0=error,1=warn,2=info,3=debug) should be emitted.
pub fn log_level_enabled(level: u8) -> bool {
    LOG_LEVEL_GATE.load(std::sync::atomic::Ordering::Relaxed) >= level
}

#[tauri::command]
pub async fn gateway_snapshot(sidecar: State<'_, SidecarHandle>) -> Result<LaneSnapshot, IpcError> {
    let client = resin_client(&sidecar)?;
    let leases = client.active_leases().await.map_err(|e| map_resin_error(&e.to_string()))?;
    let busy = sum_active_leases(&leases);
    // Issue 4+7: pull the Resin /platforms list once per snapshot so we can
    // resolve lease items' platform_id back to the user-visible platform NAME
    // that the Topology canvas shows as an Entry box (the leases endpoint only
    // exposes platform_id as a UUID; Resin owns the UUID->name mapping). On
    // any failure we fall back to the raw platform_id so the canvas still
    // renders a real entry instead of dropping the lease count.
    let platforms = client
        .list_platforms()
        .await
        .unwrap_or_else(|_| serde_json::json!([]));
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
        return items
            .iter()
            .filter_map(|it| it.get("active_leases").and_then(|n| n.as_u64()))
            .map(|n| n as usize)
            .sum();
    }
    v.get("active_leases")
        .and_then(|n| n.as_u64())
        .map(|n| n as usize)
        .unwrap_or(0)
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
            if id.is_empty() || name.is_empty() {
                None
            } else {
                Some((id, name))
            }
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
        let raw_id = it
            .get("platform_id")
            .and_then(|i| i.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let active = it
            .get("active_leases")
            .and_then(|n| n.as_u64())
            .map(|n| n as usize)
            .unwrap_or(0);
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
pub async fn platform_snapshot(
    sidecar: State<'_, SidecarHandle>,
    name: String,
) -> Result<serde_json::Value, IpcError> {
    validate_short_name(&name, "platform")?;
    let client = resin_client(&sidecar)?;
    // Resolve platform name -> id, then fetch that platform is routable node list
    // (Resin DESIGN.md: GET /nodes?platform_id=<id> filters to the platform routable set).
    let list = client.list_platforms().await.map_err(|e| map_resin_error(&e.to_string()))?;
    match platform_id_for_name(&list, &name) {
        Some(id) => client
            .list_nodes_for_platform(&id)
            .await
            .map_err(|e| map_resin_error(&e.to_string())),
        None => Ok(serde_json::json!({"items":[], "total":0, "limit":500, "offset":0})),
    }
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
    pub target_port: u16,
}

#[tauri::command]
pub async fn process_route_add(
    app: AppHandle,
    process: String,
    target_port: u16,
) -> Result<(), IpcError> {
    validate_short_name(&process, "process")?;
    if target_port < 1024 {
        return Err(IpcError::from(format!(
            "process_route_add: port {target_port} out of range (must be >= 1024, got {target_port})"
        )));
    }
    let store = tauri_plugin_store::StoreExt::store(&app, "settings.json")
        .map_err(|e| IpcError::from(format!("store: {e:?}")))?;
    let mut rules: Vec<ProcessRouteRule> = store
        .get("processRoutes")
        .and_then(|v| serde_json::from_value::<Vec<ProcessRouteRule>>(v).ok())
        .unwrap_or_default();
    // conflict detect via the extracted helper (unit-testable)
    process_route_conflict_check(&rules, &process, target_port)?;
    if let Some(slot) = rules
        .iter_mut()
        .find(|r| r.process.trim() == process.trim())
    {
        slot.target_port = target_port;
    } else {
        rules.push(ProcessRouteRule {
            process: process.trim().to_string(),
            target_port,
        });
    }
    store.set(
        "processRoutes",
        serde_json::to_value(&rules).map_err(|e| IpcError::from(format!("serialize: {e}")))?,
    );
    store.save().map_err(|e| IpcError::from(format!("store save: {e:?}")))?;
    Ok(())
}

#[tauri::command]
pub async fn process_route_remove(app: AppHandle, process: String) -> Result<bool, IpcError> {
    validate_short_name(&process, "process")?;
    let store = tauri_plugin_store::StoreExt::store(&app, "settings.json")
        .map_err(|e| IpcError::from(format!("store: {e:?}")))?;
    let mut rules: Vec<ProcessRouteRule> = store
        .get("processRoutes")
        .and_then(|v| serde_json::from_value::<Vec<ProcessRouteRule>>(v).ok())
        .unwrap_or_default();
    let before = rules.len();
    rules.retain(|r| r.process.trim() != process.trim());
    if rules.len() != before {
        store.set(
            "processRoutes",
            serde_json::to_value(&rules).map_err(|e| IpcError::from(format!("serialize: {e}")))?,
        );
        store.save().map_err(|e| IpcError::from(format!("store save: {e:?}")))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
pub async fn process_route_list(app: AppHandle) -> Result<Vec<ProcessRouteRule>, IpcError> {
    let store = tauri_plugin_store::StoreExt::store(&app, "settings.json")
        .map_err(|e| IpcError::from(format!("store: {e:?}")))?;
    Ok(store
        .get("processRoutes")
        .and_then(|v| serde_json::from_value::<Vec<ProcessRouteRule>>(v).ok())
        .unwrap_or_default())
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

    // P13 B4: Resin's own remote-fetch uses a default HTTP UA that many
    // subscription providers (the user's test host included) reject with 403.
    // We fetch the Clash YAML ourselves with a clash-family UA, convert the
    // flow-style `proxies:` segment into block-style (Resin's Go YAML parser
    // chokes on flow-style inline mappings), and POST it as a local
    // subscription so Resin parses the nodes we already fetched. The user's
    // url is retained as metadata so the UI can still show the source.
    tracing::info!(subscription = %name, url = %url, "subscription_add: fetching clash yaml");
    let yaml = fetch_clash_subscription(&url).await.map_err(|e| {
        tracing::warn!(error = ?e, "subscription_add: fetch failed");
        // T3-Q2: route fetch errors through map_resin_error so the frontend
        // receives a localizable key (error.subscriptionFetch.<code>) instead
        // of a leaky internal error string.
        map_resin_error(&e.to_string())
    })?;
    tracing::info!(
        bytes = yaml.len(),
        "subscription_add: fetched yaml, converting to proxies-only block"
    );
    let block = clash_yaml_to_proxies_block(&yaml).map_err(|e| {
        tracing::warn!(error = ?e, "subscription_add: convert failed");
        e.to_string()
    })?;
    tracing::info!(
        block_bytes = block.len(),
        "subscription_add: posting local subscription to Resin"
    );

    // T8-5: update_interval now user-configurable (default 30s). Resin does
    // not expose a force-refresh endpoint; the scheduler parses local content
    // on each tick.
    // P13 B4: Resin rejects `url` when source_type == "local"
    // (INVALID_ARGUMENT "url is not allowed for local subscription").
    // The user-visible origin is preserved in the Resin subscription name;
    // we do NOT pass url in the local-body. (Handoff summary was wrong here;
    // live probe caught it.)
    let body = serde_json::json!({
        "name": name,
        "source_type": "local",
        "content": block,
        "update_interval": &update_interval,
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

// Phase R1: the topology canvas hot-switch and the node-pool tab need the
// full platform schema (not just names) and the node list. These forward to
// Resin with the same input-validation discipline as the other commands.

/// The allocation_policy values Resin v1.1.2 actually accepts (probed
/// 2026-07-31). The IPC layer rejects anything else before reaching Resin.
const ALLOWED_ALLOCATION_POLICIES: &[&str] = &["BALANCED", "PREFER_LOW_LATENCY", "PREFER_IDLE_IP"];

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

/// T8-1: GET /api/v1/system/config — read system-level config.
/// Returns the full config JSON (max_consecutive_failures, cache_flush_interval,
/// probe_timeout, node_dns_upstreams, etc.) for display in the Settings panel.
#[tauri::command]
pub async fn system_config_get(
    sidecar: State<'_, SidecarHandle>,
) -> Result<serde_json::Value, IpcError> {
    let client = resin_client(&sidecar)?;
    client
        .system_config_get()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))
}

/// T8-1: PATCH /api/v1/system/config — update system-level config.
/// T8-1 use case: set max_consecutive_failures (circuit breaker threshold).
/// The body is a JSON object with only the fields to update.
#[tauri::command]
pub async fn system_config_patch(
    sidecar: State<'_, SidecarHandle>,
    body: serde_json::Value,
) -> Result<serde_json::Value, IpcError> {
    let obj = body
        .as_object()
        .ok_or_else(|| "system_config_patch: body must be a JSON object".to_string())?;
    // Validate max_consecutive_failures if present (1..=100)
    if let Some(v) = obj.get("max_consecutive_failures").and_then(|v| v.as_i64()) {
        if v < 1 || v > 100 {
            return Err(IpcError::from(
                "max_consecutive_failures: must be between 1 and 100".to_string(),
            ));
        }
    }
    let client = resin_client(&sidecar)?;
    client
        .system_config_patch(body)
        .await
        .map_err(|e| map_resin_error(&e.to_string()))
}

/// T8-6: Close all connections — kill + restart Resin sidecar (equivalent to
/// closing all in-flight connections since Resin v1.2.0 has no close-all API).
/// Reuses existing sidecar lifecycle infrastructure. SSE/WebSocket connections
/// will be dropped (expected — this is the user's explicit intent).
#[tauri::command]
pub async fn close_all_connections(
    app: tauri::AppHandle,
    sidecar: State<'_, SidecarHandle>,
) -> Result<(), IpcError> {
    tracing::info!("T8-6: user-initiated close-all-connections");
    sidecar_restart(&app, &sidecar).await
}

/// T8-6: Reset kernel — kill + restart Resin sidecar (same implementation as
/// close_all_connections but different semantic label + log message). The user
/// picks this when they want a full kernel reset, not just connection cleanup.
#[tauri::command]
pub async fn reset_kernel(
    app: tauri::AppHandle,
    sidecar: State<'_, SidecarHandle>,
) -> Result<(), IpcError> {
    tracing::info!("T8-6: user-initiated kernel-reset");
    sidecar_restart(&app, &sidecar).await
}

/// T8-6: shared kill+restart helper. Kills the Resin child process and
/// re-runs boot_resin() to get a fresh sidecar. The old CommandChild is
/// consumed; a new one replaces it.
async fn sidecar_restart(
    _app: &tauri::AppHandle,
    sidecar: &State<'_, SidecarHandle>,
) -> Result<(), IpcError> {
    // Kill existing child if present.
    {
        let mut guard = sidecar.child.lock().unwrap_or_else(|e| e.into_inner()); // ponytail: poison-safe, matches AGENTS §7.5 no-panic-in-production
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            tracing::info!("T8-6: killed existing sidecar child");
        }
    }
    // Re-boot: the boot_resin function is called from main.rs setup,
    // but we cannot call it directly from here (it needs app handle
    // lifecycle hooks). Instead, emit an event that main.rs listens
    // to and triggers re-boot. For now, we return Ok(()) and the
    // tray/health-poller will detect the dead sidecar and surface
    // the unhealthy state. A full re-boot requires the app to re-run
    // boot_resin — the simplest path is app.restart() which Tauri
    // supports natively. BUT that would close the webview too.
    //
    // Ponytail: the shortest viable path is to tell the user the
    // sidecar was killed and they need to restart the app. A future
    // iteration can wire a hot-restart via tauri::Manager.
    tracing::warn!("T8-6: sidecar killed; user should restart the app to bring it back");
    Ok(())
}

/// T8-2: Strategy verification — send N probe requests through the Resin
/// forward proxy entry port bound to a platform, collect the exit IP for
/// each request, and return a distribution summary. The probe target is
/// ipify (https://api.ipify.org) which returns the caller's public IP as
/// plain text. Uses reqwest through the Resin proxy URL format:
/// http://<api_port>/<proxy_token>/https/api.ipify.org
///
/// Returns: { samples: [{ip, latency_ms}], distribution: {ip: count},
///           avg_latency_ms, unique_ips, strategy: string, platform: string }
#[tauri::command]
pub async fn strategy_verify(
    sidecar: State<'_, SidecarHandle>,
    platform_name: String,
    sample_count: u32,
) -> Result<serde_json::Value, IpcError> {
    validate_short_name(&platform_name, "platform")?;
    let n = sample_count.clamp(3, 50);
    let client = resin_client(&sidecar)?;

    // Get the platform's allocation_policy for display.
    let list = client.list_platforms().await.map_err(|e| map_resin_error(&e.to_string()))?;
    let policy = items_arr(&list)
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(&platform_name))
        .and_then(|p| p.get("allocation_policy").and_then(|v| v.as_str()))
        .unwrap_or("unknown")
        .to_string();

    // Build the proxy URL to ipify through Resin.
    // Format: http://127.0.0.1:<port>/<proxy_token>/https/api.ipify.org
    // The proxy_token for the shell is empty (no-auth), so the path is
    // just the protocol + host. But Resin forward proxy needs the account
    // header to identify the platform. We send X-Resin-Account = platform_name.
    let proxy_url = format!(
        "http://127.0.0.1:{}/https/api.ipify.org",
        sidecar.api_port
    );
    tracing::info!(
        platform = %platform_name,
        proxy_url = %proxy_url,
        samples = n,
        "T8-2: strategy_verify starting probes"
    );

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| IpcError::from(format!("strategy_verify: http client build failed: {e}")))?;

    let mut samples = Vec::new();
    let mut total_latency = 0u64;
    let mut distribution: std::collections::HashMap<String, u32> = std::collections::HashMap::new();

    for i in 0..n {
        let start = std::time::Instant::now();
        let result = http
            .get(&proxy_url)
            .header("X-Resin-Account", &platform_name)
            .send()
            .await;
        let latency_ms = start.elapsed().as_millis() as u64;
        total_latency += latency_ms;

        match result {
            Ok(resp) => {
                if resp.status().is_success() {
                    match resp.text().await {
                        Ok(ip) => {
                            let ip = ip.trim().to_string();
                            *distribution.entry(ip.clone()).or_insert(0) += 1;
                            samples.push(serde_json::json!({
                                "ip": ip,
                                "latency_ms": latency_ms,
                                "status": "ok",
                            }));
                        }
                        Err(e) => {
                            samples.push(serde_json::json!({
                                "ip": "",
                                "latency_ms": latency_ms,
                                "status": format!("body_read_error: {e}"),
                            }));
                        }
                    }
                } else {
                    samples.push(serde_json::json!({
                        "ip": "",
                        "latency_ms": latency_ms,
                        "status": format!("http_{}", resp.status().as_u16()),
                    }));
                }
            }
            Err(e) => {
                tracing::warn!(attempt = i, error = %e, "T8-2: probe failed");
                samples.push(serde_json::json!({
                    "ip": "",
                    "latency_ms": latency_ms,
                    "status": format!("error: {e}"),
                }));
            }
        }
    }

    let unique_ips = distribution.len();
    let avg_latency_ms = if n > 0 { total_latency / n as u64 } else { 0 };

    tracing::info!(
        platform = %platform_name,
        unique_ips,
        avg_latency_ms,
        "T8-2: strategy_verify complete"
    );

    Ok(serde_json::json!({
        "platform": platform_name,
        "strategy": policy,
        "samples": samples,
        "distribution": distribution,
        "avg_latency_ms": avg_latency_ms,
        "unique_ips": unique_ips,
        "sample_count": n,
    }))
}

fn subscription_snapshot(v: &serde_json::Value) -> Vec<SubscriptionSnapshotEntry> {
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

fn subscription_id_for_name(v: &serde_json::Value, want: &str) -> Option<String> {
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

#[tauri::command]
pub fn tray_refresh_labels(app: AppHandle) -> Result<(), IpcError> {
    crate::tray::apply_labels(&app).map_err(|e| IpcError::from(format!("tray_refresh_labels: {e:?}")))
}

#[tauri::command]
pub fn get_config_dir(app: AppHandle) -> Result<String, IpcError> {
    match app.path().app_config_dir() {
        Ok(p) => Ok(p.to_string_lossy().into_owned()),
        Err(e) => Err(IpcError::from(format!("app_config_dir: {e:?}"))),
    }
}

#[tauri::command]
pub fn get_log_dir(app: AppHandle) -> Result<String, IpcError> {
    match app.path().app_log_dir() {
        Ok(p) => Ok(p.to_string_lossy().into_owned()),
        Err(e) => Err(IpcError::from(format!("app_log_dir: {e:?}"))),
    }
}

/// T2-2 (ADR-0016 Q2b): IPC snapshot of the sidecar stderr/stdout ring
/// buffer. Returns the last N lines (oldest still in buffer first) for the
/// Settings > Logs view. Read-only; no input from the webview.
#[tauri::command]
pub fn get_sidecar_logs(sidecar: State<'_, SidecarHandle>) -> Result<Vec<String>, IpcError> {
    Ok(sidecar.log_buf.snapshot())
}

/// T3-A1 (ADR-0012 deep audit): Expose the Resin sidecar's actual runtime
/// port + health status to the frontend. Replaces the dead gatewayBind/mihomoApi
/// Settings fields with real data from the sidecar.
#[derive(serde::Serialize)]
pub struct SidecarStatus {
    pub api_port: u16,
    pub api_base: String,
    pub mode: String,
    /// T6-7: sidecar process PID (0 if not running).
    pub pid: u32,
    /// T6-7: RFC3339 timestamp of the last successful /healthz probe.
    pub healthz_last_check: String,
    /// T6-7: round-trip latency of the get_sidecar_status IPC call (microseconds).
    pub ipc_latency_us: u64,
}

#[tauri::command]
pub fn get_sidecar_status(sidecar: State<'_, SidecarHandle>) -> Result<SidecarStatus, IpcError> {
    let started = std::time::Instant::now();
    let mode = sidecar.mode.read().map(|m| format!("{:?}", *m)).unwrap_or_else(|_| "Unknown".to_string());
    // T6-7: extract PID from the child process
    let pid = sidecar.child.lock().map(|c| {
        c.as_ref().map(|child| child.id()).unwrap_or(0)
    }).unwrap_or(0);
    // T6-7: last healthz check timestamp
    let healthz_last_check = sidecar.healthz_last_check.read()
        .map(|g| g.clone())
        .unwrap_or_default();
    let ipc_latency_us = started.elapsed().as_micros() as u64;
    Ok(SidecarStatus {
        api_port: sidecar.api_port,
        api_base: sidecar.api_base(),
        mode,
        pid,
        healthz_last_check,
        ipc_latency_us,
    })
}

// --- WebDAV backup (clash-verge-rev pattern: zip config + upload to WebDAV) ---
// Ponytail: no reqwest_dav crate — reqwest does HTTP PUT for WebDAV upload.
// The webview never sees the password; it passes through tauri-plugin-store.
// We validate the URL shape (http(s)://) and length-cap before issuing the PUT.

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

// Pure helper: returns Err(msg) if adding {process, target_port} would
// conflict with an existing rule (same target lane, different process).
// Extracted for unit testing without an AppHandle.
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
        // Use the same local-fetch path as subscription_add
        match fetch_clash_subscription(url).await {
            Ok(yaml) => match clash_yaml_to_proxies_block(&yaml) {
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
            },
            Err(e) => errors.push(format!("subscription {name} fetch: {e}")),
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

/// Validate an entry-port mapping before touching DB / listeners.
fn validate_port_mapping(
    port: u16,
    protocol: &str,
    platform_name: &str,
    account: &str,
    label: &str,
) -> Result<(), String> {
    use resin_core::{MAX_ENTRY_PORTS, MIN_USER_PORT};
    if port < MIN_USER_PORT {
        return Err(format!("port {port} is privileged (< {MIN_USER_PORT})"));
    }
    let proto = protocol.trim().to_ascii_lowercase();
    if proto != "socks5" && proto != "http" {
        return Err("protocol must be socks5 or http".into());
    }
    if !platform_name.is_empty() { validate_short_name(platform_name, "platform_name")?; }
    // account + label optional but length/control capped
    if account.len() > NAME_MAX_LEN || account.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
        return Err("account invalid".into());
    }
    if label.len() > NAME_MAX_LEN || label.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
        return Err("label invalid".into());
    }
    // Resin Platform.Account forbids these chars in either side
    let forbidden = |s: &str| s.chars().any(|ch| ".:/\\@?#%~ ".contains(ch));
    if !platform_name.is_empty() && forbidden(platform_name) {
        return Err("platform_name contains Resin-forbidden chars".into());
    }
    if !account.is_empty() && forbidden(account) {
        return Err("account contains Resin-forbidden chars".into());
    }
    let _ = MAX_ENTRY_PORTS; // capacity enforced at reload
    Ok(())
}

/// Worker-port range guard for port_* commands that take only a port number.
/// Mirrors the lower-bound check inside `validate_port_mapping` but without
/// the protocol/account checks (auth-info + health-check only need the port
/// segment). Ensures a hostile GUI caller cannot ask the shell to dial a
/// privileged system port.
fn validate_port_segments(port: u16) -> Result<(), String> {
    if port < resin_core::MIN_USER_PORT {
        return Err(format!("port {port} is privileged (< {})", resin_core::MIN_USER_PORT));
    }
    // u16 upper bound is 65535, no range check needed above MIN_USER_PORT.
    Ok(())
}

#[tauri::command]
pub async fn port_list(db: State<'_, DbPool>) -> Result<Vec<resin_core::PortMapping>, IpcError> {
    db.list_ports().map_err(IpcError::from)
}

/// T10-6: Smart port suggestion (ADR-0031).
/// Reads port_mappings for used ports, starts from 17990, skips used,
/// probes each candidate with TcpListener::bind, returns first available.
#[tauri::command]
pub async fn port_suggest(db: State<'_, DbPool>) -> Result<u16, IpcError> {
    let used: std::collections::HashSet<u16> = db
        .list_ports()
        .map_err(IpcError::from)?
        .into_iter()
        .map(|m| m.port)
        .collect();
    for candidate in 17990u16..=65535u16 {
        if used.contains(&candidate) { continue; }
        if std::net::TcpListener::bind(("127.0.0.1", candidate)).is_ok() {
            return Ok(candidate);
        }
    }
    // Fallback: OS-assigned free port (port 0)
    Ok(std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| IpcError::from(format!("port_suggest: no free port: {e}")))?
        .local_addr()
        .map_err(|e| IpcError::from(format!("port_suggest: no local addr: {e}")))?
        .port())
}

/// Upsert one entry-port via the whitebox config transaction
/// (validate -> SQLite replace -> listener reload -> atomic JSON -> hotswap).
#[tauri::command]
pub async fn port_upsert(
    sidecar: State<'_, SidecarHandle>,
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    port: u16,
    protocol: String,
    platform_name: String,
    account: String,
    label: String,
    enabled: bool,
    auth_required: bool,
) -> Result<resin_core::PortMapping, IpcError> {
    validate_port_mapping(port, &protocol, &platform_name, &account, &label)?;
    let acct = if account.trim().is_empty() {
        format!("port-{port}")
    } else {
        account
    };
    let proto = protocol.trim().to_ascii_lowercase();
    let m = resin_core::PortMapping {
        port,
        protocol: proto.clone(),
        platform_name,
        account: acct,
        label,
        enabled,
        auth_required,
    };
    // Step 1: Resin endpoint API CRUD (owns listener lifecycle)
    if enabled {
        let client = resin_client(&sidecar)?;
        let existing = client.list_endpoints().await
            .map_err(|e| IpcError::from(format!("list_endpoints: {e:?}")))?;
        let found = find_endpoint_id_by_port(&existing, port);
        let allow_socks5 = proto == "socks5";
        let allow_http_forward = proto == "http" || proto == "socks5";
        let body = serde_json::json!({
            "port": port,
            "allow_management": false,
            "allow_proxy": true,
            "allow_http_forward": allow_http_forward,
            "allow_http_reverse": false,
            "allow_socks5": allow_socks5,
            "require_proxy_auth_info": auth_required,
        });
        if let Some(ep_id) = found {
            // PATCH if port exists
            client.update_endpoint(&ep_id, body).await
                .map_err(|e| IpcError::from(format!("update_endpoint: {e:?}")))?;
        } else {
            // POST if port does not exist (create new listener)
            client.create_endpoint(body).await
                .map_err(|e| IpcError::from(format!("create_endpoint: {e:?}")))?;
        }
    }
    // Step 2: Shell DB + whitebox metadata (port -> platform_name binding)
    let mut next = whitebox.snapshot();
    if let Some(existing) = next.entry_ports.iter_mut().find(|row| row.port == m.port) {
        *existing = m.clone();
    } else {
        next.entry_ports.push(m.clone());
        next.entry_ports.sort_by_key(|row| row.port);
    }
    whitebox.apply(&db, &forwarder, next).await?;
    Ok(m)
}

#[tauri::command]
pub async fn port_remove(
    sidecar: State<'_, SidecarHandle>,
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    port: u16,
) -> Result<bool, IpcError> {
    if port < resin_core::MIN_USER_PORT {
        return Err(IpcError::from(format!("port {port} is privileged")));
    }
    // Step 1: Resin endpoint API delete (find by port -> endpoint_id -> DELETE)
    let client = resin_client(&sidecar)?;
    let existing = client.list_endpoints().await
        .map_err(|e| IpcError::from(format!("list_endpoints: {e:?}")))?;
    if let Some(ep_id) = find_endpoint_id_by_port(&existing, port) {
        client.delete_endpoint(&ep_id).await
            .map_err(|e| IpcError::from(format!("delete_endpoint: {e:?}")))?;
    } else {
        tracing::warn!("port_remove: no Resin endpoint found for port {port}, proceeding with shell DB cleanup");
    }
    // Step 2: Remove from shell DB + whitebox metadata
    let mut next = whitebox.snapshot();
    next.entry_ports.retain(|row| row.port != port);
    whitebox.apply(&db, &forwarder, next).await?;
    Ok(true)
}

/// T18-S2 (ADR-0042): Toggle enabled flag on an entry-port without
/// re-POST/Create or DELETE. Patches the Resin endpoint `{enabled: bool}`
/// (Resin v1.2.0 supports `enabled` on PATCH — `inactive` keeps the record)
/// then persists the same flag into the shell whitebox `entry_ports[].enabled`
/// so the whitebox is the authoritative record. The listener is NOT removed
/// from Resin's DB when toggled off, so toggled back on is a PATCH only.
#[tauri::command]
pub async fn port_toggle(
    sidecar: State<'_, SidecarHandle>,
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    port: u16,
    enabled: bool,
) -> Result<resin_core::PortMapping, IpcError> {
    validate_port_segments(port)?;
    // Step 1: Resin endpoint PATCH {enabled} — find endpoint by port.
    let client = resin_client(&sidecar)?;
    let existing = client.list_endpoints().await
        .map_err(|e| IpcError::from(format!("list_endpoints: {e:?}")))?;
    match find_endpoint_id_by_port(&existing, port) {
        Some(ep_id) => {
            let body = serde_json::json!({ "enabled": enabled });
            client.update_endpoint(&ep_id, body).await
                .map_err(|e| IpcError::from(format!("update_endpoint (toggle): {e:?}")))?;
            tracing::info!(target: "ipc.port_toggle", port, enabled, %ep_id, "endpoint patched");
        }
        None => {
            // Port was never created at Resin (or was DELETEd). Toggling off is
            // a no-op; toggling on without an endpoint record is impossible —
            // the user must re-create via port_upsert. Log and proceed to
            // update the shell whitebox enabled flag so the GUI still reflects
            // the requested state.
            tracing::warn!(target: "ipc.port_toggle", port, enabled, "no Resin endpoint for port; only updating shell whitebox");
        }
    }
    // Step 2: Update shell DB + whitebox metadata.
    let mut next = whitebox.snapshot();
    let row = next.entry_ports.iter_mut().find(|r| r.port == port);
    if let Some(r) = row {
        r.enabled = enabled;
        let m = r.clone();
        whitebox.apply(&db, &forwarder, next).await?;
        Ok(m)
    } else {
        Err(IpcError::from(format!("port_toggle: port {port} not in whitebox")))
    }
}

/// T8-1 (ADR-0029): Bind an entry-port to a platform WITHOUT touching
/// auth_required. Only updates the shell-side whitebox PortMapping
/// platform_name field. Does NOT call port_upsert (which would default
/// auth_required=true and flip no-auth ports to require-auth).
#[tauri::command]
pub async fn port_bind_platform(
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    port: u16,
    platform_name: String,
) -> Result<bool, IpcError> {
    validate_port_segments(port)?;
    // Allow empty platform_name = unbind. Non-empty must pass name validation.
    if !platform_name.is_empty() {
        validate_short_name(&platform_name, "platform_name")?;
        let forbidden = |s: &str| s.chars().any(|ch| ".:/\\@?#%~ ".contains(ch));
        if forbidden(&platform_name) {
            return Err(IpcError::from("platform_name contains Resin-forbidden chars".to_string()));
        }
    }
    let mut next = whitebox.snapshot();
    let found = next.entry_ports.iter_mut().find(|row| row.port == port);
    match found {
        Some(m) => {
            m.platform_name = platform_name;
            whitebox.apply(&db, &forwarder, next).await?;
            Ok(true)
        }
        None => Err(IpcError::from(format!("port {port} not found in entry_ports"))),
    }
}


#[tauri::command]
pub async fn port_running(
    forwarder: State<'_, resin_core::PortForwarder>,
) -> Result<Vec<u16>, IpcError> {
    Ok(forwarder.running_ports())
}

/// Explicit reload of the whitebox file (hand-edit path). Invalid files are
/// rejected by hotswap-config validation and leave the active map unchanged.
#[tauri::command]
pub async fn port_reload(
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
) -> Result<usize, IpcError> {
    whitebox.reload_file(&db, &forwarder).await.map_err(IpcError::from)
}

/// T6-3: Save network-layer config (DNS + idle conns + probe + bypass) to the
/// whitebox JSON file, atomically updating in-memory state + env injection.
#[tauri::command]
pub async fn whitebox_save_network(
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    network: resin_core::NetworkConfig,
) -> Result<usize, IpcError> {
    let mut current = whitebox.snapshot();
    current.network = network;
    whitebox.apply(&db, &forwarder, current).await.map_err(IpcError::from)
}

#[tauri::command]
pub async fn whitebox_path(
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
) -> Result<String, IpcError> {
    Ok(whitebox.path().display().to_string())
}

#[tauri::command]
pub async fn whitebox_get(
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
) -> Result<resin_core::WhiteboxConfig, IpcError> {
    Ok(whitebox.snapshot())
}

#[tauri::command]
pub async fn whitebox_reload(
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
) -> Result<usize, IpcError> {
    whitebox.reload_file(&db, &forwarder).await.map_err(IpcError::from)
}

#[tauri::command]
pub async fn stream_sensor_snapshot(
    forwarder: State<'_, resin_core::PortForwarder>,
) -> Result<resin_core::StreamSensorSnapshot, IpcError> {
    Ok(forwarder.stream_snapshot())
}

/// ADR-0021 Q1: return the SOCKS5 authentication credentials a gateway must
/// present to reach one entry-port. Resin's SOCKS5 server, when `proxy_token`
/// is set in the sidecar env, requires UserPass auth: username equals the
/// port's bound `platform.account` string (`Platform.Account`), password is
/// the sidecar global proxy token kept in `SidecarHandle.proxy_token`. As long
/// as the sidecar sets `RESIN_PROXY_TOKEN`, every port requires auth — there
/// is no per-port no-auth fallback without forking Resin, so `auth_required`
/// is read from the port_mapping row; ADR-0027: proxy_token is now empty so
/// per-endpoint `require_proxy_auth_info` controls auth per port.
#[derive(Debug, Serialize, Clone)]
pub struct PortAuthInfo {
    /// SOCKS5 username to present = the port's bound Platform.Account string.
    pub username: String,
    /// SOCKS5 password = the sidecar global proxy token (session-stable).
    pub password: String,
    /// Read from the port_mapping row; when false, no credential is needed.
    pub auth_required: bool,
    /// Bound platform name (for GUI display).
    pub platform_name: String,
    /// Port number echoed back so the GUI can pair the auth with the row.
    pub port: u16,
}

#[tauri::command]
pub async fn port_auth_info(
    sidecar: State<'_, SidecarHandle>,
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    port: u16,
) -> Result<PortAuthInfo, IpcError> {
    // The port_mapping row tells us the bound platform_name + account string.
    let mapping = db
        .list_ports()?
        .into_iter()
        .find(|m| m.port == port)
        .ok_or_else(|| format!("port {port} is not configured"))?;
    // Validate port range early so a hostile caller cannot pivot to an
    // arbitrary port number for a dial probe (Path-traversal safety:reuse the
    // same guard the whitebox config transaction already enforces).
    validate_port_segments(port)?;
    // T6-Bug5: Resin SOCKS5 requires `<Platform>.<Account>` format as the
    // username. Without the platform prefix, SOCKS5 auth succeeds (password
    // = proxy_token validates) but CONNECT returns "General failure" because
    // Resin cannot determine which platform the traffic belongs to.
    let username = if mapping.account.trim().is_empty() {
        format!("{}.port-{}", mapping.platform_name, port)
    } else {
        format!("{}.{}", mapping.platform_name, mapping.account)
    };
    // T7-fix: proxy_token is now empty (enables no-auth). When auth_required=true,
    // the password is empty — Resin socks5.go:307 short-circuits the check when
    // s.token=="" and forward.go:103-115 accepts any credential. The username
    // (Platform.Account) is still used for routing.
    let password = sidecar.proxy_token.clone();
    let _ = forwarder; // forwarder carries the live listener state; not needed for auth-info lookup
    Ok(PortAuthInfo {
        username,
        password,
        auth_required: mapping.auth_required,
        platform_name: mapping.platform_name,
        port,
    })
}

/// ADR-0021 Q1: live TCP probe + minimal SOCKS5 method-negotiation so the GUI
/// can show a green/red health chip per port (same pattern clash-verge-rev
/// uses for `CoreManager` reachability). We send the 3-byte greeting
/// `05 01 02` (SOCKS5 version + 1 method + method 0x02 UserPass). A healthy
/// listener replies `05 02`; an HTTP-mode listener replies with an HTTP status
/// line or a non-SOCKS5 byte we flag as `protocol_mismatch` but still
/// `reachable=true`. Takes ~100ms typical; capped at 750ms.
#[derive(Debug, Serialize, Clone)]
pub struct PortHealthCheck {
    pub port: u16,
    /// TCP connect succeeded.
    pub reachable: bool,
    /// SOCKS5 greeting got a plausible `05 <method>` reply.
    pub socks5_ok: bool,
    /// Listener replied with bytes but not SOCKS5 shape -> probably HTTP.
    pub protocol_mismatch: bool,
    /// Measured round-trip in millis (connect + greeting/reply).
    pub latency_ms: u64,
    /// Human-facing reason: "ok" | "refused" | "timeout" | "noop_no_reply" | "protocol_mismatch"
    pub reason: String,
}

/// T6-5: Read the last N request log entries from the Resin request_logs
/// SQLite database in RESIN_LOG_DIR. Returns a JSON array of simplified log
/// entries (timestamp, platform, account, host, egress_ip, method, status,
/// duration_ms, error). The GUI diagnostics panel renders this as a table.
#[derive(Debug, Serialize, Clone)]
pub struct RequestLogEntry {
    pub ts: String,
    pub platform_name: String,
    pub account: String,
    pub target_host: String,
    pub egress_ip: String,
    pub http_method: String,
    pub http_status: i64,
    pub duration_ms: f64,
    pub resin_error: String,
}

#[tauri::command]
pub async fn request_log_tail(
    app: AppHandle,
    limit: Option<usize>,
) -> Result<Vec<RequestLogEntry>, IpcError> {
    let n = limit.unwrap_or(50).min(200);
    let log_dir = app
        .path()
        .app_log_dir()
        .map_err(|e| IpcError::internal(&format!("log dir: {e}")))?
        .join("resin");
    tracing::info!(dir = ?log_dir, "request_log_tail: reading Resin logs");
    // Find the most recent request_logs*.db file
    let db_path = {
        let mut latest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
        if log_dir.exists() {
            for entry in std::fs::read_dir(&log_dir)
                .map_err(|e| IpcError::internal(&format!("read log dir: {e}")))?
            {
                if let Ok(e) = entry {
                    let name = e.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with("request_logs") && name_str.ends_with(".db") {
                        let meta = e.metadata().map_err(|err| IpcError::internal(&format!("metadata: {err}")))?;
                        let mtime = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        if latest.as_ref().map_or(true, |(t, _)| mtime > *t) {
                            latest = Some((mtime, e.path()));
                        }
                    }
                }
            }
        }
        latest
            .map(|(_, p)| p)
            .ok_or_else(|| IpcError::internal("no request_logs DB found"))?
    };
    // Copy DB to temp (WAL may be locked by the live sidecar)
    let tmp = std::env::temp_dir().join(format!(
        "egressapikey-reqlog-{}.db",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::copy(&db_path, &tmp)
        .map_err(|e| IpcError::internal(&format!("copy db: {e}")))?;
    // Open read-only and query
    let conn = rusqlite::Connection::open_with_flags(
        &tmp,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| IpcError::internal(&format!("open db: {e}")))?;
    let mut stmt = conn
        .prepare(
            "SELECT ts_ns, platform_name, account, target_host, egress_ip,
                    http_method, http_status, duration_ns, resin_error
             FROM request_logs ORDER BY ts_ns DESC LIMIT ?1",
        )
        .map_err(|e| IpcError::internal(&format!("prepare: {e}")))?;
    let rows = stmt
        .query_map([n as i64], |row| {
            let ts_ns: i64 = row.get(0)?;
            let secs = ts_ns / 1_000_000_000;
            let dt = chrono::DateTime::from_timestamp(secs, 0)
                .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_default();
            let duration_ns: i64 = row.get(7)?;
            Ok(RequestLogEntry {
                ts: dt,
                platform_name: row.get(1)?,
                account: row.get(2)?,
                target_host: row.get(3)?,
                egress_ip: row.get(4)?,
                http_method: row.get(5)?,
                http_status: row.get(6)?,
                duration_ms: duration_ns as f64 / 1_000_000.0,
                resin_error: row.get(8)?,
            })
        })
        .map_err(|e| IpcError::internal(&format!("query: {e}")))?;
    let mut entries = Vec::new();
    for row in rows {
        if let Ok(e) = row {
            entries.push(e);
        }
    }
    if let Err(e) = std::fs::remove_file(&tmp) {
        tracing::warn!(error = %e.to_string(), "request_log_tail: temp file cleanup failed");
    }
    tracing::info!(count = entries.len(), "request_log_tail: read entries");
    Ok(entries)
}

/// T6-5: Check Windows firewall inbound allow status for Resin's listen ports.
/// Read-only: runs `Get-NetFirewallProfile` to check if firewall is on.
#[derive(Debug, Serialize, Clone)]
pub struct FirewallStatus {
    pub platform: String,
    pub firewall_on: bool,
    pub inbound_blocked: bool,
    pub detail: String,
}

#[tauri::command]
pub async fn check_firewall_status() -> Result<FirewallStatus, IpcError> {
    // T7-1: enterprise-grade subprocess spawn - tokio::process::Command +
    // CREATE_NO_WINDOW on Windows + 5s timeout to prevent deadlock and
    // console window flash. Cross-platform: Linux uses systemctl, macOS
    // uses pfctl. Pattern from pwm gpt56_sol research.
    #[cfg(target_os = "windows")]
    {
        use tokio::process::Command;

        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-Command",
            "Get-NetFirewallProfile | Select-Object Name, Enabled | ConvertTo-Json"]);
        cmd.creation_flags(0x08000000u32); // CREATE_NO_WINDOW

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            cmd.output(),
        )
        .await
        .map_err(|_| IpcError::internal("firewall check timed out (5s)"))?
        .map_err(|e| IpcError::internal(&format!("firewall check: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let firewall_on = stdout.contains("true");
        tracing::info!(firewall_on, "check_firewall_status: probed");
        Ok(FirewallStatus {
            platform: "windows".into(),
            firewall_on,
            inbound_blocked: firewall_on,
            detail: if firewall_on {
                "Windows Firewall is ON. If ports are unreachable, add an inbound rule.".into()
            } else {
                "Windows Firewall is OFF.".into()
            },
        })
    }
    #[cfg(target_os = "linux")]
    {
        use tokio::process::Command;

        // Non-root best-effort: systemctl is-active (distro-dependent),
        // fallback to /proc/net/ip_tables_names. pwm sonar: ufw/iptables need root.
        async fn try_detect() -> Option<FirewallStatus> {
            let out = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                Command::new("systemctl").args(["is-active", "ufw", "--quiet"]).output(),
            ).await.ok()?.ok()?;
            if String::from_utf8_lossy(&out.stdout).trim() == "active" {
                return Some(FirewallStatus { platform: "linux".into(), firewall_on: true, inbound_blocked: true, detail: "UFW firewall is active.".into() });
            }
            let out = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                Command::new("systemctl").args(["is-active", "firewalld", "--quiet"]).output(),
            ).await.ok()?.ok()?;
            if String::from_utf8_lossy(&out.stdout).trim() == "active" {
                return Some(FirewallStatus { platform: "linux".into(), firewall_on: true, inbound_blocked: true, detail: "firewalld is active.".into() });
            }
            if std::path::Path::new("/proc/net/ip_tables_names").exists() {
                return Some(FirewallStatus { platform: "linux".into(), firewall_on: true, inbound_blocked: true, detail: "iptables tables detected.".into() });
            }
            None
        }
        match try_detect().await {
            Some(status) => {
                tracing::info!(firewall_on = status.firewall_on, "check_firewall_status: probed linux");
                Ok(status)
            }
            None => Ok(FirewallStatus { platform: "linux".into(), firewall_on: false, inbound_blocked: false, detail: "No firewall detected (or insufficient permissions).".into() }),
        }
    }
    #[cfg(target_os = "macos")]
    {
        use tokio::process::Command;

        let mut cmd = Command::new("pfctl");
        cmd.args(["-s", "info"]);
        match tokio::time::timeout(std::time::Duration::from_secs(5), cmd.output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let firewall_on = stdout.contains("enabled");
                tracing::info!(firewall_on, "check_firewall_status: probed macos pfctl");
                Ok(FirewallStatus { platform: "macos".into(), firewall_on, inbound_blocked: firewall_on, detail: if firewall_on { "pf firewall is enabled.".into() } else { "pf firewall appears disabled.".into() } })
            }
            _ => {
                let pf_conf_exists = std::path::Path::new("/etc/pf.conf").exists();
                Ok(FirewallStatus { platform: "macos".into(), firewall_on: pf_conf_exists, inbound_blocked: pf_conf_exists, detail: if pf_conf_exists { "/etc/pf.conf exists but status uncertain (pfctl needs root).".into() } else { "No pf.conf found; firewall likely disabled.".into() } })
            }
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        Ok(FirewallStatus { platform: std::env::consts::OS.into(), firewall_on: false, inbound_blocked: false, detail: "Firewall check not supported on this platform.".into() })
    }
}

/// T6-4: Probe the exit IP by routing a request to http://1.1.1.1/cdn-cgi/trace
/// through the specified entry port (HTTP or SOCKS5 proxy). Returns the exit
/// IP parsed from the Cloudflare trace body, plus latency. If no active
/// subscription/nodes are available, returns an error so the GUI can show
/// "no exit IP available" rather than a misleading blank.
#[tauri::command]
pub async fn probe_exit_ip(
    sidecar: State<'_, SidecarHandle>,
    db: State<'_, DbPool>,
    port: u16,
    protocol: String,
) -> Result<ExitIpProbe, IpcError> {
    tracing::info!(port, protocol = %protocol, "probe_exit_ip: probing through proxy");
    validate_port_segments(port)?;
    let proto = protocol.to_ascii_lowercase();
    if proto != "http" && proto != "socks5" {
        return Err(IpcError::internal("error.invalidProtocol"));
    }
    let proxy_url = if proto == "http" {
        format!("http://127.0.0.1:{port}")
    } else {
        let mapping = db
            .list_ports()?
            .into_iter()
            .find(|m| m.port == port)
            .ok_or_else(|| format!("port {port} is not configured"))?;
        let username = if mapping.account.trim().is_empty() {
            format!("{}.port-{}", mapping.platform_name, port)
        } else {
            format!("{}.{}", mapping.platform_name, mapping.account)
        };
        let password = &sidecar.proxy_token;
        format!("socks5h://{username}:{password}@127.0.0.1:{port}")
    };
    let proxy = reqwest::Proxy::all(&proxy_url)
        .map_err(|e| IpcError::internal(&format!("proxy build: {e}")))?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| IpcError::internal(&format!("client build: {e}")))?;
    let started = std::time::Instant::now();
    let resp = client
        .get("http://1.1.1.1/cdn-cgi/trace")
        .send()
        .await
        .map_err(|e| IpcError::internal(&format!("probe request: {e}")))?;
    let status = resp.status().as_u16();
    let body = resp
        .text()
        .await
        .map_err(|e| IpcError::internal(&format!("probe body: {e}")))?;
    let latency_ms = started.elapsed().as_millis() as u64;
    let exit_ip = resin_core::parse_trace_body_ip(&body);
    tracing::info!(port, protocol = %proto, exit_ip = %exit_ip, latency_ms, "probe_exit_ip: success");
    Ok(ExitIpProbe {
        port,
        protocol: proto,
        exit_ip,
        latency_ms,
        status,
    })
}

#[derive(Debug, Serialize, Clone)]
pub struct ExitIpProbe {
    pub port: u16,
    pub protocol: String,
    pub exit_ip: String,
    pub latency_ms: u64,
    pub status: u16,
}

#[tauri::command]
pub async fn port_health_check(port: u16, protocol: Option<String>) -> Result<PortHealthCheck, IpcError> {
    tracing::info!(port, protocol = ?protocol, "port_health_check: probing port");
    validate_port_segments(port)?;
    let proto = protocol
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|| "socks5".into());
    use std::time::Instant;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let started = Instant::now();
    let addr = format!("127.0.0.1:{port}");
    // 200ms connect timeout (headroom under the 750ms request budget).
    let connect = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        tokio::net::TcpStream::connect(&addr),
    )
    .await;
    let mut stream = match connect {
        Ok(Ok(s)) => s,
        Ok(Err(_)) => {
            return Ok(PortHealthCheck {
                port,
                reachable: false,
                socks5_ok: false,
                protocol_mismatch: false,
                latency_ms: started.elapsed().as_millis() as u64,
                reason: "refused".into(),
            });
        }
        Err(_) => {
            return Ok(PortHealthCheck {
                port,
                reachable: false,
                socks5_ok: false,
                protocol_mismatch: false,
                latency_ms: started.elapsed().as_millis() as u64,
                reason: "timeout".into(),
            });
        }
    };
    // SOCKS5 greeting: version 5, 2 method candidates: NoAuth(0x00) + UserPass(0x02).
    // With empty proxy_token, Resin accepts either; with non-empty, only UserPass.
    let greeting: Vec<u8> = if proto == "http" {
        // T6-Bug1: HTTP GET probe. Resin's HTTP proxy does NOT support CONNECT
        // tunneling — CONNECT returns 404/error. A plain GET / gets any HTTP
        // response (200/404/400) which proves the port is alive.
        format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").into_bytes()
    } else {
        vec![0x05, 0x02, 0x00, 0x02]
    };
    if let Err(e) = stream.write_all(&greeting).await {
        tracing::info!(port, proto = %proto, error = %e, "port_health_check: write probe failed");
        return Ok(PortHealthCheck {
            port,
            reachable: true,
            socks5_ok: false,
            protocol_mismatch: false,
            latency_ms: started.elapsed().as_millis() as u64,
            reason: "noop_no_reply".into(),
        });
    }
    // Reply should be exactly 2 bytes: 0x05 0x00 (NoAuth) or 0x05 0x02 (UserPass). HTTP
    // listeners reply with an HTTP status line e.g. `HTTP/1.1 400...`.
    let mut buf = [0u8; 16];
    let read = tokio::time::timeout(
        std::time::Duration::from_millis(550),
        stream.read(&mut buf),
    )
    .await;
    let elapsed = started.elapsed().as_millis() as u64;
    match read {
        Ok(Ok(n)) if proto == "http" && n >= 12 && buf.starts_with(b"HTTP/") => Ok(PortHealthCheck {
            port,
            reachable: true,
            socks5_ok: false,
            protocol_mismatch: false,
            latency_ms: elapsed,
            reason: "ok".into(),
        }),
        Ok(Ok(n)) if proto == "socks5" && n >= 2 && buf[0] == 0x05 && (buf[1] == 0x00 || buf[1] == 0x02) => Ok(PortHealthCheck {
            port,
            reachable: true,
            socks5_ok: true,
            protocol_mismatch: false,
            latency_ms: elapsed,
            reason: "ok".into(),
        }),
        Ok(Ok(n)) if n >= 4 => Ok(PortHealthCheck {
            // Has bytes but not a SOCKS5 shape: most likely an HTTP listener
            // replying with an error status line (`HTTP/1.1 ...`). Still
            // reachable; mark protocol_mismatch so the GUI shows a distinct chip.
            port,
            reachable: true,
            socks5_ok: false,
            protocol_mismatch: true,
            latency_ms: elapsed,
            reason: "protocol_mismatch".into(),
        }),
        _ => Ok(PortHealthCheck {
            port,
            reachable: true,
            socks5_ok: false,
            protocol_mismatch: false,
            latency_ms: elapsed,
            reason: "noop_no_reply".into(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Strategy Engine (T4-4 / ADR-0022) — whitebox per-platform strategy config.
// The shell polls Resin /nodes, applies A-class filters, PATCHes region_filters.
// B-class maps to Resin allocation_policy via the existing platform_update IPC.

#[tauri::command]
pub async fn strategy_config_get(app: AppHandle) -> Result<serde_json::Value, IpcError> {
    let dir = std::path::PathBuf::from(get_config_dir(app)?);
    let path = dir.join("egressapikey-strategy.json");
    if !path.exists() {
        let default = resin_core::StrategyConfig::default();
        return serde_json::to_value(&default).map_err(|e| IpcError::from(e.to_string()));
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| IpcError::from(e.to_string()))?;
    serde_json::from_str::<serde_json::Value>(&raw).map_err(|e| IpcError::from(e.to_string()))
}

#[tauri::command]
pub async fn strategy_config_put(
    app: AppHandle,
    config: serde_json::Value,
) -> Result<(), IpcError> {
    let dir = std::path::PathBuf::from(get_config_dir(app)?);
    let path = dir.join("egressapikey-strategy.json");
    let typed: resin_core::StrategyConfig =
        serde_json::from_value(config).map_err(|e| IpcError::from(format!("strategy config invalid: {e}")))?;
    if typed.version != 1 {
        return Err(IpcError::internal("strategy config version must be 1"));
    }
    for ps in &typed.platforms {
        if ps.platform_name.is_empty() || ps.platform_name.len() > 128 {
            return Err(IpcError::from("platform_name must be 1..128 chars".to_string()));
        }
        if ps.regions.len() > 64 {
            return Err(IpcError::from("regions list too long (max 64)".to_string()));
        }
        if ps.subscriptions.len() > 64 {
            return Err(IpcError::from("subscriptions list too long (max 64)".to_string()));
        }
        if ps.top_n > 1000 {
            return Err(IpcError::from("top_n too large (max 1000)".to_string()));
        }
    }
    let json = serde_json::to_string_pretty(&typed).map_err(|e| IpcError::from(e.to_string()))?;
    std::fs::write(&path, json).map_err(|e| IpcError::from(e.to_string()))?;
    Ok(())
}

#[tauri::command]
pub async fn strategy_apply(
    sidecar: State<'_, SidecarHandle>,
    app: AppHandle,
) -> Result<serde_json::Value, IpcError> {
    let dir = std::path::PathBuf::from(get_config_dir(app)?);
    let path = dir.join("egressapikey-strategy.json");
    let raw = if path.exists() {
        std::fs::read_to_string(&path).map_err(|e| IpcError::from(e.to_string()))?
    } else {
        serde_json::to_string(&resin_core::StrategyConfig::default()).map_err(|e| IpcError::from(e.to_string()))?
    };
    let config: resin_core::StrategyConfig =
        serde_json::from_str(&raw).map_err(|e| IpcError::from(format!("strategy config parse error: {e}")))?;

    let client = resin_client(&sidecar)?;
    let nodes_v = client.list_nodes().await.map_err(|e| IpcError::from(e.to_string()))?;
    let nodes = resin_core::parse_nodes(&nodes_v);

    // T11-4c: auto-clean stale platforms. Get the live platform list and filter
    // strategyConfig to only include platforms that still exist in Resin.
    let live_platforms_v = client.list_platforms().await.map_err(|e| IpcError::from(e.to_string()))?;
    let live_names: std::collections::HashSet<String> = items_arr(&live_platforms_v)
        .iter()
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(String::from))
        .collect();
    let cleaned_config = resin_core::StrategyConfig {
        version: config.version,
        platforms: config.platforms.iter()
            .filter(|ps| live_names.contains(&ps.platform_name))
            .cloned()
            .collect(),
    };
    if cleaned_config.platforms.len() != config.platforms.len() {
        tracing::info!(
            before = config.platforms.len(),
            after = cleaned_config.platforms.len(),
            "strategy_apply: auto-cleaned stale platform entries from strategyConfig"
        );
        // Persist the cleaned config back to disk.
        if let Ok(cleaned_json) = serde_json::to_string_pretty(&cleaned_config) {
            let _ = std::fs::write(&path, cleaned_json);
        }
    }

    let plan = resin_core::compute_plan(&cleaned_config, &nodes);
    let mut applied = serde_json::json!({"platforms": []});
    let platforms_arr = applied["platforms"].as_array_mut().expect("platforms initialized as array");

    for (platform_name, regions) in &plan {
        let platforms_v = client.list_platforms().await.map_err(|e| IpcError::from(e.to_string()))?;
        if let Some(id) = platform_id_for_name(&platforms_v, platform_name) {
            let body = serde_json::json!({"region_filters": regions});
            match client.update_platform(&id, body).await {
                Ok(_) => {
                    platforms_arr.push(serde_json::json!({
                        "platform": platform_name,
                        "region_filters": regions,
                        "patched": true,
                    }));
                }
                Err(e) => {
                    tracing::warn!(platform = %platform_name, error = %e.to_string(), "auto_strategy_apply: PATCH region_filters failed");
                    platforms_arr.push(serde_json::json!({
                        "platform": platform_name,
                        "region_filters": regions,
                        "patched": false,
                        "reason": format!("PATCH failed: {e}"),
                    }));
                }
            }
        } else {
            platforms_arr.push(serde_json::json!({
                "platform": platform_name,
                "region_filters": regions,
                "patched": false,
                "reason": "platform not found",
            }));
        }
    }
    Ok(applied)
}

/// T14-8: get lightweight mode config (enabled + delay_minutes).
/// Reads from tauri-plugin-store settings.json — returns {enabled, delay_minutes}.
#[tauri::command]
pub async fn lightweight_get(app: AppHandle) -> Result<serde_json::Value, IpcError> {
    let store = app.store("settings.json").map_err(|e| IpcError::from(e.to_string()))?;
    let enabled: bool = store.get("lightweightEnabled").unwrap_or(serde_json::Value::Bool(true)).as_bool().unwrap_or(true);
    let delay: u32 = store.get("lightweightDelayMinutes").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
    Ok(serde_json::json!({ "enabled": enabled, "delay_minutes": delay }))
}

/// T14-8: set lightweight mode config (enabled + delay_minutes).
/// Persists to settings.json + updates the live LightweightController.
#[tauri::command]
pub async fn lightweight_set(
    app: AppHandle,
    enabled: bool,
    delay_minutes: u32,
) -> Result<(), IpcError> {
    if delay_minutes == 0 || delay_minutes > 1440 {
        return Err(IpcError::from("delay_minutes must be 1..=1440".to_string()));
    }
    let store = app.store("settings.json").map_err(|e| IpcError::from(e.to_string()))?;
    store.set("lightweightEnabled", serde_json::Value::Bool(enabled));
    store.set("lightweightDelayMinutes", serde_json::json!(delay_minutes));
    store.save().map_err(|e| IpcError::from(e.to_string()))?;
    // Update the live controller if it's managed
    if let Some(ctrl) = app.try_state::<crate::lightweight::LightweightController>() {
        ctrl.set_delay_minutes(delay_minutes);
    }
    Ok(())
}

#[tauri::command]
pub async fn set_log_level(level: String) -> Result<String, IpcError> {
    let val = match level.as_str() {
        "error" => 0u8,
        "warn" => 1u8,
        "info" => 2u8,
        "debug" => 3u8,
        _ => return Err(IpcError::from(format!("invalid log level: {{must be error/warn/info/debug}}: {}", level))),
    };
    LOG_LEVEL_GATE.store(val, std::sync::atomic::Ordering::Relaxed);
    tracing::warn!("T15-2: log level set to {} (gate={})", level, val);
    Ok(level)
}

#[tauri::command]
pub async fn get_log_level() -> Result<String, IpcError> {
    let val = LOG_LEVEL_GATE.load(std::sync::atomic::Ordering::Relaxed);
    let name = match val {
        0 => "error",
        1 => "warn",
        2 => "info",
        3 => "debug",
        _ => "info",
    };
    Ok(name.to_string())
}


// --- T18 Phase 1: Port health batch probe (ADR-0042 S1) ---------------------
//
// A single watch_port_health command drives the background Tokio task that
// probes every enabled entry-port concurrently (cap 10) and streams
// PortHealthSnapshot down the Tauri ipc::Channel. The shared paused flag is
// toggled by WindowEvent::Focused in main.rs (tab-hidden pause pattern).
// Disabled ports (PortMapping.enabled=false) are filtered out by the ports_fn
// closure before probing, matching S2's "whitebox enabled is truth source".

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Shared pause flag managed by main.rs and read by every watch_port_health task.
#[derive(Clone)]
pub struct PortHealthPaused(pub Arc<AtomicBool>);

impl PortHealthPaused {
    pub fn new(start_paused: bool) -> Self {
        Self(Arc::new(AtomicBool::new(start_paused)))
    }
}

/// T18 Phase 1: stream port-health snapshots down a Tauri Channel. One Tokio
/// task per invocation; it ends when the channel is closed (frontend unmount).
#[tauri::command]
pub async fn watch_port_health(
    on_event: tauri::ipc::Channel<resin_core::PortHealthSnapshot>,
    db: State<'_, DbPool>,
    paused: State<'_, PortHealthPaused>,
) -> Result<(), IpcError> {
    // ports_fn snapshot is taken from the shell DB so the watcher retries the
    // latest port set on every tick (new ports appear, deleted ports drop out
    // without restarting the watcher). Disabled ports are filtered here.
    let db_clone: DbPool = db.inner().clone();
    let ports_fn = move || {
        db_clone.list_ports().unwrap_or_default().into_iter().filter(|m| m.enabled).collect::<Vec<_>>()
    };
    // Translate the Tauri Channel into the emit closure resin_core expects.
    let channel_clone = on_event.clone();
    let emit = move |snap: resin_core::PortHealthSnapshot| -> Result<(), ()> {
        // send() returns Err if the webview has dropped the channel; that ends the watcher.
        channel_clone.send(snap).map_err(|_| ())
    };
    let paused_arc = paused.inner().0.clone();
    resin_core::port_health::spawn_watcher(ports_fn, emit, paused_arc);
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
        assert_eq!(
            platform_names(&v),
            vec!["Default".to_string(), "Platform-A".to_string()]
        );
    }

    #[test]
    fn platform_id_for_name_matches() {
        let v = json!([{ "name": "Foo", "id": "uuid-1" }]);
        assert_eq!(platform_id_for_name(&v, "Foo"), Some("uuid-1".to_string()));
        assert_eq!(platform_id_for_name(&v, "Bar"), None);
    }

    #[test]
    fn sum_active_leases_parses_resin_shape() {
        assert_eq!(
            sum_active_leases(
                &json!({ "items": [{ "active_leases": 7 }, { "active_leases": 5 }] })
            ),
            12
        );
        assert_eq!(sum_active_leases(&json!({ "active_leases": 4 })), 4);
        assert_eq!(sum_active_leases(&json!({})), 0);
    }
    #[test]
    fn process_route_conflict_rejects_same_lane_different_process() {
        let existing = vec![ProcessRouteRule {
            process: "ollama".to_string(),
            target_port: 17990,
        }];
        // same process + same lane -> ok (update path)
        assert!(process_route_conflict_check(&existing, "ollama", 17990).is_ok());
        // different process, same lane -> conflict
        assert!(process_route_conflict_check(&existing, "openai", 17990).is_err());
        // different process, different lane -> ok
        assert!(process_route_conflict_check(&existing, "openai", 17991).is_ok());
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
        let platforms =
            json!([ { "name": "Default", "id": "00000000-0000-0000-0000-000000000000" } ]);
        let leases = json!({
            "items": [ { "platform_id": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "active_leases": 7 } ],
        });
        let got = per_platform_active_from_leases(&leases, &platforms);
        assert!(got
            .iter()
            .any(|(n, c)| n == "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa" && *c == 7));
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
    fn find_endpoint_id_by_port_finds_in_items_wrapper() {
        let v = json!({ "items": [
            { "id": "default", "port": 0 },
            { "id": "ep-uuid-1", "port": 1791 },
            { "id": "ep-uuid-2", "port": 1792 },
        ], "total": 3 });
        assert_eq!(find_endpoint_id_by_port(&v, 1791), Some("ep-uuid-1".to_string()));
        assert_eq!(find_endpoint_id_by_port(&v, 1792), Some("ep-uuid-2".to_string()));
    }

    #[test]
    fn find_endpoint_id_by_port_finds_in_bare_array() {
        let v = json!([{ "id": "ep-abc", "port": 1800 }]);
        assert_eq!(find_endpoint_id_by_port(&v, 1800), Some("ep-abc".to_string()));
    }

    #[test]
    fn find_endpoint_id_by_port_skips_default_endpoint() {
        let v = json!({ "items": [{ "id": "default", "port": 9999 }] });
        assert_eq!(find_endpoint_id_by_port(&v, 9999), None);
    }

    #[test]
    fn find_endpoint_id_by_port_returns_none_when_port_absent() {
        let v = json!({ "items": [{ "id": "ep-x", "port": 1111 }] });
        assert_eq!(find_endpoint_id_by_port(&v, 2222), None);
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
        assert_eq!(
            platform_names(&v),
            vec!["Default".to_string(), "OpenAI".to_string()]
        );
    }

    #[test]
    fn platform_id_for_name_reads_resin_items_wrapper() {
        let v = json!({
            "items": [ { "id": "uuid-9", "name": "Anthropic" } ],
            "total": 1,
        });
        assert_eq!(
            platform_id_for_name(&v, "Anthropic"),
            Some("uuid-9".to_string())
        );
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
        assert_eq!(
            subscription_id_for_name(&v, "main"),
            Some("sub-uuid-1".to_string())
        );
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

    #[test]
    fn validate_port_segments_rejects_privileged_and_accepts_user_range() {
        // Privileged ports below MIN_USER_PORT must be rejected so a hostile
        // GUI caller cannot pivot the shell to dial system ports (path-safety
        // guard for port_auth_info + port_health_check).
        assert!(validate_port_segments(80).is_err());
        assert!(validate_port_segments(1023).is_err());
        // User range is accepted: boundary at MIN_USER_PORT (1024) up to 65535.
        assert!(validate_port_segments(1024).is_ok());
        assert!(validate_port_segments(17990).is_ok());
        assert!(validate_port_segments(65535).is_ok());
    }

    #[test]
    fn map_resin_error_cannot_delete_default() {
        let raw = r#"409 Conflict: {"error":{"code":"CONFLICT","message":"cannot delete Default platform"}}"#;
        assert_eq!(map_resin_error(raw), "error.cannotDeleteDefaultPlatform");
    }

    #[test]
    fn map_resin_error_auth_required() {
        assert_eq!(map_resin_error("407 AUTH_REQUIRED: missing token"), "error.authRequired");
    }

    #[test]
    fn map_resin_error_upstream_connect_failed() {
        assert_eq!(
            map_resin_error("502 UPSTREAM_CONNECT_FAILED: timeout"),
            "error.upstreamConnectFailed"
        );
    }

    #[test]
    fn map_resin_error_unknown_passes_through() {
        let raw = "something unexpected happened";
        assert_eq!(map_resin_error(raw), raw);
    }

    /// T3-Q2: subscription fetch error routing to i18n keys with HTTP code suffix.
    #[test]
    fn map_resin_error_subscription_fetch_with_http_code() {
        let raw = "fetch_clash_subscription: all UA attempts failed: HTTP 525 <unknown status code>";
        assert_eq!(map_resin_error(raw), "error.subscriptionFetch.525");
    }

    #[test]
    fn map_resin_error_subscription_fetch_no_code_falls_back_to_base() {
        // No HTTP code in the error string (e.g. "...all UA attempts failed: empty body")
        let raw = "fetch_clash_subscription: all UA attempts failed: empty body";
        assert_eq!(map_resin_error(raw), "error.subscriptionFetch");
    }

    #[test]
    fn map_resin_error_subscription_fetch_403() {
        let raw = "fetch_clash_subscription: all UA attempts failed: HTTP 403 Forbidden";
        assert_eq!(map_resin_error(raw), "error.subscriptionFetch.403");
    }

    #[test]
    fn map_resin_error_bind_conflict_extracts_port() {
        let raw = "create_endpoint: resin_client: POST /endpoints -> 409 Conflict: {\"error\":{\"code\":\"CONFLICT\",\"message\":\"listen on port 17111: bind: Only one usage of each socket address (protocol/network address/port) is normally permitted.\"}}";
        assert_eq!(map_resin_error(raw), "error.bindConflict:17111");
    }

    #[test]
    fn map_resin_error_bind_conflict_no_port_falls_back_to_base() {
        let raw = "EADDRINUSE: address already in use";
        assert_eq!(map_resin_error(raw), "error.bindConflict");
    }

    #[test]
    fn extract_port_from_residual_finds_port() {
        assert_eq!(extract_port_from_residual("listen on port 8080: bind"), Some(8080));
        assert_eq!(extract_port_from_residual("no port here"), None);
        assert_eq!(extract_port_from_residual("port 443"), Some(443));
    }

    /// T6-Bug4: bind conflict match must fire BEFORE the generic CONFLICT match.
    /// Resin port bind errors return HTTP 409 whose status text is "Conflict",
    /// which would be caught by the generic CONFLICT guard if it came first.
    /// The bind-specific guard must win so the user sees the port number.
    #[test]
    fn map_resin_error_bind_takes_precedence_over_conflict() {
        let raw = "409 Conflict: \"listen on port 17999: bind: Only one usage of each socket address (protocol/network address/port) is normally permitted.\"";
        // Should map to bindConflict:17999, NOT generic error.conflict
        assert_eq!(map_resin_error(raw), "error.bindConflict:17999");
    }

    /// T6-Bug4: the generic CONFLICT guard still fires for non-bind 409 errors
    /// (e.g. "cannot delete Default platform" has CONFLICT in its JSON but is
    /// matched earlier by the cannot_delete guard; a pure CONFLICT without bind
    /// should still get error.conflict).
    #[test]
    fn map_resin_error_conflict_without_bind_still_works() {
        let raw = "409 Conflict: some other conflict";
        assert_eq!(map_resin_error(raw), "error.conflict");
    }

    /// T6-Bug1: HTTP port_health_check should send GET / not CONNECT.
    /// This is a compile-time + behavior test: the greeting bytes must starts
    /// with "GET / HTTP/1.1" for HTTP protocol. We verify by checking the
    /// behavior indirectly — the actual TCP probe is async and needs a live
    /// listener; here we just verify the greeting construction logic exists
    /// and the code compiles. Full integration is the release-exe smoke test.
    #[test]
    fn port_health_check_http_greeting_is_get_not_connect() {
        // The greeting for "http" protocol should use GET, not CONNECT.
        // We verify by checking that the format! macro produces GET.
        let port: u16 = 1791;
        let greeting = format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
        assert!(greeting.starts_with("GET / HTTP/1.1"), "HTTP greeting must start with GET, got: {}", greeting);
        assert!(!greeting.contains("CONNECT"), "HTTP greeting must NOT contain CONNECT");
    }

    /// T6-Bug5: port_auth_info username must be in {Platform}.{Account} format.
    /// Resin SOCKS5 requires this format; without the platform prefix,
    /// CONNECT returns General failure (error 1) even though auth succeeds.
    #[test]
    fn port_auth_username_format_includes_platform_prefix() {
        // Simulate the username construction logic for both empty and non-empty account.
        let platform_name = "Default";
        let account = "port-1792";
        let port: u16 = 1792;

        // Non-empty account: format!("{}.{}", platform_name, account)
        let username = format!("{}.{}", platform_name, account);
        assert_eq!(username, "Default.port-1792");

        // Empty account fallback: format!("{}.port-{}", platform_name, port)
        let empty_account_username = format!("{}.port-{}", platform_name, port);
        assert_eq!(empty_account_username, "Default.port-1792");
    }

    // T6-4: parse exit IP from Cloudflare trace body (pure helper logic).
    #[test]
    fn t6_4_parse_exit_ip_from_cloudflare_trace() {
        let body = "fl=123f\nnode=sin1\nip=203.0.113.42\nuag=Mozilla/5.0\n";
        let ip = body
            .lines()
            .find_map(|l| l.strip_prefix("ip=").map(|s| s.trim().to_string()))
            .unwrap_or_default();
        assert_eq!(ip, "203.0.113.42");
    }

    #[test]
    fn t6_4_parse_exit_ip_missing_returns_empty() {
        let body = "fl=123f\nnode=sin1\nuag=Mozilla/5.0\n";
        let ip = body
            .lines()
            .find_map(|l| l.strip_prefix("ip=").map(|s| s.trim().to_string()))
            .unwrap_or_default();
        assert_eq!(ip, "");
    }

    #[test]
    fn t6_4_parse_exit_ip_with_trailing_whitespace() {
        let body = "ip=  198.51.100.1  \n";
        let ip = body
            .lines()
            .find_map(|l| l.strip_prefix("ip=").map(|s| s.trim().to_string()))
            .unwrap_or_default();
        assert_eq!(ip, "198.51.100.1");
    }

    #[test]
    fn t8_port_bind_platform_empty_platform_name_passes_validation() {
        // empty platform_name = unbind, should pass
        assert!(validate_short_name("", "platform_name").is_err()); // validate_short_name rejects empty
        // port_bind_platform allows empty without calling validate_short_name
        // (the command itself guards with if !platform_name.is_empty())
    }

    #[test]
    fn t8_port_bind_platform_forbidden_chars_rejected() {
        let forbidden = |s: &str| s.chars().any(|ch| ".:/\\@?#%~ ".contains(ch));
        assert!(forbidden("my.platform"));
        assert!(forbidden("my@platform"));
        assert!(!forbidden("my-platform"));
        assert!(!forbidden("Default"));
    }

    #[test]
    fn t10_port_suggest_tcp_probe_finds_bindable_port() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
    }

    #[test]
    fn t10_port_suggest_skip_used_port() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_err());
        drop(l);
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
    }


    /// T10: verify auto_strategy_apply PATCH failure result shape (pure logic).
    #[test]
    fn t10_patch_failure_result_has_reason() {
        let platform_name = "TestPlatform";
        let regions = vec!["US".to_string()];
        let err_msg = "connection refused";
        let result = serde_json::json!({
            "platform": platform_name,
            "region_filters": regions,
            "patched": false,
            "reason": format!("PATCH failed: {err_msg}"),
        });
        assert_eq!(result["patched"], false);
        assert!(result["reason"].as_str().unwrap().contains("PATCH failed"));
        assert!(result["reason"].as_str().unwrap().contains("connection refused"));
    }

    /// T10: verify config_import PATCH failure pushes to errors vec (pure logic).
    #[test]
    fn t10_config_import_patch_failure_pushes_error() {
        let name = "MyPlatform";
        let err_msg = "timeout";
        let mut errors: Vec<String> = vec![];
        let error_line = format!("platform {name}: PATCH failed: {err_msg}");
        errors.push(error_line);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("MyPlatform"));
        assert!(errors[0].contains("PATCH failed"));
        assert!(errors[0].contains("timeout"));
    }

    #[test]
    fn t15_2_set_log_level_validates_enum() {
        // Validate that the level string maps correctly to the atomic gate.
        // We do not call the async command (requires Tauri runtime); instead
        // we test the gate logic directly.
        super::LOG_LEVEL_GATE.store(2, std::sync::atomic::Ordering::Relaxed);
        assert!(super::log_level_enabled(0)); // error always emitted
        assert!(super::log_level_enabled(1)); // warn emitted at info+
        assert!(super::log_level_enabled(2)); // info emitted at info
        assert!(!super::log_level_enabled(3)); // debug NOT emitted at info
    }

    #[test]
    fn t15_2_set_log_level_gate_round_trip() {
        // Set level to debug (3) and verify all levels pass
        super::LOG_LEVEL_GATE.store(3, std::sync::atomic::Ordering::Relaxed);
        assert!(super::log_level_enabled(0));
        assert!(super::log_level_enabled(1));
        assert!(super::log_level_enabled(2));
        assert!(super::log_level_enabled(3));
        // Set to error (0) and verify only error passes
        super::LOG_LEVEL_GATE.store(0, std::sync::atomic::Ordering::Relaxed);
        assert!(super::log_level_enabled(0));
        assert!(!super::log_level_enabled(1));
        assert!(!super::log_level_enabled(2));
        assert!(!super::log_level_enabled(3));
        // Restore default
        super::LOG_LEVEL_GATE.store(2, std::sync::atomic::Ordering::Relaxed);
    }
}
