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
///
/// IPC input validation mirrors `gateway_record_latency`: empty api_key /
/// account / authority are rejected so lane hashing never collapses on the
/// empty-string bucket, and authority is bounded to keep TdEwma finite.
#[tauri::command]
pub fn gateway_reserve(
    state: State<SharedGateway>,
    api_key: String,
    account: String,
    authority: String,
    exit_ip: Option<String>,
) -> Result<ReserveResult, String> {
    if api_key.is_empty() || account.is_empty() {
        return Err("api_key and account must be non-empty".to_string());
    }
    validate_authority(&authority)?;
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
///
/// Defense-in-depth: the IPC layer rejects out-of-range lanes with an explicit
/// error BEFORE reaching resin-core; the kernel's `LeaseTable::evict_lane`
/// also self-guards. Either alone is sufficient; both together prevent a
/// stale/hostile lane value from panicking the proxy over the IPC boundary.
/// Maximum lane index is `resin_core::MAX_LANES - 1` (50 lanes, design ceiling).
#[tauri::command]
pub fn gateway_evict_lane(state: State<SharedGateway>, lane: usize) -> Result<(), String> {
    if lane >= resin_core::MAX_LANES {
        return Err(format!("evict_lane: lane {lane} out of range (max {})", resin_core::MAX_LANES - 1));
    }
    let g = state.lock();
    if g.evict_lane(lane) {
        Ok(())
    } else {
        Err(format!("evict_lane: lane {lane} rejected by kernel"))
    }
}

/// Record a latency sample for an authority (drives TD-EWMA trend stats).
///
/// IPC input validation:
/// - `authority` is bounded to 253 chars (DNS host ceiling) and must not
///   contain NUL / control characters; this caps TdEwma growth and blocks
///   garbage keys that would otherwise live forever in the latency table.
/// - `latency_ms` is capped to a plausible 24h upper bound so a hostile
///   caller cannot poison the EMA with u64::MAX.
const AUTHORITY_MAX_LEN: usize = 253;
const LATENCY_CAP_MS: u64 = 24 * 60 * 60 * 1000;

fn validate_authority(authority: &str) -> Result<(), String> {
    if authority.is_empty() || authority.len() > AUTHORITY_MAX_LEN {
        return Err(format!("authority length out of range (1..={AUTHORITY_MAX_LEN})"));
    }
    if authority.bytes().any(|b| b == 0 || (b < 0x20 && b != 0x09) || b == 0x7f) {
        return Err("authority contains control characters".to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn gateway_record_latency(
    state: State<SharedGateway>,
    authority: String,
    latency_ms: u64,
) -> Result<(), String> {
    validate_authority(&authority)?;
    let capped = latency_ms.min(LATENCY_CAP_MS);
    let g = state.lock();
    g.record_latency(&authority, std::time::Duration::from_millis(capped));
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
///
/// `lane_count` reads the live `GatewayState::lanes.lanes` (which already
/// ran through `sanitize_lanes` at construction) so a session launched with
/// `AI_API_ROUTE_LANES=8` does NOT report the wrong ceilin at the topology
/// canvas while still reserving on the correct hashed lane.
#[tauri::command]
pub fn gateway_snapshot(state: State<SharedGateway>) -> Result<LaneSnapshot, String> {
    let g = state.lock();
    let lane_count = g.lanes.lanes;
    let busy = g.lease_table.live_count();
    let latencies = g.tdewma_snapshot();
    Ok(LaneSnapshot { lane_count, busy, latencies })
}