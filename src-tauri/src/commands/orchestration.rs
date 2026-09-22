//! Orchestration controller shell seam.
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
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Manager, State};

use super::common::{resin_client, validate_short_name};
use super::strategy::strategy_service;
use crate::sidecar::SidecarHandle;
use resin_core::db::PortMapping;
use resin_core::orchestration as orch;
use resin_core::resolve_id_in;
use resin_core::DbPool;
use resin_core::IpcError;

/// Tick cadence of the in-shell driver loop (both transports).
const TICK_INTERVAL_SECS: u64 = 60;
/// Max retained per-platform tick verdicts (sliding window bound).
const RING_CAP: usize = 64;

/// Process-local verdict rings: platform_name -> recent typed tick
/// verdicts (ADR-0080). In-memory by design; see module docs.
static SIGNALS: OnceLock<Mutex<HashMap<String, VecDeque<orch::ProbeVerdict>>>> =
    OnceLock::new();

fn rings() -> &'static Mutex<HashMap<String, VecDeque<orch::ProbeVerdict>>> {
    SIGNALS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Tick-level environment-suspect streak (ADR-0080 §4): consecutive
/// common-mode-suspect ticks, reset by any clean tick. Deliberately
/// independent of the per-platform rings.
static SUSPECT_STREAK: AtomicU32 = AtomicU32::new(0);

/// Suspect ticks in a row that emit an environment audit row — hardwired,
/// not a parameter (anti-Goodhart).
const SUSPECT_STREAK_AUDIT: u32 = 3;

/// Per-node engine `failure_count` rings: node_hash -> recent samples (one
/// per region-metrics pass). The engine counter is cumulative, so the
/// ok/err caliber windows it on the caller side: a node is err only
/// when the counter GREW inside the window.
static NODE_FC: OnceLock<Mutex<HashMap<String, VecDeque<i64>>>> = OnceLock::new();

fn fc_rings() -> &'static Mutex<HashMap<String, VecDeque<i64>>> {
    NODE_FC.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolve the effective autonomy tier: explicit config wins; otherwise the
/// transport default (desktop = suggest, headless = auto).
pub fn effective_autonomy(
    sec: &orch::OrchestrationSection,
    transport_default: orch::Autonomy,
) -> orch::Autonomy {
    sec.params.autonomy.unwrap_or(transport_default)
}

/// Per-port joined probe outcomes for one platform in one tick.
struct PortProbeBatch {
    verdicts: Vec<orch::ProbeVerdict>,
    /// ≤64B operator-facing evidence per non-ok port (IPC/audit only).
    details: Vec<String>,
    local_fails: u32,
}

/// One tick's signal plane (ADR-0080): the evaluator-facing window stats,
/// the typed per-platform verdicts, and the environment streak.
struct TickSignals {
    verdicts: HashMap<String, orch::WindowStats>,
    signal_map: HashMap<String, orch::ProbeVerdict>,
    platform_verdicts: Vec<serde_json::Value>,
    verdict_details: Vec<serde_json::Value>,
    suspect_streak: u32,
}

/// Collect SLI probe verdicts for EVERY platform owning >=1 enabled bound
/// port, independent of the orchestration section: a disabled or absent
/// controller must not blind the signal plane (the acceptance line reads
/// these rings). One port-list read, then one probe per bound port —
/// the pass is bounded by bound-port count. Rings for platforms that no
/// longer own an enabled bound port are evicted here (rename/remove would
/// otherwise leak them in-process forever).
///
/// The common-mode suppressor runs at TICK level (ADR-0080 §3): when
/// >=80% of all probed bound ports fail loopback, the tick is stamped
/// environment_suspect for every probed platform — a local outage is one
/// environmental event, never N platform failures.
async fn collect_all_verdicts(
    db: &DbPool,
    slow_call_ms: u64,
    proxy_token: &str,
) -> TickSignals {
    let rows = db.list_ports().unwrap_or_default();
    let mut by_platform: HashMap<String, Vec<PortMapping>> = HashMap::new();
    for p in rows.into_iter().filter(|p| p.enabled) {
        by_platform
            .entry(p.platform_name.clone())
            .or_default()
            .push(p);
    }

    let mut batches: HashMap<String, PortProbeBatch> = HashMap::new();
    let (mut probed, mut local_fails) = (0u32, 0u32);
    for (name, ports) in &by_platform {
        if let Some(b) = probe_platform_ports(ports, slow_call_ms, proxy_token, db).await {
            probed += b.verdicts.len() as u32;
            local_fails += b.local_fails;
            batches.insert(name.clone(), b);
        }
    }

    let suspect = orch::common_mode_suspect(local_fails, probed);
    let suspect_streak = if suspect {
        SUSPECT_STREAK.fetch_add(1, Ordering::Relaxed) + 1
    } else {
        SUSPECT_STREAK.store(0, Ordering::Relaxed);
        0
    };
    if suspect_streak == SUSPECT_STREAK_AUDIT {
        // One audit row per crossing (tick-dimension environment event).
        let mut ev = resin_core::audit::event(
            "signal-plane",
            "environment_suspect",
            "orchestration:tick",
            String::new(),
            String::new(),
            "suspect",
            None,
            None,
        );
        ev.reason = Some(orch::bounded_detail(&format!(
            "streak={SUSPECT_STREAK_AUDIT} local_fail={local_fails}/{probed}"
        )));
        let _ = resin_core::audit::append(&ev);
    }

    let mut sig = TickSignals {
        verdicts: HashMap::new(),
        signal_map: HashMap::new(),
        platform_verdicts: Vec::new(),
        verdict_details: Vec::new(),
        suspect_streak,
    };
    let mut names: Vec<&String> = batches.keys().collect();
    names.sort();
    let mut guard = match rings().lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    for name in names {
        let batch = &batches[name];
        let verdict = if suspect {
            orch::ProbeVerdict::EnvironmentSuspect
        } else {
            orch::aggregate_platform_verdict(&batch.verdicts)
        };
        let ring = guard.entry(name.clone()).or_insert_with(VecDeque::new);
        ring.push_back(verdict);
        while ring.len() > RING_CAP {
            ring.pop_front();
        }
        sig.verdicts
            .insert(name.clone(), orch::window_stats(ring, verdict));
        sig.signal_map.insert(name.clone(), verdict);
        sig.platform_verdicts.push(serde_json::json!({
            "platform": name,
            "verdict": verdict,
            "ok_share": orch::ok_share(ring),
            "streak": orch::verdict_streak(ring, verdict),
        }));
        if !batch.details.is_empty() {
            sig.verdict_details.push(serde_json::json!({
                "platform": name,
                "detail": orch::bounded_detail(&batch.details.join("; ")),
            }));
        }
    }
    guard.retain(|k, _| by_platform.contains_key(k));
    sig
}

/// Probe one platform's bound ports and return the per-port joined
/// verdicts (ADR-0080 §1 join matrix). Returns None when the platform has
/// no enabled bound ports (no signal source -> no evidence -> no action).
///
/// Per port the verdict joins TWO independent SLI probes:
///   loopback `port_health_check` - the shell listener is up and dialect-
///   coherent (cheap, no dataplane);
///   `probe_exit_ip` through the port - the real end-to-end egress path
///   (port -> engine -> node -> Cloudflare trace). A port whose listener is
///   healthy but whose egress is dead is a real failure the loopback probe
///   alone could not see.
/// The probe count is bounded by the platform's bound-port count (one per
/// port, concurrently) and neither probe consults or mutates any engine-side
/// orchestration switch - pure measurement.
async fn probe_platform_ports(
    ports: &[PortMapping],
    slow_call_ms: u64,
    proxy_token: &str,
    db: &DbPool,
) -> Option<PortProbeBatch> {
    if ports.is_empty() {
        return None;
    }
    let mut set = tokio::task::JoinSet::new();
    for p in ports.iter().cloned() {
        let db2 = db.clone();
        let token = proxy_token.to_string();
        set.spawn(async move {
            let port = p.port;
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
            // `mixed` listeners probe via the SOCKS5 dialect (the same rule
            // probe_exit_ip applies); declared http/socks5 ports use theirs.
            let proto = p.protocol.to_ascii_lowercase();
            let egress = super::diagnostics::probe_exit_ip_impl(&token, &db2, port, proto).await;
            (port, h, egress)
        });
    }
    let mut batch = PortProbeBatch {
        verdicts: Vec::new(),
        details: Vec::new(),
        local_fails: 0,
    };
    while let Some(res) = set.join_next().await {
        let Ok((port, h, egress)) = res else {
            batch.verdicts.push(orch::ProbeVerdict::Skipped);
            continue;
        };
        let loopback_ok =
            h.reachable && !h.protocol_mismatch && h.latency_ms <= slow_call_ms;
        let (egress_ok, egress_note) = match &egress {
            Ok(ep) => (
                ep.status == 200 && !ep.exit_ip.is_empty() && ep.latency_ms <= slow_call_ms,
                format!("eg st={} lat={}ms", ep.status, ep.latency_ms),
            ),
            Err(_) => (false, "eg err".to_string()),
        };
        let v = orch::join_probe(loopback_ok, egress_ok);
        if v == orch::ProbeVerdict::LocalFail {
            batch.local_fails += 1;
        }
        if v != orch::ProbeVerdict::Ok {
            // <=64B operator evidence (ADR-0080 §1): the egress latency
            // reading is what tells a half-dead WAN apart from a dead
            // platform on the remote_fail rows.
            let note = if !loopback_ok && egress_ok {
                format!("p{port} contradiction lb_fail+eg_ok")
            } else if !loopback_ok {
                format!("p{port} lb_fail:{} {}", h.reason, egress_note)
            } else {
                format!("p{port} {egress_note}")
            };
            batch.details.push(orch::bounded_detail(&note));
        }
        batch.verdicts.push(v);
    }
    Some(batch)
}

/// Aggregate the node pool into per-region ok/err shares (candidate
/// metrics). ok = has_outbound && the engine failure_count did not grow
/// inside the caller-side window (NODE_FC ring; the engine counter is
/// cumulative, so failure_count==0 would permanently disqualify recovered
/// nodes). `window` is the ring cap in samples.
fn region_metrics(
    nodes: &[resin_core::strategy_engine::NodeSummary],
    window: usize,
) -> HashMap<String, orch::RegionMetric> {
    let mut acc: HashMap<String, (u32, u32, u32)> = HashMap::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut rings = match fc_rings().lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    for n in nodes {
        if n.region.is_empty() {
            continue;
        }
        seen.insert(n.node_hash.clone());
        let delta = orch::failure_count_window_delta(
            rings.entry(n.node_hash.clone()).or_default(),
            n.failure_count,
            window.max(1),
        );
        let e = acc.entry(n.region.clone()).or_insert((0, 0, 0));
        e.0 += 1; // nodes
        if n.has_outbound && delta <= 0 {
            e.1 += 1; // ok
        } else {
            e.2 += 1; // err
        }
    }
    // Bound the ring map to nodes still present (a vanished node's ring is
    // stale evidence and would leak memory otherwise).
    rings.retain(|k, _| seen.contains(k));
    drop(rings);
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
    proxy_token: &str,
) -> Result<serde_json::Value, IpcError> {
    let config = svc.get().map_err(IpcError::from)?;

    // Signal collection runs BEFORE the orchestration gates: the SLI probe
    // pass is an independent bounded path (one probe per enabled bound
    // port), not a parasite of the enabled switch — disabling orchestration
    // parks the controller, it must not blind the signal plane.
    let probe_slow_ms = config
        .orchestration
        .as_ref()
        .map(|s| s.params.slow_call_ms)
        .unwrap_or_else(|| orch::OrchestrationParams::default().slow_call_ms);
    let sig = collect_all_verdicts(db, probe_slow_ms, proxy_token).await;
    let verdicts = &sig.verdicts;

    let Some(mut sec) = config.orchestration.clone() else {
        // Absent section: never created just to hold signals (zero
        // migration); the tick still reports the live verdicts.
        return Ok(serde_json::json!({
            "enabled": false, "actions": [],
            "signals": sig.signal_map.len(),
            "platform_verdicts": sig.platform_verdicts,
            "verdict_details": sig.verdict_details,
            "environment_status": { "suspect_streak": sig.suspect_streak },
        }));
    };
    if !sec.params.enabled {
        // Disabled-but-present section: persist the signal projection so
        // the UI reflects evidence even with the controller parked.
        let map = sig.signal_map.clone();
        let streak = sig.suspect_streak;
        svc.orchestration_mutate(|s| {
            s.signal_verdicts = map;
            s.suspect_streak = streak;
        })
        .map_err(IpcError::from)?;
        return Ok(serde_json::json!({
            "enabled": false, "actions": [],
            "signals": sig.signal_map.len(),
            "platform_verdicts": sig.platform_verdicts,
            "verdict_details": sig.verdict_details,
            "environment_status": { "suspect_streak": sig.suspect_streak },
        }));
    };
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
        let m = region_metrics(&nodes, params.window_min_samples as usize);
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
        verdicts,
        &region_metrics_map,
        &current_regions,
        autonomy,
        now,
    );

    // Signal-plane bookkeeping rides the same per-tick status write
    // (ADR-0080 §7): latest verdicts + suspect streak, no generation bump.
    sec.signal_verdicts = sig.signal_map.clone();
    sec.suspect_streak = sig.suspect_streak;
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
        "signals": sig.signal_map.len(),
        "platform_verdicts": sig.platform_verdicts,
        "verdict_details": sig.verdict_details,
        "environment_status": { "suspect_streak": sig.suspect_streak },
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
    orchestration_tick_impl(
        &svc,
        &client,
        &db,
        orch::Autonomy::Suggest,
        &sidecar.proxy_token,
    )
    .await
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
    // Propagate apply failure: the whitebox write landed but the engine was
    // not applied — surface the error so the proposal stays parked (the
    // caller can retry approve) instead of silently advancing the phase
    // with a divergent live state.
    svc.apply(client, resolve_id_in)
        .await
        .map_err(IpcError::from)?;

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
            if let Err(e) = orchestration_tick_impl(
                &svc,
                &client,
                &db,
                orch::Autonomy::Suggest,
                &sidecar.proxy_token,
            )
            .await
            {
                tracing::warn!(error = %e, "orchestration tick failed");
            }
        }
    });
}
