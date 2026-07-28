//! IPC commands exposed to the React frontend via `tauri::generate_handler!`.
//!
//! Each command locks the shared GatewayState for one short critical section
//! and never awaits across the lock, so the per-lane Mutex lease invariants
//! in resin-core stay intact.

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::{SharedGateway, SharedRegistry};
use resin_core::platform::{Account, Platform};

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
/// empty-string bucket, and authority is bounded to keep TdEwma finite. All
/// free-text inputs are length-capped (KEY_MAX_LEN / AUTHORITY_MAX_LEN /
/// IP_MAX_LEN) so a hostile or misbehaving caller cannot grow memory by
/// passing multi-MB strings each call — defense-in-depth (AGENTS §7.5).
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
    if api_key.len() > KEY_MAX_LEN || account.len() > KEY_MAX_LEN {
        return Err(format!("api_key/account length out of range (1..={KEY_MAX_LEN})"));
    }
    validate_authority(&authority)?;
    if let Some(ip) = exit_ip.as_deref() {
        validate_ip(ip)?;
    }
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
/// Cap for free-text IPC identifiers (api_key, account). Generous: real
/// api keys are <256 chars; 4096 covers exotic providers without leaving a
/// surface for hostile multi-MB allocations from a misbehaving caller.
const KEY_MAX_LEN: usize = 4096;

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

/// Live-refresh tray labels after the user changes the UI language in
/// SettingsView (Re10). Stateless: does NOT take SharedGateway, so a flood of
/// language toggles cannot contend on the gateway lock. The tray itself owns
/// the public label text; the only failure mode (no tray yet) is silently
/// ignored via `apply_labels` returning Ok.
#[tauri::command]
pub fn tray_refresh_labels(app: AppHandle) -> Result<(), String> {
    crate::tray::apply_labels(&app).map_err(|e| format!("tray_refresh_labels: {e:?}"))
}

// ---- Re3 upper-layer wiring: Platform/Account registry over IPC ----
//
// All inputs validated at the IPC boundary per AGENTS.md §7.5 BEFORE touching
// registry: lane range, name/id length + control chars, IP basic shape.

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

/// Basic IPv4/IPv6 sanity without pulling a parse dep; rejects empty, control
/// chars, and obvious garbage. mihomo validates the real binding downstream.
fn validate_ip(ip: &str) -> Result<(), String> {
    if ip.is_empty() || ip.len() > 253 {
        return Err("exit_ip length out of range".to_string());
    }
    if ip.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f || b == b' ') {
        return Err("exit_ip contains control/space characters".to_string());
    }
    Ok(())
}

/// Add (or replace) an empty Platform. Idempotent on name.
#[tauri::command]
pub fn platform_add(reg: State<SharedRegistry>, name: String) -> Result<(), String> {
    validate_short_name(&name, "platform")?;
    reg.upsert(Platform::new(name));
    Ok(())
}

/// Remove a Platform. Returns true if it existed.
#[tauri::command]
pub fn platform_remove(reg: State<SharedRegistry>, name: String) -> Result<bool, String> {
    validate_short_name(&name, "platform")?;
    Ok(reg.remove(&name))
}

/// List all Platform names.
#[tauri::command]
pub fn platform_list(reg: State<SharedRegistry>) -> Result<Vec<String>, String> {
    Ok(reg.list())
}

/// Snapshot of one Platform's accounts (serialised).
#[tauri::command]
pub fn platform_snapshot(reg: State<SharedRegistry>, name: String) -> Result<Vec<Account>, String> {
    validate_short_name(&name, "platform")?;
    reg.account_snapshot(&name).ok_or_else(|| format!("platform not found: {name}"))
}

/// Add an account to a platform with a bound lane. Validates lane range and
/// id; upserts (does not dedupe — frontend manages uniqueness).
#[tauri::command]
pub fn account_add(
    reg: State<SharedRegistry>,
    platform: String,
    id: String,
    lane: usize,
) -> Result<(), String> {
    validate_short_name(&platform, "platform")?;
    validate_short_name(&id, "account")?;
    if lane >= resin_core::MAX_LANES {
        return Err(format!("lane {lane} out of range (max {})", resin_core::MAX_LANES - 1));
    }
    let p = reg.get(&platform).ok_or_else(|| format!("platform not found: {platform}"))?;
    p.write().add_account(Account::new(id, platform, lane));
    Ok(())
}

/// Bind an anchored exit IP to an account.
#[tauri::command]
pub fn account_bind_ip(
    reg: State<SharedRegistry>,
    platform: String,
    account: String,
    ip: String,
) -> Result<bool, String> {
    validate_short_name(&platform, "platform")?;
    validate_short_name(&account, "account")?;
    validate_ip(&ip)?;
    Ok(reg.bind_ip(&platform, &account, &ip))
}

#[derive(Debug, serde::Serialize)]
pub struct SelectResult {
    pub account: Option<String>,
    pub lane: usize,
    pub exit_ip: Option<String>,
    /// "none" when no active account, else the account id.
    pub reason: String,
}

/// Re3: select a routable account for (platform, api_key). The lane is hashed
/// from api_key (so the same key always targets the same lane — SSE stickiness
/// invariant); among active accounts on that platform we pick:
///   - weighted = false: pick_account — deterministic lane-prefer
///   - weighted = true:  pick_account_weighted — TD-EWMA P2C path, latency
///     pulled from the gateway's tdewma for `authority` (cold lanes win when
///     no sample). The latency source is pluggable; wiring real per-account
///     EMA here is the documented extension point.
#[tauri::command]
pub fn gateway_select_account(
    gw: State<SharedGateway>,
    reg: State<SharedRegistry>,
    platform: String,
    api_key: String,
    authority: String,
    weighted: Option<bool>,
) -> Result<SelectResult, String> {
    validate_short_name(&platform, "platform")?;
    if api_key.is_empty() {
        return Err("api_key must be non-empty".to_string());
    }
    validate_authority(&authority)?;
    let p = reg.get(&platform).ok_or_else(|| format!("platform not found: {platform}"))?;
    let prefer_lane = {
        let g = gw.lock();
        resin_core::lane_index(&api_key, &g.lanes)
    };
    if weighted.unwrap_or(false) {
        let g = gw.lock();
        // Weighted selection: TD-EWMA latency lookup for the authority. Cold
        // lanes (no sample) win via pick_account_weighted's `(has_sample,
        // latency, lane_match, lane)` ordering - has_sample=0 sorts first.
        // Gateway lock held only across the synchronous pick; no await.
        let plat = p.read();
        let acc = plat
            .pick_account_weighted(prefer_lane, |_| g.tdewma.get(&authority).map(|s| s.ema_ms));
        Ok(match acc {
            Some(a) => SelectResult {
                account: Some(a.id.clone()),
                lane: a.lane,
                exit_ip: a.exit_ip.clone(),
                reason: a.id.clone(),
            },
            None => SelectResult {
                account: None,
                lane: prefer_lane,
                exit_ip: None,
                reason: "none".into(),
            },
        })
    } else {
        let plat = p.read();
        let acc = plat.pick_account(prefer_lane);
        Ok(match acc {
            Some(a) => SelectResult {
                account: Some(a.id.clone()),
                lane: a.lane,
                exit_ip: a.exit_ip.clone(),
                reason: a.id.clone(),
            },
            None => SelectResult {
                account: None,
                lane: prefer_lane,
                exit_ip: None,
                reason: "none".into(),
            },
        })
    }
}


/// Return the app config directory (where settings.json lives) so the
/// Settings Open-config-directory button can open it in the file manager.
/// Path comes from the Tauri Manager path API (server-trusted); the webview
/// never supplies the path, so there is no arbitrary-open risk — only the
/// app own dirs are ever returned. Empty-string path is impossible here
/// (app_config_dir only errors when the OS cannot resolve the base, which
/// is rare); the JS handler treats Err as a no-op toast.
#[tauri::command]
pub fn get_config_dir(app: AppHandle) -> Result<String, String> {
    match app.path().app_config_dir() {
        Ok(p) => Ok(p.to_string_lossy().into_owned()),
        Err(e) => Err(format!("app_config_dir: {e:?}")),
    }
}

/// Return the app log directory (tauri-plugin-tracing daily-rotating file
/// appender writes here). Same trust model as get_config_dir.
#[tauri::command]
pub fn get_log_dir(app: AppHandle) -> Result<String, String> {
    match app.path().app_log_dir() {
        Ok(p) => Ok(p.to_string_lossy().into_owned()),
        Err(e) => Err(format!("app_log_dir: {e:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-C2 regression: the IPC api_key/account length cap constant exists
    /// and is non-trivial. A future edit that drops KEY_MAX_LEN to 0 or
    /// removes the constant would let a huge string through unguarded.
    #[test]
    fn key_max_len_is_reasonable_cap() {
        assert!(KEY_MAX_LEN >= 64, "KEY_MAX_LEN too small: {KEY_MAX_LEN}");
        assert!(KEY_MAX_LEN <= 32_768, "KEY_MAX_LEN absurdly large: {KEY_MAX_LEN}");
    }

    /// validate_authority: rejects empty, over-cap, NUL/control, accepts a
    /// normal host. This is the guard `gateway_reserve` + `gateway_record_latency`
    /// both route through; a regression that lets garbage through would poison
    /// the TD-EWMA table or the lease bucket.
    #[test]
    fn validate_authority_accepts_normal_rejects_bad() {
        assert!(validate_authority("api.openai.com").is_ok());
        assert!(validate_authority("").is_err());
        assert!(validate_authority(&"x".repeat(AUTHORITY_MAX_LEN + 1)).is_err());
        // NUL + a control char (0x01) + DEL must be rejected; tab (0x09) allowed.
        assert!(validate_authority("a\x00b").is_err());
        assert!(validate_authority("a\x01b").is_err());
        assert!(validate_authority("a\x7fb").is_err());
        assert!(validate_authority("a\tb").is_ok());
    }

    /// validate_ip: rejects empty / over-cap / control / space. gateway_reserve
    /// now routes exit_ip through this guard too (F-C2), so a hostile IP string
    /// can never reach the lease table.
    #[test]
    fn validate_ip_accepts_normal_rejects_bad() {
        assert!(validate_ip("203.0.113.7").is_ok());
        assert!(validate_ip("::1").is_ok());
        assert!(validate_ip("").is_err());
        assert!(validate_ip(&"1".repeat(254)).is_err());
        assert!(validate_ip("127.0.0.1 x").is_err()); // space
        assert!(validate_ip("127.0.\x00.1").is_err()); // NUL
    }

    /// validate_short_name: empty / over-cap / control rejected; normal ok.
    #[test]
    fn validate_short_name_bounds() {
        assert!(validate_short_name("openai", "platform").is_ok());
        assert!(validate_short_name("", "platform").is_err());
        assert!(validate_short_name(&"x".repeat(NAME_MAX_LEN + 1), "account").is_err());
        assert!(validate_short_name("a\x01z", "platform").is_err());
    }
}
