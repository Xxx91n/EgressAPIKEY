//! strategy domain IPC commands (EgressAPIKEY).
//!
//! Extracted from the former commands/mod.rs monolith by architecture-recovery
//! ticket 08: pure mechanical move - no behavior, naming, or IPC-surface change.
use tauri::{AppHandle, State};
use crate::sidecar::SidecarHandle;
use resin_core::DbPool;
use resin_core::IpcError;
use super::common::{items_arr, map_resin_error, resin_client, validate_short_name};
use super::settings::{get_config_dir};
use super::platform::{platform_id_for_name};

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

/// Architecture-recovery ticket 07: the authoritative effective-config
/// snapshot (CONTEXT.md: Authoritative Snapshot; ARCHITECTURE.md §Config
/// Authority). ONE call reads the three configuration sources and merges them
/// at the single sanctioned merge point in resin-core:
///   L2 whitebox strategy intent  <- egressapikey-strategy.json
///   L2 whitebox ports + partner  <- WhiteboxConfigStore + egressapikey.db
///   L3 Resin runtime             <- GET /api/v1/platforms + /api/v1/endpoints
/// Views consume the result and must not re-merge stores themselves. This is
/// a read-only command; the Read Retry contract applies to the Resin GETs.
/// The B-class plan uses the same compute_plan entry as strategy_apply, so
/// the snapshot reports the regions the NEXT apply would produce.
#[tauri::command]
pub async fn authoritative_snapshot(
    sidecar: State<'_, SidecarHandle>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    db: State<'_, DbPool>,
    app: AppHandle,
) -> Result<resin_core::AuthoritativeSnapshot, IpcError> {
    // L2 strategy whitebox: file is the truth (ADR-0036); missing file = defaults.
    let dir = std::path::PathBuf::from(get_config_dir(app)?);
    let strategy_path = dir.join("egressapikey-strategy.json");
    let config: resin_core::StrategyConfig = if strategy_path.exists() {
        let raw = std::fs::read_to_string(&strategy_path)
            .map_err(|e| IpcError::from(e.to_string()))?;
        serde_json::from_str(&raw)
            .map_err(|e| IpcError::from(format!("strategy config parse error: {e}")))?
    } else {
        resin_core::StrategyConfig::default()
    };

    // L2 ports whitebox + its SQLite sync partner. The whitebox file is the
    // truth source (ADR-0042 S2); DB rows that are absent from the whitebox
    // snapshot (boot-seed temp-store fallback path) are still surfaced so the
    // partner drift stays visible instead of silently disappearing.
    let whitebox_cfg = whitebox.snapshot();
    let db_ports = db.list_ports().map_err(IpcError::from)?;
    let wb_port_set: std::collections::HashSet<u16> =
        whitebox_cfg.entry_ports.iter().map(|m| m.port).collect();
    let mut all_ports = whitebox_cfg.entry_ports.clone();
    all_ports.extend(db_ports.into_iter().filter(|m| !wb_port_set.contains(&m.port)));

    // L3 Resin runtime. A sidecar that is not Running reports an empty,
    // unreachable runtime (resin_reachable=false) instead of failing the
    // snapshot: the whitebox half is still assertable while the sidecar is down.
    let reachable = sidecar.mode() == crate::sidecar::RunningMode::Running;
    let (mut resin_platforms, resin_endpoint_ports) = if reachable {
        let client = resin_client(&sidecar)?;
        let platforms_v = client
            .list_platforms()
            .await
            .map_err(|e| map_resin_error(&e.to_string()))?;
        let endpoints_v = client
            .list_endpoints()
            .await
            .map_err(|e| map_resin_error(&e.to_string()))?;
        (
            resin_core::snapshot::parse_resin_platforms(&platforms_v),
            endpoint_ports(&endpoints_v),
        )
    } else {
        (vec![], vec![])
    };
    resin_platforms.sort_by(|a, b| a.name.cmp(&b.name));

    // A-class plan for each whitebox platform, identical to strategy_apply.
    let nodes_v = if reachable {
        let client = resin_client(&sidecar)?;
        let v = client
            .list_nodes()
            .await
            .map_err(|e| map_resin_error(&e.to_string()))?;
        resin_core::parse_nodes(&v)
    } else {
        vec![]
    };
    let plan = resin_core::compute_plan(&config, &nodes_v);

    let platforms = resin_core::snapshot::merge_strategies(&config, &resin_platforms, &plan, strategy_path.exists());
    let ports = resin_core::snapshot::merge_ports(&all_ports, &resin_endpoint_ports);
    Ok(resin_core::AuthoritativeSnapshot {
        strategy_version: config.version,
        platforms,
        ports,
        resin_reachable: reachable,
    })
}

/// Extract the set of listener ports from a GET /api/v1/endpoints response.
/// Both the {"items":[..]} wrapper and bare-array shapes are accepted; the
/// read-only "default" endpoint is included because a listener exists there.
pub fn endpoint_ports(existing: &serde_json::Value) -> Vec<u16> {
    let arr = if let Some(a) = existing.get("items").and_then(|i| i.as_array()) {
        a.as_slice()
    } else if let Some(a) = existing.as_array() {
        a.as_slice()
    } else {
        &[]
    };
    let mut ports: Vec<u16> = arr
        .iter()
        .filter_map(|ep| ep.get("port").and_then(|p| p.as_u64()))
        .filter(|p| *p <= u16::MAX as u64)
        .map(|p| p as u16)
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}
