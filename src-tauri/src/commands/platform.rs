//! platform domain IPC commands (EgressAPIKEY).
//!
//! Extracted from the former commands/mod.rs monolith by
//! pure mechanical move - no behavior, naming, or IPC-surface change.
use super::common::{
    items_arr, map_resin_error, resin_client, validate_ip, validate_short_name, KEY_MAX_LEN,
};
use crate::sidecar::SidecarHandle;
use resin_core::{
    parse_public_ips, resolve_id_in, ReputationClient, ReputationProvider, ReputationSnapshot,
    MAX_LANES,
};
use resin_core::{DbPool, IpcError};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_store::StoreExt;

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
    // name→UUID two-step hop now lives in ResinClient
    // (resolve_platform_id_by_name); miss = typed IpcError::NotFound.
    let id = client.resolve_platform_id_by_name(&name).await?;
    client
        .delete_platform(&id)
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
    Ok(true)
}

#[tauri::command]
pub async fn platform_list(sidecar: State<'_, SidecarHandle>) -> Result<Vec<String>, IpcError> {
    let client = resin_client(&sidecar)?;
    let list = client
        .list_platforms()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
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
    client
        .list_platforms()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))
}

/// @deprecated since ADR-0050: account semantics are owned by the Resin
/// sidecar (the forward proxy anchors sticky leases per account). The shell
/// only validates input and echoes `Ok` — no behavior. Kept solely for IPC
/// contract compatibility (no in-repo caller; see AGENTS.md §7.6 echo list).
#[tauri::command]
pub async fn account_add(
    sidecar: State<'_, SidecarHandle>,
    platform: String,
    id: String,
    lane: usize,
) -> Result<(), IpcError> {
    tracing::warn!(target: "ipc.account_deprecated", "account_add is deprecated since ADR-0050 (Resin owns account semantics); manage accounts via Resin directly");
    validate_short_name(&platform, "platform")?;
    validate_short_name(&id, "account")?;
    if lane >= MAX_LANES {
        return Err(IpcError::from(format!(
            "lane {lane} out of range (max {})",
            MAX_LANES - 1
        )));
    }
    let _client = resin_client(&sidecar)?;
    Ok(())
}

/// @deprecated since ADR-0050: account semantics are owned by the Resin
/// sidecar (sticky egress IPs are managed on the Resin side). The shell
/// only validates input and echoes `true` — no behavior. Kept solely for
/// IPC contract compatibility (no in-repo caller; see AGENTS.md §7.6 echo list).
#[tauri::command]
pub async fn account_bind_ip(
    sidecar: State<'_, SidecarHandle>,
    platform: String,
    account: String,
    ip: String,
) -> Result<bool, IpcError> {
    tracing::warn!(target: "ipc.account_deprecated", "account_bind_ip is deprecated since ADR-0050 (Resin owns account semantics); manage accounts via Resin directly");
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

// the former platform_id_for_name /
// subscription_id_for_name duplicates are deleted — the name→UUID two-step
// hop now lives once in resin-core (resin_client::resolve_id_in for the
// fn-pointer seams, ResinClient::resolve_*_by_name for the command bodies).

/// ADR-0055 D2: the ONLY write entry for process routes. The
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
    process_route_add_impl(&db, &forwarder, &whitebox, process, target_port).await
}

/// Transport-free body (R11-03): the whitebox write entry is identical on
/// both transports.
pub async fn process_route_add_impl(
    db: &DbPool,
    forwarder: &resin_core::PortForwarder,
    whitebox: &resin_core::WhiteboxConfigStore,
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
    whitebox.apply(db, forwarder, next).await?;
    Ok(())
}

/// ADR-0055 D2: remove by process name (case-insensitive).
/// Returns false when no rule matched (nothing was written).
#[tauri::command]
pub async fn process_route_remove(
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    process: String,
) -> Result<bool, IpcError> {
    process_route_remove_impl(&db, &forwarder, &whitebox, process).await
}

/// Transport-free body (R11-03).
pub async fn process_route_remove_impl(
    db: &DbPool,
    forwarder: &resin_core::PortForwarder,
    whitebox: &resin_core::WhiteboxConfigStore,
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
    whitebox.apply(db, forwarder, next).await?;
    Ok(true)
}

/// ADR-0055 D2: read the whitebox route family.
#[tauri::command]
pub async fn process_route_list(
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
) -> Result<Vec<resin_core::ProcessRouteRule>, IpcError> {
    Ok(whitebox.snapshot().process_routes)
}

// ── Account header rules (ADR-0063) ──────────────────
// Thin IPC facades over the ResinClient account-header-rules family
// (R32-R35). These are L3 pass-through reads/writes on the Resin
// control plane — NO L2 whitebox file, no snapshot field, no reconcile
// action (coexistence legislation in ADR-0063). §7.5: every string
// input is length-capped and control-char-rejected BEFORE reaching
// ResinClient (which re-validates defensively).

/// §7.5 shared gate for account-header-rule string inputs: DNS-host
/// style cap (253) + reject NUL/control characters.
fn validate_rule_string(value: &str, field: &str) -> Result<(), IpcError> {
    if value.trim().is_empty() {
        return Err(IpcError::from(format!(
            "account_header_rules: {field} must be non-empty"
        )));
    }
    if value.len() > 253 {
        return Err(IpcError::from(format!(
            "account_header_rules: {field} length out of range (1..=253, got {})",
            value.len()
        )));
    }
    if value.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
        return Err(IpcError::from(format!(
            "account_header_rules: {field} contains control characters"
        )));
    }
    Ok(())
}

#[tauri::command]
pub async fn list_account_header_rules(
    sidecar: State<'_, SidecarHandle>,
    keyword: Option<String>,
) -> Result<serde_json::Value, IpcError> {
    if let Some(k) = &keyword {
        if k.len() > 253 {
            return Err(IpcError::from(
                "account_header_rules: keyword length out of range (<=253)".to_string(),
            ));
        }
        if k.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
            return Err(IpcError::from(
                "account_header_rules: keyword contains control characters".to_string(),
            ));
        }
    }
    let client = resin_client(&sidecar)?;
    client
        .list_account_header_rules(keyword.as_deref())
        .await
        .map_err(|e| map_resin_error(&e.to_string()))
}

#[tauri::command]
pub async fn put_account_header_rules(
    sidecar: State<'_, SidecarHandle>,
    url_prefix: String,
    headers: Vec<String>,
) -> Result<serde_json::Value, IpcError> {
    validate_rule_string(&url_prefix, "url_prefix")?;
    if headers.is_empty() {
        return Err(IpcError::from(
            "account_header_rules: headers must be a non-empty array".to_string(),
        ));
    }
    if headers.len() > 64 {
        return Err(IpcError::from(
            "account_header_rules: headers length out of range (1..=64)".to_string(),
        ));
    }
    for h in &headers {
        validate_rule_string(h, "header")?;
    }
    let client = resin_client(&sidecar)?;
    client
        .put_account_header_rules(&url_prefix, &headers)
        .await
        .map_err(|e| map_resin_error(&e.to_string()))
}

#[tauri::command]
pub async fn resolve_account_header_rule(
    sidecar: State<'_, SidecarHandle>,
    url: String,
) -> Result<serde_json::Value, IpcError> {
    // resolve URL is an absolute http(s) URL, not a DNS host: cap at 2048.
    if url.trim().is_empty() {
        return Err(IpcError::from(
            "account_header_rules: url must be non-empty".to_string(),
        ));
    }
    if url.len() > 2048 {
        return Err(IpcError::from(
            "account_header_rules: url length out of range (1..=2048)".to_string(),
        ));
    }
    if url.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
        return Err(IpcError::from(
            "account_header_rules: url contains control characters".to_string(),
        ));
    }
    // §7.6 URL convention: absolute http(s) only; Resin's
    // parseHTTPAbsoluteURL would reject anything else anyway.
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(IpcError::from(
            "account_header_rules: url must be an absolute http(s) URL".to_string(),
        ));
    }
    let client = resin_client(&sidecar)?;
    client
        .resolve_account_header_rule(&url)
        .await
        .map_err(|e| map_resin_error(&e.to_string()))
}

#[tauri::command]
pub async fn delete_account_header_rule(
    sidecar: State<'_, SidecarHandle>,
    url_prefix: String,
) -> Result<serde_json::Value, IpcError> {
    validate_rule_string(&url_prefix, "url_prefix")?;
    let client = resin_client(&sidecar)?;
    client
        .delete_account_header_rule(&url_prefix)
        .await
        .map_err(|e| map_resin_error(&e.to_string()))
}

#[tauri::command]
pub async fn subscription_add(
    sidecar: State<'_, SidecarHandle>,
    app: AppHandle,
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    name: String,
    url: String,
    update_interval: Option<String>,
    pipeline: Option<String>,
    default_port: Option<u16>,
) -> Result<(), IpcError> {
    validate_short_name(&name, "subscription")?;
    if url.trim().is_empty() {
        return Err(IpcError::from(
            "subscription url must be non-empty".to_string(),
        ));
    }
    if url.len() > KEY_MAX_LEN {
        return Err(IpcError::from("subscription url out of range".to_string()));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(IpcError::from(
            "subscription url must start with http:// or https://".to_string(),
        ));
    }
    // validate update_interval Go duration format (default 30s).
    // 30s is Resin's enforced floor (>= 30s,
    // resin/internal/service/control_plane_subscription.go:130, create AND
    // PATCH); a 5s default was investigated and rejected — a below-floor
    // value 400s on POST. Gap documented in docs/research/OPENAPI-GAP.md.
    let update_interval = update_interval.unwrap_or_else(|| "30s".to_string());
    if update_interval.len() > 10
        || update_interval
            .bytes()
            .any(|b| b == 0 || b < 0x20 || b == 0x7f)
    {
        return Err(IpcError::from(
            "update_interval: invalid (max 10 chars, no control)".to_string(),
        ));
    }
    // the user's explicit binding target for the
    // cascade's optional default-port tail. Provided -> the suggest probe is
    // skipped. §7.5 numeric boundary: reject privileged ports here so a
    // hostile caller cannot burn the pipeline's retry budget on a
    // deterministic rejection.
    if let Some(dp) = default_port {
        if dp < resin_core::MIN_USER_PORT {
            return Err(IpcError::from(format!(
                "default_port {dp} is privileged (< {})",
                resin_core::MIN_USER_PORT
            )));
        }
    }
    let client = resin_client(&sidecar)?;

    // POST source_type=remote directly (ADR-0045). Resin's Scheduler
    // re-pulls the remote URL via its own clash.meta UA fetcher
    // (cmd/resin/main.go const downloadUserAgent); the shell no longer
    // re-fetches or converts the Clash YAML itself (P13 B4 chain deleted).
    // update_interval default 30s — the 30s scheduler tick lands the first
    // background fetch within seconds, so the import path itself does not need
    // to force one. (Resin DOES expose a synchronous force-refresh:
    // POST /api/v1/subscriptions/{id}/actions/refresh
    // — resin/internal/api/server.go:108 -> HandleRefreshSubscription ->
    // RefreshSubscription -> Scheduler.UpdateSubscription. The
    // `subscription_refresh` command below uses it. The earlier "Resin has no
    // public force-refresh endpoint" note here was stale.)
    tracing::info!(subscription = %name, url = %url, "subscription_add: POST source_type=remote");
    let body = serde_json::json!({
        "name": name.clone(),
        "source_type": "remote",
        "url": url.clone(),
        "update_interval": update_interval,
    });
    match client.create_subscription(body).await {
        Ok(v) => {
            tracing::info!(?v, "subscription_add: Resin accepted subscription");
            // pipeline=establish opts INTO the
            // five-step cascade (resolve -> whitebox platform -> apply).
            // The enqueue + drain is the user-triggered reconcile — no
            // background loop (ADR-0054 discipline). Without the parameter
            // the behavior is exactly the legacy import-only POST.
            if pipeline.as_deref() == Some("establish") {
                let svc = super::strategy::strategy_service(&app)?;
                let pipeline_state = app.state::<SubscriptionPipelineState>();
                // Level-triggered enqueue (coalesces per name; bounded queue).
                // A partial failure is persistent state inside the queue
                // (backoff + parking); the IPC still returns Ok — the import
                // itself succeeded. The drain is the single reconciler pass,
                // running on this command task: no background loop owns it
                // (ADR-0054 user-triggered discipline).
                if pipeline_state.0.enqueue(resin_core::EstablishEvent {
                    subscription: name.clone(),
                    url: url.clone(),
                }) {
                    let reports = pipeline_state
                        .0
                        .drain(&client, &svc, resin_core::whitebox_backup::now_unix())
                        .await;
                    // the cascade's OPTIONAL default-port
                    // tail — only after a GREEN establish pass, and only when
                    // the user did not provide a binding target. Conflicts
                    // (already-bound platform, taken port, foreign Resin
                    // listener) are warnings inside the step itself; the
                    // import result stays Ok either way.
                    // Scope the tail to THIS call's subscription: a drain
                    // may also re-run other queued events, whose tail runs at
                    // their own subscription_add (level-triggered per call).
                    if reports.iter().any(|r| r.subscription == name && r.all_ok()) {
                        let port_status = resin_core::subscription_pipeline::ensure_default_port(
                            &client,
                            &db,
                            &forwarder,
                            &whitebox,
                            &name,
                            default_port,
                        )
                        .await;
                        tracing::info!(subscription = %name, ?port_status, "subscription_add: default-port tail settled");
                    }
                }
            }
            Ok(())
        }
        Err(e) => {
            tracing::warn!(error = ?e, "subscription_add: Resin POST failed");
            Err(IpcError::from(e.to_string()))
        }
    }
}

/// process-local pipeline queue managed as Tauri state
/// (same pattern as DRIFT_NOTIFY_STATE / LightweightController — one owner,
/// no background task; drain runs on the command task that enqueued).
#[derive(Default)]
pub struct SubscriptionPipelineState(pub resin_core::SubscriptionPipeline);

#[tauri::command]
pub async fn subscription_remove(
    sidecar: State<'_, SidecarHandle>,
    db: State<'_, DbPool>,
    forwarder: State<'_, resin_core::PortForwarder>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    name: String,
) -> Result<bool, IpcError> {
    validate_short_name(&name, "subscription")?;
    let client = resin_client(&sidecar)?;
    // name→UUID two-step hop centralized in ResinClient; the
    // list GET count is unchanged and a miss resolves to typed NotFound.
    let id = client.resolve_subscription_id_by_name(&name).await?;
    client
        .delete_subscription(&id)
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
    // Reverse tail (ticket 05, A-008 / Round 9 D-007): release the default
    // entry port the establish cascade bound to this subscription's
    // platform. The subscription is already deleted; a failed release is a
    // WARNING (the leftover surfaces as drift for port_remove), never an
    // error — the removal itself succeeded.
    let released = resin_core::subscription_pipeline::remove_default_port_if_orphaned(
        &client, &db, &forwarder, &whitebox, &name,
    )
    .await;
    if !released {
        tracing::warn!(
            subscription = %name,
            "subscription_remove: default-port reverse tail failed; entry port may surface as drift"
        );
    }
    Ok(true)
}

/// trigger Resin-native subscription refresh by POST /actions/refresh.
/// source_type must be "remote" (ADR-0045); a local-source subscription re-parses
/// in-memory content on /actions/refresh (no HTTP, no new upstream nodes).
/// The paired subscription_add migrated to source_type=remote so refresh
/// actually pulls new nodes. Resin's Scheduler re-fetches via its own clash.meta
/// UA fetcher (cmd/resin/main.go const downloadUserAgent); the shell no longer
/// re-fetches or converts the Clash YAML itself (P13 B4 chain deleted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionRefreshResult {
    pub node_count: u64,
    pub changed: bool,
}

pub fn extract_sub_row_stats(item: &serde_json::Value) -> (u64, Option<serde_json::Value>) {
    let count = item.get("node_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let version = item
        .get("node_version")
        .cloned()
        .or_else(|| item.get("config_version").cloned());
    (count, version)
}

/// Returns the post-refresh node_count and changed status so the UI can show
/// an accurate toast and avoid closure stale-read bugs.
/// Waits up to 5 attempts (500ms interval) for Resin's diff/apply to settle.
pub fn subscription_refresh_changed_changed(
    initial: u64,
    initial_version: Option<&serde_json::Value>,
    latest: u64,
    latest_version: Option<&serde_json::Value>,
) -> bool {
    if latest != initial {
        return true;
    }
    match (initial_version, latest_version) {
        (Some(prev), Some(curr)) if prev != curr => true,
        _ => false,
    }
}

#[tauri::command]
pub async fn subscription_refresh(
    sidecar: State<'_, SidecarHandle>,
    name: String,
) -> Result<SubscriptionRefreshResult, IpcError> {
    validate_short_name(&name, "subscription")?;
    let client = resin_client(&sidecar)?;
    let initial_list = client
        .list_subscriptions()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
    // the name→id hop goes through the shared pure resolver.
    // The initial row stats are extracted from the SAME list response, so
    // the request shape (one GET before the refresh POST) is unchanged.
    let id = resolve_id_in(&initial_list, &name)
        .ok_or_else(|| IpcError::not_found(&format!("subscription not found: {name}")))?;

    let initial_item = items_arr(&initial_list)
        .into_iter()
        .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(id.as_str()));
    let (initial_count, initial_ver) = initial_item
        .as_ref()
        .map(|it| extract_sub_row_stats(it))
        .unwrap_or((0, None));

    tracing::info!(subscription = %name, id = %id, initial_count, "subscription_refresh: POST /actions/refresh");
    client
        .refresh_subscription_native(&id)
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;

    // Poll list_subscriptions until node_count or node_version changes (up to 5 retries, 500ms each).
    let max_attempts = 5;
    let poll_delay = std::time::Duration::from_millis(500);
    let mut final_count = initial_count;
    let mut changed = false;

    for attempt in 1..=max_attempts {
        let latest_list = match client.list_subscriptions().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(subscription = %name, attempt, error = %e, "subscription_refresh: poll list_subscriptions failed");
                tokio::time::sleep(poll_delay).await;
                continue;
            }
        };

        if let Some(target) = items_arr(&latest_list)
            .into_iter()
            .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
        {
            let (latest_count, latest_ver) = extract_sub_row_stats(&target);
            final_count = latest_count;
            if subscription_refresh_changed_changed(
                initial_count,
                initial_ver.as_ref(),
                latest_count,
                latest_ver.as_ref(),
            ) {
                changed = true;
                tracing::info!(
                    subscription = %name,
                    attempt,
                    initial_count,
                    latest_count,
                    "subscription_refresh: detected subscription state change"
                );
                break;
            }
        }

        if attempt < max_attempts {
            tokio::time::sleep(poll_delay).await;
        }
    }

    tracing::info!(
        subscription = %name,
        node_count = final_count,
        changed,
        "subscription_refresh: action settled"
    );
    Ok(SubscriptionRefreshResult {
        node_count: final_count,
        changed,
    })
}

#[derive(Debug, Serialize)]
pub struct SubscriptionSnapshotEntry {
    pub name: String,
    /// Resin `url` — the remote source this subscription pulls from. Surfaced
    /// so the GUI can re-drive a failed establish cascade: the Failed chip's
    /// retry affordance re-enqueues `EstablishEvent { subscription, url }`,
    /// and this is the ONLY name→url hop on the wire (the whitebox status row
    /// is name-only and the local view cache is name-less).
    pub url: String,
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
    client
        .node_pool_snapshot()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))
}

/// The allocation_policy values Resin v1.1.2 actually accepts (probed
/// 2026-07-31). The IPC layer rejects anything else before reaching Resin.
pub const ALLOWED_ALLOCATION_POLICIES: &[&str] =
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
    passive_circuit_breaker_disabled: Option<bool>,
) -> Result<serde_json::Value, IpcError> {
    validate_short_name(&name, "platform")?;
    let client = resin_client(&sidecar)?;
    // name→UUID resolution centralized in ResinClient (F2);
    // same single list_platforms GET as before, miss = typed NotFound.
    let id = client.resolve_platform_id_by_name(&name).await?;

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
            return Err(IpcError::from(
                "regex_filters: too many entries (max 64)".to_string(),
            ));
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
            return Err(IpcError::from(
                "region_filters: too many entries (max 64)".to_string(),
            ));
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
            return Err(IpcError::from(
                "sticky_ttl: invalid (max 32 chars, no control)".to_string(),
            ));
        }
        body.insert(
            "sticky_ttl".to_string(),
            serde_json::Value::String(ttl.clone()),
        );
    }
    // passive_circuit_breaker_disabled — platform-level boolean.
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
        return Err(IpcError::from(
            "platform_update: no fields to update".to_string(),
        ));
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
    client
        .list_nodes()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))
}

/// pure input validation for node_probe. Exposed as a module-level
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

/// on-demand node probe. Forwards to Resin's native
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
    validate_node_probe_inputs(&node_hash, &kind).map_err(IpcError::from)?;
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
            return Err(IpcError::from(
                "regex_filters: too many entries (max 64)".to_string(),
            ));
        }
        for f in arr {
            if let Some(s) = f.as_str() {
                if s.len() > 253 || s.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
                    return Err(IpcError::from(
                        "regex_filters: entry invalid (max 253 chars, no control)".to_string(),
                    ));
                }
            }
        }
    }
    // If region_filters present, cap each at 16 chars (ISO 3166-1 alpha-2 + negation).
    if let Some(arr) = obj.get("region_filters").and_then(|v| v.as_array()) {
        if arr.len() > 64 {
            return Err(IpcError::from(
                "region_filters: too many entries (max 64)".to_string(),
            ));
        }
        for r in arr {
            if let Some(s) = r.as_str() {
                if s.len() > 16
                    || s.bytes()
                        .any(|b| b == 0 || b < 0x20 || b == 0x7f || b == b' ')
                {
                    return Err(IpcError::from(
                        "region_filter invalid (max 16, no control/space)".to_string(),
                    ));
                }
            }
        }
    }
    // If sticky_ttl present, cap at 32 chars + no control (mirrors platform_update).
    if let Some(ttl) = obj.get("sticky_ttl").and_then(|v| v.as_str()) {
        if ttl.len() > 32 || ttl.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
            return Err(IpcError::from(
                "sticky_ttl: invalid (max 32 chars, no control)".to_string(),
            ));
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
    // name→UUID two-step hop centralized in ResinClient; miss
    // = typed IpcError::NotFound instead of a stringly error round-trip.
    let id = client.resolve_platform_id_by_name(&name).await?;
    client
        .platform_leases(&id)
        .await
        .map_err(|e| map_resin_error(&e.to_string()))
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
                // Resin's SubscriptionResponse already carries `url`
                // (resin/internal/service/control_plane_subscription.go:28);
                // the shell projection used to drop it.
                let url = p
                    .get("url")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(SubscriptionSnapshotEntry {
                    name: name.to_string(),
                    url,
                    node_count,
                    healthy_node_count,
                    last_error,
                    last_checked,
                })
            }
        })
        .collect()
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
    let raw = client
        .active_leases()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
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
    let client = resin_client(&sidecar)?;
    ip_reputation_snapshot_impl(|k| store.get(k), &client).await
}

/// Transport-free body (R11-03): the headless BFF reads the same settings
/// keys from its own settings.json store.
pub async fn ip_reputation_snapshot_impl(
    get_setting: impl Fn(&str) -> Option<serde_json::Value>,
    client: &resin_core::ResinClient,
) -> Result<ReputationSnapshot, IpcError> {
    let provider_name =
        get_setting("ipReputationProvider").and_then(|v| v.as_str().map(str::to_string));
    let Some(provider) = provider_name.as_deref().and_then(ReputationProvider::parse) else {
        return Ok(ReputationSnapshot {
            provider: None,
            status: "disabled".into(),
            entries: Vec::new(),
        });
    };
    let api_key = if provider.requires_key() {
        get_setting(provider.key_name()).and_then(|v| v.as_str().map(str::to_string))
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
    let raw = client
        .active_leases()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
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
