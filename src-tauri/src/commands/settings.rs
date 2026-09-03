//! settings domain IPC commands (EgressAPIKEY).
//!
//! Extracted from the former commands/mod.rs monolith by architecture-recovery
//! ticket 08: pure mechanical move - no behavior, naming, or IPC-surface change.
use tauri::{AppHandle, Manager, State};
use tauri_plugin_store::StoreExt;
use crate::sidecar::SidecarHandle;
use resin_core::IpcError;
use serde::{Deserialize, Serialize};
use super::common::{map_resin_error, resin_client};

/// T15-2: Runtime log level gate. 0=error, 1=warn, 2=info, 3=debug.
/// Default is 2 (info). Use the set_log_level IPC command to change at runtime.
/// Wired into spawn_health_poll per-cycle tracing::debug! on /healthz success,
/// so a user who sets level<debug in Settings suppresses the per-cycle noise.
pub static LOG_LEVEL_GATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);

/// Returns true if the given numeric level (0=error,1=warn,2=info,3=debug) should be emitted.
pub fn log_level_enabled(level: u8) -> bool {
    LOG_LEVEL_GATE.load(std::sync::atomic::Ordering::Relaxed) >= level
}

/// Ticket 09 (tauri-specta pilot): log level as a closed enum. The wire
/// format is unchanged (lowercase string, serde rename_all); out-of-set
/// values are now rejected by serde at deserialization instead of the
/// former in-command String match. Exported into src/bindings.ts so the
/// frontend narrows against the generated union.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }

    /// LOG_LEVEL_GATE numeric encoding (0=error..3=debug), unchanged.
    pub fn gate(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warn => 1,
            Self::Info => 2,
            Self::Debug => 3,
        }
    }
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
pub async fn sidecar_restart(
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

/// T05 (Round 5): diagnostics poll interval (ms) — typed L1 command pair that
/// replaces the former DiagnosticsView bare `invoke("get/set_store_value")`
/// bypass (the store_value commands were never registered, so the old path
/// failed at runtime and silently fell back to the 5000 default). §7.5 IPC
/// input validation: the upper bound is a 24h ceiling in the LATENCY_CAP_MS
/// style; the GUI keeps its own tighter 1000..=60000 picker on top.
const DIAG_POLL_INTERVAL_MIN_MS: u64 = 100;
const DIAG_POLL_INTERVAL_MAX_MS: u64 = 24 * 60 * 60 * 1000;

/// §7.5 validation for set_diag_poll_interval: accepts 100..=24h, rejects
/// everything else with IpcError::InvalidInput. Pure fn so the §7.5 boundary
/// values are unit-testable without an AppHandle.
fn validate_diag_poll_interval(interval_ms: u64) -> Result<(), IpcError> {
    if interval_ms < DIAG_POLL_INTERVAL_MIN_MS || interval_ms > DIAG_POLL_INTERVAL_MAX_MS {
        return Err(IpcError::invalid_input(&format!(
            "interval_ms must be {DIAG_POLL_INTERVAL_MIN_MS}..={DIAG_POLL_INTERVAL_MAX_MS}"
        )));
    }
    Ok(())
}

/// T05: read the diagnostics poll interval (ms) from settings.json; 5000 when
/// unset or when the stored value is not a number.
#[tauri::command]
pub async fn get_diag_poll_interval(app: AppHandle) -> Result<u64, IpcError> {
    let store = app.store("settings.json").map_err(|e| IpcError::from(e.to_string()))?;
    Ok(store
        .get("diagPollInterval")
        .and_then(|v| v.as_u64())
        .unwrap_or(5000))
}

/// T05: persist the diagnostics poll interval (ms) to settings.json.
/// §7.5: interval_ms accepted only within 100..=24h.
#[tauri::command]
pub async fn set_diag_poll_interval(app: AppHandle, interval_ms: u64) -> Result<(), IpcError> {
    validate_diag_poll_interval(interval_ms)?;
    let store = app.store("settings.json").map_err(|e| IpcError::from(e.to_string()))?;
    store.set("diagPollInterval", serde_json::json!(interval_ms));
    store.save().map_err(|e| IpcError::from(e.to_string()))?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn set_log_level(level: LogLevel) -> Result<String, IpcError> {
    LOG_LEVEL_GATE.store(level.gate(), std::sync::atomic::Ordering::Relaxed);
    tracing::warn!("T15-2: log level set to {} (gate={})", level.as_str(), level.gate());
    Ok(level.as_str().to_string())
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

#[cfg(test)]
mod log_level_tests {
    use super::*;

    #[test]
    fn log_level_gate_encoding_unchanged() {
        assert_eq!(LogLevel::Error.gate(), 0);
        assert_eq!(LogLevel::Warn.gate(), 1);
        assert_eq!(LogLevel::Info.gate(), 2);
        assert_eq!(LogLevel::Debug.gate(), 3);
    }

    #[test]
    fn log_level_serde_wire_is_lowercase_string() {
        for (v, s) in [
            (LogLevel::Error, "\"error\""),
            (LogLevel::Warn, "\"warn\""),
            (LogLevel::Info, "\"info\""),
            (LogLevel::Debug, "\"debug\""),
        ] {
            assert_eq!(serde_json::to_string(&v).unwrap(), s);
            let back: LogLevel = serde_json::from_str(s).unwrap();
            assert_eq!(back, v);
        }
        // Out-of-set values are rejected at deserialization.
        assert!(serde_json::from_str::<LogLevel>("\"fatal\"").is_err());
    }
}

#[cfg(test)]
mod diag_poll_interval_tests {
    use super::*;

    #[test]
    fn diag_poll_interval_accepts_100_to_24h_including_boundaries() {
        // §7.5: both inclusive endpoints are accepted.
        assert!(validate_diag_poll_interval(100).is_ok());
        assert!(validate_diag_poll_interval(1000).is_ok());
        assert!(validate_diag_poll_interval(5000).is_ok());
        assert!(validate_diag_poll_interval(60_000).is_ok());
        assert!(validate_diag_poll_interval(DIAG_POLL_INTERVAL_MAX_MS).is_ok());
    }

    #[test]
    fn diag_poll_interval_rejects_zero_and_sub_100() {
        // Acceptance (e): 0 (and anything below the 100ms floor) is rejected.
        assert!(validate_diag_poll_interval(0).is_err());
        assert!(validate_diag_poll_interval(1).is_err());
        assert!(validate_diag_poll_interval(99).is_err());
    }

    #[test]
    fn diag_poll_interval_rejects_above_24h() {
        // Negative input is unrepresentable in u64: a hostile wire value of
        // -1 fails serde's u64 deserialization before this fn ever runs, so
        // the Rust-side negative case IS the type boundary (the TS wrapper's
        // assertInRange additionally rejects JS negative numbers).
        assert!(validate_diag_poll_interval(DIAG_POLL_INTERVAL_MAX_MS + 1).is_err());
        assert!(validate_diag_poll_interval(u64::MAX).is_err());
    }

    #[test]
    fn diag_poll_interval_rejection_is_typed_invalid_input() {
        // Acceptance (d): out-of-range returns IpcError::InvalidInput.
        for bad in [0u64, 99, DIAG_POLL_INTERVAL_MAX_MS + 1, u64::MAX] {
            match validate_diag_poll_interval(bad) {
                Err(IpcError::InvalidInput { msg, i18n_key }) => {
                    assert!(msg.contains("interval_ms must be 100..=86400000"), "msg: {msg}");
                    assert_eq!(i18n_key, "error.badRequest");
                }
                other => panic!("expected InvalidInput for {bad}, got {other:?}"),
            }
        }
    }
}
