//! IPC commands exposed to the React frontend via `tauri::generate_handler!`.
//!
//! Each command locks the shared GatewayState for one short critical section
//! and never awaits across the lock, so the per-lane Mutex lease invariants
//! in resin-core stay intact.

use serde::Serialize;
use tauri::State;

use crate::SharedGateway;

/// Outcome of one attempt to acquire a lane+lease for a request.
#[derive(Debug, Serialize)]
pub struct ReserveResult {
    pub lane: usize,
    pub lease: Option<u64>,
    pub reason: String,
}

/// Reserve a lane+lease for (api_key, account, authority) + a known exit IP.
/// When the lane is busy the frontend picks another account / waits.
#[tauri::command]
pub fn gateway_reserve(
    state: State<SharedGateway>,
    api_key: String,
    account: String,
    authority: String,
    exit_ip: Option<String>,
) -> Result<ReserveResult, String> {
    let g = state.lock();
    let r = g.reserve(&api_key, &account, &authority, exit_ip.as_deref());
    Ok(ReserveResult {
        lane: r.lane,
        lease: r.lease.map(|l| l.raw()),
        reason: format!("{:?}", r.reason),
    })
}

/// Release a previously acquired lease (stream ended / errored). No-op on None.
#[tauri::command]
pub fn gateway_release(state: State<SharedGateway>, lease: Option<u64>) -> Result<(), String> {
    let g = state.lock();
    g.release(lease.map(resin_core::LeaseId));
    Ok(())
}

/// Force-evict everything on a lane (failure path).
#[tauri::command]
pub fn gateway_evict_lane(state: State<SharedGateway>, lane: usize) -> Result<(), String> {
    let g = state.lock();
    g.evict_lane(lane);
    Ok(())
}

/// Record a latency sample for an authority (drives TD-EWMA trend stats).
#[tauri::command]
pub fn gateway_record_latency(
    state: State<SharedGateway>,
    authority: String,
    latency_ms: u64,
) -> Result<(), String> {
    let g = state.lock();
    g.record_latency(&authority, std::time::Duration::from_millis(latency_ms));
    Ok(())
}

/// Serialisable topology snapshot returned to the frontend.
#[derive(Debug, Serialize)]
pub struct LaneSnapshot {
    pub lane_count: usize,
    pub busy: usize,
    /// (authority, ema_ms, samples, trend)
    pub latencies: Vec<(String, f64, u64, i8)>,
}

/// Snapshot of the lane topology: lane_count, busy leases, TD-EWMA table.
#[tauri::command]
pub fn gateway_snapshot(state: State<SharedGateway>) -> Result<LaneSnapshot, String> {
    let g = state.lock();
    let lane_count = resin_core::DEFAULT_LANES;
    let busy = g.lease_table.live_count();
    let latencies = g.tdewma_snapshot();
    Ok(LaneSnapshot { lane_count, busy, latencies })
}