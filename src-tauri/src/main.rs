//! Binary entrypoint for the ai-api-route desktop shell. Runs the full
//! Tauri 2 Builder pipeline: React frontend, resin-core gateway as managed
//! state, the plugin set the GUI uses, and a system tray.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ai_api_route_app::{build_shared_gateway, commands, tray::build_tray};
use resin_core::DEFAULT_LANES;

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
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .manage(gateway)
        .invoke_handler(tauri::generate_handler![
            commands::gateway_reserve,
            commands::gateway_release,
            commands::gateway_evict_lane,
            commands::gateway_record_latency,
            commands::gateway_snapshot,
        ])
        .setup(|app| {
            build_tray(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running ai-api-route Tauri shell");
}