//! Binary entrypoint for the ai-api-route desktop shell. Runs the full
//! Tauri 2 Builder pipeline: React frontend, resin-core gateway as managed
//! state, the plugin set the GUI uses, and a system tray.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ai_api_route_app::{build_shared_gateway, build_shared_registry, commands, tray::build_tray};
use resin_core::{CoreConfig, DEFAULT_LANES};
use tauri::{Manager, Emitter, WindowEvent};
use tauri_plugin_store::StoreExt;

fn main() {
    let lanes = std::env::var("AI_API_ROUTE_LANES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_LANES);

    let gateway = build_shared_gateway(lanes);
    let registry = build_shared_registry();

    tauri::Builder::default()
        // Re9: Single-instance must be the FIRST plugin registered (Tauri 2
        // requirement). When a second GUI exe is launched, this plugin kills
        // the new process and runs the closure in the already-running instance:
        // we focus the existing main window instead of opening a second GUI.
        // A panic in this callback would take down the running app, so every
        // step is fallible and logged; we never `.expect()` on window state.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            } else {
                tracing::warn!("single-instance callback: main window not found; rebroadcasting new-instance event without focus");
                let _ = app.emit("single-instance-launched", ());
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        // Re7: tauri-plugin-tracing — one `tracing` pipeline for resin-core +
        // shell. Wires stdout + a WebviewLayer (Rust logs -> frontend log panel
        // via the `tracing://log` event the npm guest binds) + a daily-rotating
        // file appender under app_log_dir() (Re6). Replaces tauri-plugin-log so
        // every `tracing::` macro in the codebase flows through one subscriber.
        .plugin(
            tauri_plugin_tracing::Builder::new()
                .with_max_level(tauri_plugin_tracing::LevelFilter::INFO)
                .with_file_logging()
                .with_default_subscriber()
                .build(),
        )
        .manage(gateway)
        .manage(registry)
        // Closing the main window hides to tray instead of quitting the app
        // (problem 5). The tray "Quit" item is the real exit path; the tray
        // left-click and "Show Window" item restore the hidden window. We
        // prevent the default close so the process keeps running for SSE
        // streams and lane leases while the user has dismissed the GUI.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Best-effort hide; never panic if the window is already gone.
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::gateway_reserve,
            commands::gateway_release,
            commands::gateway_evict_lane,
            commands::gateway_record_latency,
            commands::gateway_snapshot,
            commands::tray_refresh_labels,
            // #7: open config / log directory buttons in Settings.
            commands::get_config_dir,
            commands::get_log_dir,
            // Re3: Platform/Account registry + weighted account selection.
            commands::platform_add,
            commands::platform_remove,
            commands::platform_list,
            commands::platform_snapshot,
            commands::account_add,
            commands::account_bind_ip,
            commands::gateway_select_account,
        ])
        .setup(|app| {
            // #2/#6: read persisted network settings so the user
            // edits to gatewayBind/mihomoApi actually reach the Rust
            // side (previously the shell always used CoreConfig::default
            // and ignored settings.json for these keys). We construct a
            // CoreConfig from the persisted values and log it; the live
            // gateway listen (axum) wiring is the next #6 phase. We never
            // panic on missing/invalid values - we fall back to defaults.
            let mut cfg = CoreConfig::default();
            if let Ok(store) = app.store("settings.json") {
                // tauri-plugin-store 2.4.4 store.get returns Option<JsonValue>;
                // match on serde_json::Value variants for string + number keys.
                if let Some(serde_json::Value::String(v)) = store.get("gatewayBind") {
                    if !v.trim().is_empty() { cfg.bind = v.trim().to_string(); }
                }
                if let Some(serde_json::Value::String(v)) = store.get("mihomoApi") {
                    if !v.trim().is_empty() { cfg.mihomo_api = v.trim().to_string(); }
                }
                if let Some(serde_json::Value::Number(n)) = store.get("laneCount") {
                    if let Some(u) = n.as_u64() {
                        cfg.lanes = resin_core::sanitize_lanes(u as usize);
                    } else if let Some(f) = n.as_f64() {
                        cfg.lanes = resin_core::sanitize_lanes(f as usize);
                    }
                }
            }
            tracing::info!(
                "ai-api-route config: lanes={}, bind={}, mihomo_api={}",
                cfg.lanes, cfg.bind, cfg.mihomo_api
            );
            // The CoreConfig is dropped here today; the live gateway state
            // (managed below) still uses the env/DEFAULT_LANES value for
            // the lane count pending the axum listen + mihomo sidecar
            // lifecycle port (Resin proxy runtime, #6 next phase).
            let _ = cfg;
            build_tray(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running ai-api-route Tauri shell");
}
