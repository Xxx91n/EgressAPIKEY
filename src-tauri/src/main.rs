//! Binary entrypoint for the ai-api-route desktop shell. Runs the full
//! Tauri 2 Builder pipeline: React frontend, resin-core gateway as managed
//! state, the plugin set the GUI uses, and a system tray.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ai_api_route_app::{build_shared_gateway, commands, tray::build_tray};
use resin_core::DEFAULT_LANES;
use tauri::{Manager, Emitter};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let lanes = std::env::var("AI_API_ROUTE_LANES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_LANES);

    let gateway = build_shared_gateway(lanes);

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
        // Re7: Tauri official log plugin — bridges Rust `log`/`tracing` events
        // to the frontend `@tauri-apps/plugin-log` surface (log viewer panel).
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .manage(gateway)
        .invoke_handler(tauri::generate_handler![
            commands::gateway_reserve,
            commands::gateway_release,
            commands::gateway_evict_lane,
            commands::gateway_record_latency,
            commands::gateway_snapshot,
            commands::tray_refresh_labels,
        ])
        .setup(|app| {
            build_tray(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running ai-api-route Tauri shell");
}