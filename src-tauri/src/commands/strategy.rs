//! strategy domain IPC commands (EgressAPIKEY).
//!
//! Extracted from the former commands/mod.rs monolith by
//! pure mechanical move - no behavior, naming, or IPC-surface change.
use super::common::{items_arr, map_resin_error, resin_client, validate_short_name};
use super::settings::get_config_dir;
use crate::sidecar::SidecarHandle;
use resin_core::resolve_id_in;
use resin_core::DbPool;
use resin_core::IpcError;
use tauri::{AppHandle, Manager, State};

/// ADR-0054 §C: process-local first-drift memory backing
/// `divergentSince`. Keyed by entity id (platform name, or decimal port
/// number for ports), valued by the Unix second the entity FIRST entered a
/// drift state within THIS process. Deliberately NOT persisted: a restart
/// clears it, so the next drift observation re-times from zero
/// "重启进程后清零"). Cleared per-entity when the snapshot reports the
/// entity consistent; never enters the three-state merge itself.
static DRIFT_MEMORY: once_cell::sync::Lazy<std::sync::Mutex<resin_core::snapshot::DriftMemory>> =
    once_cell::sync::Lazy::new(|| {
        std::sync::Mutex::new(resin_core::snapshot::DriftMemory::default())
    });

/// Strategy verification — send N probe requests through the Resin
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
    let list = client
        .list_platforms()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
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
    let proxy_url = format!("http://127.0.0.1:{}/https/api.ipify.org", sidecar.api_port);
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

/// Thin IPC facade over `resin_core::StrategyService` (ADR-0052): the
/// Service owns read/validate/store; the command validates only the IPC
/// boundary (AGENTS 7.5) and maps errors. No JSON assembly here.
#[tauri::command]
pub async fn strategy_config_get(app: AppHandle) -> Result<serde_json::Value, IpcError> {
    let svc = strategy_service(&app)?;
    let config = svc.get().map_err(IpcError::from)?;
    serde_json::to_value(&config).map_err(|e| IpcError::from(e.to_string()))
}

#[tauri::command]
pub async fn strategy_config_put(
    app: AppHandle,
    config: serde_json::Value,
) -> Result<(), IpcError> {
    let svc = strategy_service(&app)?;
    let typed: resin_core::StrategyConfig = serde_json::from_value(config)
        .map_err(|e| IpcError::from(format!("strategy config invalid: {e}")))?;
    // ADR-0058: store() bumps generation and stamps updated_at.
    svc.store(typed).map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command]
pub async fn strategy_apply(
    sidecar: State<'_, SidecarHandle>,
    app: AppHandle,
) -> Result<serde_json::Value, IpcError> {
    let svc = strategy_service(&app)?;
    let client = resin_client(&sidecar)?;
    let report = svc
        .apply(&client, resolve_id_in)
        .await
        .map_err(IpcError::from)?;
    serde_json::to_value(&report).map_err(|e| IpcError::from(e.to_string()))
}

/// deep edit: set one platform's region list in the whitebox
/// through the Service (single sanctioned write path). The topology canvas
/// calls this instead of assembling strategyConfig JSON client-side.
#[tauri::command]
pub async fn strategy_platform_regions_set(
    app: AppHandle,
    platform_name: String,
    regions: Vec<String>,
) -> Result<serde_json::Value, IpcError> {
    validate_short_name(&platform_name, "platform")?;
    let svc = strategy_service(&app)?;
    let stored = svc
        .set_platform_regions(&platform_name, regions)
        .map_err(IpcError::from)?;
    serde_json::to_value(&stored).map_err(|e| IpcError::from(e.to_string()))
}

/// Build the Service against the app config dir. The Service owns the only
/// strategyConfig write path in the shell (ADR-0036 discipline).
pub(crate) fn strategy_service(
    app: &AppHandle,
) -> Result<resin_core::StrategyService<resin_core::FsStrategyStore>, IpcError> {
    let dir = std::path::PathBuf::from(get_config_dir(app.clone())?);
    let path = dir.join("egressapikey-strategy.json");
    Ok(resin_core::StrategyService::new(
        resin_core::FsStrategyStore::new(path),
    ))
}

/// the authoritative effective-config
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
    forwarder: State<'_, resin_core::PortForwarder>,
    app: AppHandle,
) -> Result<resin_core::AuthoritativeSnapshot, IpcError> {
    // L2 strategy whitebox: file is the truth (ADR-0036); missing file =
    // defaults. Read goes through the Service (ADR-0052).
    let svc = strategy_service(&app)?;
    let config: resin_core::StrategyConfig = svc.get().map_err(IpcError::from)?;
    let strategy_path = svc.store_ref().path().clone();
    let strategy_path_exists = strategy_path.exists();

    // L2 ports whitebox + its SQLite sync partner. The whitebox file is the
    // truth source (ADR-0042 S2); DB rows that are absent from the whitebox
    // snapshot (boot-seed temp-store fallback path) are still surfaced so the
    // partner drift stays visible instead of silently disappearing.
    let whitebox_cfg = whitebox.snapshot();
    let db_ports = db.list_ports().map_err(IpcError::from)?;
    let wb_port_set: std::collections::HashSet<u16> =
        whitebox_cfg.entry_ports.iter().map(|m| m.port).collect();
    let mut all_ports = whitebox_cfg.entry_ports.clone();
    all_ports.extend(
        db_ports
            .into_iter()
            .filter(|m| !wb_port_set.contains(&m.port)),
    );

    // L3 Resin runtime. A sidecar that is not Running reports an empty,
    // unreachable runtime (resin_reachable=false) instead of failing the
    // snapshot: the whitebox half is still assertable while the sidecar is down.
    let reachable = sidecar.mode() == crate::sidecar::RunningMode::Running;
    let (mut resin_platforms, resin_endpoint_ports, live_subscriptions) = if reachable {
        let client = resin_client(&sidecar)?;
        let platforms_v = client
            .list_platforms()
            .await
            .map_err(|e| map_resin_error(&e.to_string()))?;
        let endpoints_v = client
            .list_endpoints()
            .await
            .map_err(|e| map_resin_error(&e.to_string()))?;
        // F4: one extra GET /api/v1/subscriptions so the
        // reverse-lookup section can join Resin stats against the whitebox
        // references. Read-only cosmetic section: a failed read degrades to
        // an empty live list (whitebox refs still surface as dangling).
        let subs_v = client.list_subscriptions().await.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "authoritative_snapshot: list_subscriptions failed; subscriptions section degrades");
            serde_json::Value::Array(vec![])
        });
        (
            resin_core::snapshot::parse_resin_platforms(&platforms_v),
            endpoint_ports(&endpoints_v),
            parse_live_subscriptions(&subs_v),
        )
    } else {
        (vec![], vec![], vec![])
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

    let mut platforms = resin_core::snapshot::merge_strategies(
        &config,
        &resin_platforms,
        &plan,
        strategy_path_exists,
    );
    // The listener set the three-state merge compares against is mode-aware
    // (ADR-0068 D1, ticket 17): Engine mode reads the Resin endpoint ports
    // (the L3 runtime truth); Mode A unions in the forwarder's bound entry
    // ports, because there the shell listener IS the realisation of the row
    // and must read Consistent. The forwarder only ever binds whitebox rows,
    // so the union can close the mode gap without inventing listeners.
    let mut listener_ports = resin_endpoint_ports;
    if forwarder.is_shell() {
        listener_ports.extend(forwarder.running_ports());
    }
    let mut ports = resin_core::snapshot::merge_ports(&all_ports, &listener_ports);

    // F4: subscription reverse lookup — union of live Resin
    // subscription rows and every whitebox reference, each with its
    // consuming platforms. Pure assembly of data already in hand; the
    // platform rows' own `subscriptions` field stays untouched.
    let whitebox_refs: Vec<(String, Vec<String>)> = config
        .platforms
        .iter()
        .filter(|ps| !ps.subscriptions.is_empty())
        .map(|ps| (ps.platform_name.clone(), ps.subscriptions.clone()))
        .collect();
    let subscriptions =
        resin_core::snapshot::merge_subscriptions(&live_subscriptions, &whitebox_refs);

    // ADR-0054 §D: read-side exemption stamps. The whitebox
    // `acknowledged` arrays NEVER enter the three-state merge above — they
    // are stamped onto the merged output only. Platform key = platform_name;
    // port key = decimal port number.
    resin_core::snapshot::stamp_platform_acknowledged(&mut platforms, &config.acknowledged);
    resin_core::snapshot::stamp_port_acknowledged(&mut ports, &whitebox_cfg.acknowledged);

    // ADR-0055 D3: route family merge — a route's live side IS
    // its target port. The live listener set and the enabled desired ports
    // are both already in hand; no extra request. Then the D6 stamp.
    let desired_enabled_ports: Vec<u16> = whitebox_cfg
        .entry_ports
        .iter()
        .filter(|m| m.enabled)
        .map(|m| m.port)
        .collect();
    // Mode-aware like the ports half above (ticket 17): a route's live side
    // is the listener on its target port, and in Mode A that listener is
    // the shell forwarder's - so the union set is the right live view.
    let mut routes = resin_core::snapshot::merge_routes(
        &whitebox_cfg.process_routes,
        &desired_enabled_ports,
        &listener_ports,
    );
    resin_core::snapshot::stamp_route_acknowledged(&mut routes, &whitebox_cfg.route_acknowledged);

    // ADR-0054 §C: divergentSince = first in-process drift
    // instant per entity. Advance the drift memory with this snapshot's
    // per-entity drift flags, then stamp the resolved instants onto the
    // drifting entries (consistent entries keep None). Process restart
    // re-instantiates an empty memory => times reset, per the legislated
    // in-memory-only semantics.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut entries: Vec<(String, bool)> = platforms
        .iter()
        .map(|p| (p.platform_name().to_string(), p.state_tag() != "consistent"))
        .collect();
    entries.extend(
        ports
            .iter()
            .map(|pp| (pp.port().to_string(), pp.state_tag() != "consistent")),
    );
    // route drift keys are prefixed "route:" so a process name
    // can never collide with a platform name or a decimal port key.
    entries.extend(routes.iter().map(|rr| {
        (
            format!("route:{}", rr.process().to_lowercase()),
            rr.state_tag() != "consistent",
        )
    }));
    let memory = {
        let mut guard = DRIFT_MEMORY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let next = resin_core::snapshot::advance_drift_memory(&guard, &entries, now);
        *guard = next;
        guard.clone()
    };
    for p in platforms.iter_mut() {
        if p.state_tag() != "consistent" {
            let since = resin_core::snapshot::divergent_since_for(&memory, p.platform_name());
            match p {
                resin_core::StrategySnapshot::Divergent {
                    divergent_since, ..
                }
                | resin_core::StrategySnapshot::MissingOnResin {
                    divergent_since, ..
                } => {
                    *divergent_since = since;
                }
                _ => {}
            }
        }
    }
    for pp in ports.iter_mut() {
        if pp.state_tag() != "consistent" {
            let since = resin_core::snapshot::divergent_since_for(&memory, &pp.port().to_string());
            match pp {
                resin_core::PortSnapshot::MissingOnResin {
                    divergent_since, ..
                } => {
                    *divergent_since = since;
                }
                _ => {}
            }
        }
    }
    for rr in routes.iter_mut() {
        if rr.state_tag() != "consistent" {
            let since = resin_core::snapshot::divergent_since_for(
                &memory,
                &format!("route:{}", rr.process().to_lowercase()),
            );
            match rr {
                resin_core::ProcessRouteSnapshot::MissingOnResin {
                    divergent_since, ..
                } => {
                    *divergent_since = since;
                }
                _ => {}
            }
        }
    }

    // ADR-0058: top-level convergence phase. Pure
    // derivation over data already in hand: the generation pair comes from
    // the whitebox strategy doc read at the top of this command, the drift
    // flag reuses the per-entry three-state stamps (acknowledged entries are
    // exempt per ADR-0054 D — known drift never masks convergence).
    let drift_platforms = platforms
        .iter()
        .any(|e| e.state_tag() != "consistent" && !e.acknowledged());
    let drift_ports = ports
        .iter()
        .any(|e| e.state_tag() != "consistent" && !e.acknowledged());
    let drift_routes = routes
        .iter()
        .any(|e| e.state_tag() != "consistent" && !e.acknowledged());
    let unacknowledged_drift = drift_platforms || drift_ports || drift_routes;
    let converge_phase = resin_core::snapshot::derive_converge_phase(
        config.generation,
        config.applied_generation,
        config.last_apply_error.as_deref(),
        reachable,
        unacknowledged_drift,
    );

    // per-subscription establish-phase STATUS rows,
    // projected straight from the strategy whitebox read at the top of this
    // command — zero new requests, read-only surface (discipline).
    let subscription_phases = config
        .subscriptions
        .iter()
        .map(resin_core::SubscriptionPhaseSnapshot::from_status)
        .collect::<Vec<_>>();

    let snapshot = resin_core::AuthoritativeSnapshot {
        strategy_version: config.version,
        platforms,
        ports,
        routes,
        subscriptions,
        subscription_phases,
        resin_reachable: reachable,
        // generation instant of THIS snapshot; monotonic
        // non-decreasing across consecutive calls (wall clock).
        last_checked_at: now,
        strategy_generation: config.generation,
        strategy_applied_generation: config.applied_generation,
        converge_phase,
        last_apply_at: config.last_apply_at,
        last_apply_error: config.last_apply_error,
    };

    // ADR-0054 §E: one-shot drift notice. Hooked on the only
    // sanctioned merge point so every snapshot consumer (TopologyView 5s
    // poll, EffectiveConfigView open/re-check/reconcile/rollback re-verify)
    // feeds the same per-process notify-once state machine — no extra
    // polling, no background loop. Best-effort: a failed toast is logged.
    crate::tray::fire_drift_notification(&app, &snapshot);

    // mirror the top-level
    // convergence phase on the tray (icon colour + tooltip tag). Same hook
    // point discipline as the drift notice above: every snapshot consumer
    // feeds one edge-driven state machine — no extra polling, no new IPC.
    // Best-effort: a failed tray repaint is logged, never fatal.
    crate::tray::apply_converge_mirror(
        &app,
        snapshot.converge_phase,
        snapshot.strategy_generation,
        snapshot.strategy_applied_generation,
    );

    // the Inline-Polling driver for the subscription
    // pipeline's backoff retries. 07 报告 §2.E3 found the due-retry schedule
    // had NO driver in production: a failed cascade re-queued its event with
    // `next_retry_at` and nothing in the repo ever drained it. This is the
    // lightest variant of the drift-detection patterns the research pass
    // surveyed (atomcode C3: "Inline Polling — reconcile 时顺手查"), so it
    // rides the existing snapshot poll (TopologyView 5s / EffectiveConfigView
    // open / manual re-check) instead of adding a second timer or a background
    // loop (ADR-0054 rejects the latter outright).
    //
    // Discipline (ADR-0054): the driver drains ONLY retries the user already
    // enqueued — it never enqueues a desired establish, so detection stays
    // resident while the ACTION stays opt-in (atomcode C2/C6). Gated on
    // `due_count > 0`, so a tick with nothing due costs one mutex read and
    // never builds a Resin client. Best-effort: a failed drain is logged and
    // swallowed — the queue keeps the event for the next tick, and a retry
    // problem must never break the snapshot read.
    if reachable {
        if let Some(pipeline_state) = app.try_state::<super::platform::SubscriptionPipelineState>()
        {
            if pipeline_state.0.due_count(now) > 0 {
                if let Ok(client) = resin_client(&sidecar) {
                    let reports = pipeline_state.0.drive_due(&client, &svc, now).await;
                    for r in &reports {
                        tracing::info!(
                            subscription = %r.subscription,
                            ok = r.all_ok(),
                            error = r.first_error().unwrap_or(""),
                            "subscription_pipeline: inline-polling driver drained a due retry"
                        );
                    }
                }
            }
        }
    }

    Ok(snapshot)
}

/// F4: parse GET /api/v1/subscriptions into (name, node_count,
/// healthy_node_count) triples for the snapshot's reverse-lookup section.
/// Accepts both the items-wrapper and bare-array shapes (mirrors
/// `subscription_snapshot` in platform.rs); malformed rows are skipped.
fn parse_live_subscriptions(v: &serde_json::Value) -> Vec<(String, u64, u64)> {
    items_arr(v)
        .iter()
        .filter_map(|s| {
            let name = s.get("name").and_then(|n| n.as_str())?;
            if name.is_empty() {
                return None;
            }
            let node_count = s.get("node_count").and_then(|n| n.as_u64()).unwrap_or(0);
            let healthy = s
                .get("healthy_node_count")
                .and_then(|n| n.as_u64())
                .unwrap_or(0);
            Some((name.to_string(), node_count, healthy))
        })
        .collect()
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

/// ADR-0054 section B: strategy whitebox versioning - list + rollback.
// ---------------------------------------------------------------------------

/// ADR-0054 section B: list the versioned backups of the strategy whitebox
/// file (newest first). Read-only; no inputs to validate.
#[tauri::command]
pub async fn strategy_backup_list(
    app: AppHandle,
) -> Result<Vec<resin_core::WhiteboxBackupEntry>, IpcError> {
    let svc = strategy_service(&app)?;
    svc.store_ref().list_backups().map_err(IpcError::from)
}

/// ADR-0054 section B: roll the strategy whitebox back to a listed backup.
/// The backup content re-enters the SAME validate + store write entry as
/// strategy_config_put (ADR-0036, no bypass), then apply PATCHes the Resin
/// region_filters so the next snapshot re-check reports the restored state.
#[tauri::command]
pub async fn strategy_rollback(
    app: AppHandle,
    sidecar: State<'_, SidecarHandle>,
    backup_name: String,
) -> Result<serde_json::Value, IpcError> {
    if backup_name.is_empty() || backup_name.len() > 200 {
        return Err(IpcError::from("backup_name invalid".to_string()));
    }
    let svc = strategy_service(&app)?;
    // ADR-0059: scope a rollback audit context so the row
    // emitted by FsStrategyStore::store carries op:"rollback" + source_backup.
    let audit_ctx = resin_core::audit::AuditCtx {
        op: Some("rollback".into()),
        actor: Some("gui:strategy_rollback".into()),
        source_backup: Some(backup_name.clone()),
        reason: None,
    };
    resin_core::audit::AUDIT_CTX
        .scope(audit_ctx, async {
            svc.store_ref()
                .rollback(&backup_name)
                .map_err(IpcError::from)
        })
        .await?;
    let client = resin_client(&sidecar)?;
    let report = svc
        .apply(&client, resolve_id_in)
        .await
        .map_err(IpcError::from)?;
    serde_json::to_value(&report).map_err(|e| IpcError::from(e.to_string()))
}

// ---------------------------------------------------------------------------
// ADR-0054 §A: one-way reconcile. Serial strategy apply ->
// ports restore, fail-fast, whitebox always wins. There is deliberately NO
// "accept current state" reverse write (§A rejection).
// ---------------------------------------------------------------------------

/// ADR-0054 §A: process-local reconcile idempotency memory. The
/// strategy half of a reconcile is naturally idempotent (re-PATCHing the
/// computed plan is a no-op); the ports half is throttled by this window so
/// a second reconcile NOW re-asserts nothing. In-process only; a restart
/// clears it (boot-time restore_ports_from_whitebox re-asserts independently).
static RECONCILE_MEMORY: once_cell::sync::Lazy<resin_core::ReconcileMemory> =
    once_cell::sync::Lazy::new(resin_core::ReconcileMemory::default);

/// One-way reconcile (ADR-0054 §A): run the strategy apply then re-assert
/// the whitebox ports, stopping at the first failure. Errors surface through
/// the ADR-0045 IpcError contract; the frontend re-pulls the snapshot after
/// either outcome. The ports half goes through the SAME
/// `restore_ports_from_whitebox` loop the boot path uses (ADR-0042 S6 seam),
/// minus its log-only error swallowing: a reconcile must FAIL loudly, not
/// degrade silently, so the closure re-implements the per-port POST with the
/// shared helpers and returns Err on the first non-409 failure.
#[tauri::command]
pub async fn reconcile_now(
    sidecar: State<'_, SidecarHandle>,
    whitebox: State<'_, resin_core::WhiteboxConfigStore>,
    forwarder: State<'_, resin_core::PortForwarder>,
    app: AppHandle,
) -> Result<serde_json::Value, IpcError> {
    let svc = strategy_service(&app)?;
    let client = resin_client(&sidecar)?;
    let sidecar_ref = sidecar.inner();
    let whitebox_ref = whitebox.inner();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let fwd_ref = forwarder.inner();
    let report = svc
        .reconcile(&client, resolve_id_in, async {
            reconcile_ports_half(sidecar_ref, whitebox_ref, fwd_ref, now).await
        })
        .await
        .map_err(IpcError::from)?;
    Ok(serde_json::json!({
        "strategy": report.strategy,
        "portsRestored": report.ports_restored,
        "portsSkipped": report.ports_skipped,
    }))
}

/// The ports half of one reconcile pass. Mirrors the per-port body of
/// `restore_ports_from_whitebox` (commands/common.rs) but fails loudly:
/// the reconcile contract is fail-fast, not best-effort. 409
/// (endpoint already present) counts as satisfied, not an error. Shared with
/// `config_import` so an import triggers the SAME one-way
/// reconcile as `reconcile_now` (ADR-0054 §A) rather than a parallel path.
pub(crate) async fn reconcile_ports_half(
    sidecar: &SidecarHandle,
    whitebox: &resin_core::WhiteboxConfigStore,
    forwarder: &resin_core::PortForwarder,
    now: u64,
) -> Result<resin_core::ReconcilePortsOutcome, String> {
    let cfg = whitebox.snapshot();
    if forwarder.is_shell() {
        // Mode A (ticket 17): the assertion target is the shell's own accept
        // loops, not Resin endpoints. reload() is level-triggered (enabled
        // rows already serving are skipped, missing ones spawn, stale ones
        // stop), so the bound set is the liveness filter and the 409
        // anti-hammer stamp below is Engine-path only.
        let before: std::collections::HashSet<u16> =
            forwarder.running_ports().into_iter().collect();
        forwarder.reload(&cfg.entry_ports).await?;
        let mut outcome = resin_core::ReconcilePortsOutcome::default();
        for p in forwarder.running_ports() {
            if before.contains(&p) {
                outcome.skipped += 1;
            } else {
                outcome.restored.push(p);
                tracing::info!(
                    port = p,
                    "reconcile: mode A entry listener asserted from whitebox"
                );
            }
        }
        return Ok(outcome);
    }
    let client = resin_client(sidecar)?;
    let existing = client
        .list_endpoints()
        .await
        .map_err(|e| format!("list_endpoints: {e:?}"))?;
    let live_ports = resin_core::endpoint_live_ports(&existing);
    let to_assert = RECONCILE_MEMORY.ports_to_assert(&cfg.entry_ports, &live_ports, now);
    let mut outcome = resin_core::ReconcilePortsOutcome::default();
    for m in to_assert {
        let proto = m.protocol.trim().to_ascii_lowercase();
        // ONE shared derivation, not a fourth
        // hand-rolled copy. mixed opens both capabilities, http only HTTP
        // forwarding, socks5 only SOCKS5.
        let (allow_socks5, allow_http_forward) = resin_core::entry_protocol::engine_flags(&proto);
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
                // ADR-0069 D2: a SUCCESSFUL assertion is deliberately NOT
                // stamped. Idempotency on the success path comes from the
                // liveness filter (the endpoint is live now, so the next pass
                // skips it); leaving it unstamped is what lets an
                // externally-DELETED endpoint be restored on the NEXT pass
                // instead of after the 24h TTL window. Only the 409 conflict
                // path stamps - that is the anti-hammer case.
                outcome.restored.push(m.port);
                tracing::info!(
                    port = m.port,
                    "reconcile: re-asserted Resin endpoint from whitebox"
                );
            }
            Err(e) => {
                let msg = format!("{e:?}");
                if msg.contains("409") || msg.contains("CONFLICT") || msg.contains("Only one usage")
                {
                    // ADR-0069 D2: this is the ONLY stamp site. 409 means the
                    // port is held by something Resin refuses to re-create
                    // (typically a listener Resin does not own), so the TTL
                    // window exists to stop the pass from hammering it.
                    RECONCILE_MEMORY.stamp_asserted(m.port, now);
                    outcome.skipped += 1;
                    tracing::info!(
                        port = m.port,
                        "reconcile: endpoint already in Resin; satisfied"
                    );
                } else {
                    return Err(format!("create_endpoint {}: {msg}", m.port));
                }
            }
        }
    }
    Ok(outcome)
}
