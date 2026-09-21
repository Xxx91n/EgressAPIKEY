//! Orchestration controller shell seam (round 11 R11-06, wave-C D-003).
//!
//! The state machine itself is pure and lives in
//! `resin_core::orchestration`; this module owns the IO: per-tick signal
//! collection (end-to-end `port_health_check` probes on a platform's bound
//! entry ports), candidate-region metrics (node-pool aggregates, no new
//! dataplane behavior), persistence through `StrategyService` (spec writes
//! via `set_platform_regions` + `apply`; bookkeeping via
//! `orchestration_mutate`), and the suggest-tier approval gate.
//!
//! Sample rings are process-local and in-memory only: a restart clears the
//! evidence window — conservative by construction, since a stale verdict
//! can never trigger a switch on boot.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, State};

use super::common::{resin_client, validate_short_name};
use super::strategy::strategy_service;
use crate::sidecar::SidecarHandle;
use resin_core::orchestration as orch;
use resin_core::resolve_id_in;
use resin_core::DbPool;
use resin_core::IpcError;

/// Tick cadence of the in-shell driver loop (both transports).
const TICK_INTERVAL_SECS: u64 = 60;
/// Max retained per-platform tick verdicts (sliding window bound).
const RING_CAP: usize = 64;

/// Process-local verdict rings: platform_name -> recent tick verdicts
/// (true = failed). In-memory by design; see module docs.
static SIGNALS: OnceLock<Mutex<HashMap<String, VecDeque<bool>>>> = OnceLock::new();

fn rings() -> &'static Mutex<HashMap<String, VecDeque<bool>>> {
    SIGNALS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolve the effective autonomy tier: explicit config wins; otherwise the
/// transport default (desktop = suggest, headless = auto — D-003).
pub fn effective_autonomy(
    sec: &orch::OrchestrationSection,
    transport_default: orch::Autonomy,
) -> orch::Autonomy {
    sec.params.autonomy.unwrap_or(transport_default)
}

/// Aggregate one platform's bound-port probes into this tick's verdict and
/// push it into the sliding window. Returns the WindowStats for the
/// evaluator, or None when the platform has no enabled bound ports (no
/// signal source -> no evidence -> no action).
async fn collect_verdict(
    db: &DbPool,
    platform_name: &str,
    slow_call_ms: u64,
) -> Option<orch::WindowStats> {
    let rows = db.list_ports().ok()?;
    let ports: Vec<u16> = rows
        .iter()
        .filter(|p| p.platform_name == platform_name && p.enabled)
        .map(|p| p.port)
        .collect();
    if ports.is_empty() {
        return None;
    }
    let total = ports.len() as u32;
    let mut bad = 0u32;
    for port in ports {
        // End-to-end entry-port probe: connect + dialect greeting against
        // the shell's own listener — no Resin dataplane impact.
        let h = super::ports::port_health_check(port, None)
            .await
            .unwrap_or_else(|_| super::ports::PortHealthCheck {
                port,
                reachable: false,
                socks5_ok: false,
                protocol_mismatch: false,
                latency_ms: slow_call_ms + 1,
                reason: "error".into(),
            });
        if !h.reachable || h.protocol_mismatch || h.latency_ms > slow_call_ms {
            bad += 1;
        }
    }
    // Majority rule: the tick fails when MORE THAN HALF the platform's
    // bound ports are unreachable / mismatched / slow — one bad port must
    // not trip a whole platform.
    let tick_failed = bad * 2 > total.max(1);
    let mut guard = rings().lock().ok()?;
    let ring = guard
        .entry(platform_name.to_string())
        .or_insert_with(VecDeque::new);
    ring.push_back(tick_failed);
    while ring.len() > RING_CAP {
        ring.pop_front();
    }
    let fails = ring.iter().filter(|&&f| f).count() as u32;
    let consecutive = ring.iter().rev().take_while(|&&f| f).count() as u32;
    Some(orch::WindowStats {
        samples: ring.len() as u32,
        fails,
        consecutive_fails: consecutive,
        tick_failed,
    })
}

/// Aggregate the node pool into per-region ok/err shares (candidate
/// metrics): ok = has_outbound && failure_count==0.
fn region_metrics(
    nodes: &[resin_core::strategy_engine::NodeSummary],
) -> HashMap<String, orch::RegionMetric> {
    let mut acc: HashMap<String, (u32, u32, u32)> = HashMap::new();
    for n in nodes {
        if n.region.is_empty() {
            continue;
        }
        let e = acc.entry(n.region.clone()).or_insert((0, 0, 0));
        e.0 += 1; // nodes
        if n.has_outbound && n.failure_count == 0 {
            e.1 += 1; // ok
        } else {
            e.2 += 1; // err
        }
    }
    acc.into_iter()
        .map(|(r, (nodes, ok, err))| {
            (
                r,
                orch::RegionMetric {
                    ok_share: ok as f64 / nodes as f64,
                    err_share: err as f64 / nodes as f64,
                    node_count: nodes,
                },
            )
        })
        .collect()
}

/// One controller tick: collect signals, evaluate the section, execute the
/// emitted actions per the resolved autonomy tier, persist bookkeeping.
/// Shared by the `orchestration_tick` IPC, the headless route, and the
/// 60s driver loop.
pub async fn orchestration_tick_impl(
    svc: &resin_core::StrategyService<resin_core::FsStrategyStore>,
    client: &resin_core::ResinClient,
    db: &DbPool,
    transport_default: orch::Autonomy,
) -> Result<serde_json::Value, IpcError> {
    let config = svc.get().map_err(IpcError::from)?;
    let Some(mut sec) = config.orchestration.clone() else {
        return Ok(serde_json::json!({ "enabled": false, "actions": [] }));
    };
    if !sec.params.enabled {
        return Ok(serde_json::json!({ "enabled": false, "actions": [] }));
    }
    let params = sec.params.clone();
    let autonomy = effective_autonomy(&sec, transport_default);
    let now = resin_core::whitebox_backup::now_unix();

    // Reconcile the managed row set: one row per desired region-class
    // platform; orphan rows (platform removed from the whitebox) drop out.
    let desired: Vec<&resin_core::strategy_engine::PlatformStrategy> = config
        .platforms
        .iter()
        .filter(|p| {
            matches!(
                p.a_class,
                resin_core::strategy_engine::AClassStrategy::Region
            )
        })
        .collect();
    let desired_names: std::collections::HashSet<&str> =
        desired.iter().map(|p| p.platform_name.as_str()).collect();
    sec.platforms
        .retain(|r| desired_names.contains(r.platform_name.as_str()));
    for p in &desired {
        if !sec
            .platforms
            .iter()
            .any(|r| r.platform_name == p.platform_name)
        {
            sec.platforms
                .push(orch::PlatformOrch::new(&p.platform_name));
        }
    }

    // Signals: one verdict per managed platform (end-to-end port probes).
    let mut verdicts = HashMap::new();
    for p in &desired {
        if let Some(stats) = collect_verdict(db, &p.platform_name, params.slow_call_ms).await {
            verdicts.insert(p.platform_name.clone(), stats);
        }
    }

    // Candidate metrics only when something could act on them (a Degraded
    // row without a parked proposal) — zero extra Resin calls otherwise.
    let need_metrics = sec
        .platforms
        .iter()
        .any(|r| r.phase == orch::OrchPhase::Degraded && r.pending.is_none());
    let mut region_metrics_map: HashMap<String, HashMap<String, orch::RegionMetric>> =
        HashMap::new();
    if need_metrics {
        let nodes_v = client
            .list_nodes()
            .await
            .map_err(|e| IpcError::from(format!("orchestration: list_nodes failed: {e}")))?;
        let nodes = resin_core::parse_nodes(&nodes_v);
        let m = region_metrics(&nodes);
        for p in &desired {
            region_metrics_map.insert(p.platform_name.clone(), m.clone());
        }
    }

    let current_regions: HashMap<String, Vec<String>> = desired
        .iter()
        .map(|p| (p.platform_name.clone(), p.regions.clone()))
        .collect();

    let actions = orch::evaluate_section(
        &mut sec,
        &verdicts,
        &region_metrics_map,
        &current_regions,
        autonomy,
        now,
    );

    // Persist bookkeeping once per tick (status-subresource write: no
    // generation bump, same store entry / backup ring / audit row).
    let sec_snapshot = sec.clone();
    svc.orchestration_mutate(|s| *s = sec_snapshot)
        .map_err(IpcError::from)?;

    // Execute actions: desired-state changes re-enter the ONE authoritative
    // write entry (set_platform_regions -> apply), generation + audit
    // included. A failed apply is reported, never hidden — the observation
    // window will regress the platform into Cooldown on its own.
    let mut executed: Vec<serde_json::Value> = Vec::new();
    for a in actions {
        match a {
            orch::OrchAction::Switch {
                platform_name,
                regions,
                reason,
                ..
            }
            | orch::OrchAction::Restore {
                platform_name,
                regions,
                reason,
            } => {
                let write = svc
                    .set_platform_regions(&platform_name, regions.clone())
                    .map_err(IpcError::from);
                let applied = match write {
                    Ok(_) => svc.apply(client, resolve_id_in).await.ok(),
                    Err(e) => {
                        executed.push(serde_json::json!({
                            "platform": platform_name, "action": "switch",
                            "regions": regions, "ok": false, "error": e.to_string(),
                        }));
                        continue;
                    }
                };
                executed.push(serde_json::json!({
                    "platform": platform_name, "action": "switch",
                    "regions": regions, "reason": reason, "ok": applied.is_some(),
                }));
            }
            orch::OrchAction::Bookkeep { platform_name } => {
                executed.push(serde_json::json!({
                    "platform": platform_name, "action": "bookkeep", "ok": true,
                }));
            }
            orch::OrchAction::Blocked {
                platform_name,
                reason,
            } => {
                executed.push(serde_json::json!({
                    "platform": platform_name, "action": "blocked",
                    "reason": reason, "ok": true,
                }));
            }
        }
    }

    Ok(serde_json::json!({
        "enabled": true,
        "autonomy": format!("{autonomy:?}").to_lowercase(),
        "actions": executed,
    }))
}

// ── IPC surface ────────────────────────────────────────────────────────────

/// Shared read projection (desktop command + headless route).
pub fn orchestration_get_impl(
    svc: &resin_core::StrategyService<resin_core::FsStrategyStore>,
    transport_default: orch::Autonomy,
) -> Result<serde_json::Value, IpcError> {
    let config = svc.get().map_err(IpcError::from)?;
    let autonomy = config
        .orchestration
        .as_ref()
        .map(|s| effective_autonomy(s, transport_default))
        .unwrap_or(transport_default);
    Ok(serde_json::json!({
        "orchestration": config.orchestration,
        "autonomy": format!("{autonomy:?}").to_lowercase(),
    }))
}

/// Read the orchestration section plus the resolved autonomy tier
/// (transport default applied when `params.autonomy` is unset).
#[tauri::command]
pub async fn orchestration_get(app: AppHandle) -> Result<serde_json::Value, IpcError> {
    let svc = strategy_service(&app)?;
    orchestration_get_impl(&svc, orch::Autonomy::Suggest)
}

/// Replace the orchestration parameter pack (validated through the same
/// store entry; bookkeeping-class write, no generation bump — the pack
/// never maps to Resin desired state).
pub fn orchestration_config_put_impl(
    svc: &resin_core::StrategyService<resin_core::FsStrategyStore>,
    params: serde_json::Value,
) -> Result<(), IpcError> {
    let typed: orch::OrchestrationParams = serde_json::from_value(params)
        .map_err(|e| IpcError::from(format!("orchestration params invalid: {e}")))?;
    svc.orchestration_mutate(|s| s.params = typed)
        .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command]
pub async fn orchestration_config_put(
    app: AppHandle,
    params: serde_json::Value,
) -> Result<(), IpcError> {
    let svc = strategy_service(&app)?;
    orchestration_config_put_impl(&svc, params)
}

/// Manual tick (the 60s driver loop calls the same impl).
#[tauri::command]
pub async fn orchestration_tick(
    app: AppHandle,
    sidecar: State<'_, SidecarHandle>,
    db: State<'_, DbPool>,
) -> Result<serde_json::Value, IpcError> {
    let svc = strategy_service(&app)?;
    let client = resin_client(&sidecar).map_err(IpcError::from)?;
    orchestration_tick_impl(&svc, &client, &db, orch::Autonomy::Suggest).await
}

/// Suggest-tier gate: execute a parked proposal through the authoritative
/// write entry. A rollback proposal (`is_rollback`) returns the platform to
/// Healthy directly; a forward proposal enters the Observing window with
/// the pre-approval regions recorded as baseline.
pub async fn orchestration_approve_impl(
    svc: &resin_core::StrategyService<resin_core::FsStrategyStore>,
    client: &resin_core::ResinClient,
    platform_name: &str,
) -> Result<(), IpcError> {
    validate_short_name(platform_name, "platform")?;
    let config = svc.get().map_err(IpcError::from)?;
    let sec = config
        .orchestration
        .as_ref()
        .ok_or_else(|| IpcError::invalid_input("orchestration section absent"))?;
    let row = sec
        .platforms
        .iter()
        .find(|r| r.platform_name == platform_name)
        .ok_or_else(|| IpcError::invalid_input("no orchestration row for platform"))?;
    let proposal = row
        .pending
        .clone()
        .ok_or_else(|| IpcError::invalid_input("no pending proposal for platform"))?;
    // Pre-approval regions become the observation baseline for a forward
    // switch (the regression rollback target).
    let prior_regions = config
        .platforms
        .iter()
        .find(|p| p.platform_name == platform_name)
        .map(|p| p.regions.clone())
        .unwrap_or_default();

    svc.set_platform_regions(platform_name, proposal.regions.clone())
        .map_err(IpcError::from)?;
    let _ = svc.apply(client, resolve_id_in).await;

    let now = resin_core::whitebox_backup::now_unix();
    let is_rollback = proposal.is_rollback;
    svc.orchestration_mutate(|s| {
        if let Some(r) = s
            .platforms
            .iter_mut()
            .find(|r| r.platform_name == platform_name)
        {
            r.pending = None;
            if is_rollback {
                r.phase = orch::OrchPhase::Healthy;
                r.baseline = None;
                r.good_cycles = 0;
            } else {
                r.phase = orch::OrchPhase::Observing;
                r.good_cycles = 0;
                r.last_switch_at = now;
                r.switch_log.push(now);
                r.baseline = Some(orch::OrchBaseline {
                    regions: prior_regions,
                });
            }
        }
    })
    .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command]
pub async fn orchestration_approve(
    app: AppHandle,
    sidecar: State<'_, SidecarHandle>,
    platform_name: String,
) -> Result<(), IpcError> {
    validate_short_name(&platform_name, "platform")?;
    let svc = strategy_service(&app)?;
    let client = resin_client(&sidecar).map_err(IpcError::from)?;
    orchestration_approve_impl(&svc, &client, &platform_name).await
}

/// Suggest-tier gate: drop a parked proposal and park the platform in
/// cooldown so the machine does not re-propose it on the next tick.
pub fn orchestration_dismiss_impl(
    svc: &resin_core::StrategyService<resin_core::FsStrategyStore>,
    platform_name: &str,
) -> Result<(), IpcError> {
    validate_short_name(platform_name, "platform")?;
    let now = resin_core::whitebox_backup::now_unix();
    svc.orchestration_mutate(|s| {
        if let Some(r) = s
            .platforms
            .iter_mut()
            .find(|r| r.platform_name == platform_name)
        {
            r.pending = None;
            orch::cool_down(r, &s.params, now);
        }
    })
    .map_err(IpcError::from)?;
    Ok(())
}

#[tauri::command]
pub async fn orchestration_dismiss(app: AppHandle, platform_name: String) -> Result<(), IpcError> {
    let svc = strategy_service(&app)?;
    orchestration_dismiss_impl(&svc, &platform_name)
}

// ── driver loop ─────────────────────────────────────────────────────────────

/// Desktop driver: one tick per minute while the app runs. The tick itself
/// is fully gated by `params.enabled` — the loop is a cheap no-op when the
/// controller is off.
pub fn spawn_orchestration_driver(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(TICK_INTERVAL_SECS)).await;
            let Ok(svc) = strategy_service(&app) else {
                continue;
            };
            let (Some(sidecar), Some(db)) =
                (app.try_state::<SidecarHandle>(), app.try_state::<DbPool>())
            else {
                continue;
            };
            let Ok(client) = resin_client(&sidecar) else {
                continue;
            };
            if let Err(e) =
                orchestration_tick_impl(&svc, &client, &db, orch::Autonomy::Suggest).await
            {
                tracing::warn!(error = %e, "orchestration tick failed");
            }
        }
    });
}
