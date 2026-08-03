//! Binary entrypoint for the ai-api-route desktop shell. Runs the full
//! Tauri 2 Builder pipeline: React frontend, resin-core gateway as managed
//! state, the plugin set the GUI uses, and a system tray.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ai_api_route_app::{build_shared_gateway, build_shared_registry, commands, sidecar::{boot_resin, spawn_health_poll, SidecarHandle, InterceptorPort}, tray::build_tray};
use resin_core::{CoreConfig, DEFAULT_LANES, interceptor_serve, InterceptorConfig};
use resin_core::DbPool;
use tauri::{Manager, Emitter, WindowEvent};
use tauri_plugin_store::StoreExt;

fn main() {
    // Issue 11: capture panics into the tracing pipeline so a crashed
    // sidecar thread or IPC handler surfaces a log line instead of silently
    // unwinding. The default hook prints to stderr only, which the GUI hides.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(panic = %info, backtrace = ?std::backtrace::Backtrace::force_capture(), "panic captured");
        prev_hook(info);
    }));
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
            // Issue 11: log hardening. Rotate daily OR at 10MB, keep the 7 most
            // recent files so a runaway stream cannot fill the user's disk.
            // Ponytail: use the plugin's built-in MaxFileSize + RotationStrategy
            // rather than a custom subscriber (zero new code, no overflow path).
            tauri_plugin_tracing::Builder::new()
                .with_max_level(tauri_plugin_tracing::LevelFilter::INFO)
                .with_file_logging()
                .with_rotation(tauri_plugin_tracing::Rotation::Daily)
                .with_max_file_size(tauri_plugin_tracing::MaxFileSize::mb(10))
                .with_rotation_strategy(tauri_plugin_tracing::RotationStrategy::KeepSome(7))
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
            commands::platform_list_full,
            commands::platform_snapshot,
            commands::account_add,
            commands::account_bind_ip,
            commands::gateway_select_account,
            commands::process_route_add,
            commands::process_route_remove,
            commands::process_route_list,
            commands::subscription_add,
            commands::subscription_remove,
            commands::subscription_list,
            commands::node_pool_snapshot,
            commands::platform_update,
            commands::node_list,
            commands::platform_create_with_fields,
            commands::platform_leases,
            commands::backup_create,
            commands::backup_upload,
            commands::backup_list,
            commands::config_export,
            commands::config_import,
            commands::interceptor_port,
            commands::lease_map,
            commands::observed_keys,
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

            // G1: boot the Resin Go sidecar and expose it to IPC commands (G2
            // ResinClient reads admin_token from this State). boot_resin blocks
            // up to 15s waiting for the sidecar /health endpoint; on timeout we
            // surface the error to setup so the app refuses to start rather
            // than running a half-broken shell with no proxy backend.
            let sidecar = boot_resin(app.handle())?;
            tracing::info!(
                "resin sidecar booted: api_base={}",
                sidecar.api_base()
            );
            // A4-3: capture the interceptor-feeding values BEFORE the move into
            // SidecarHandle (which consumes proxy_token/admin_token/child).
            let api_port_val = sidecar.api_port;
            let resin_base_for_interceptor = format!("http://127.0.0.1:{}", api_port_val);
            let proxy_token_for_interceptor = sidecar.proxy_token.clone();
            app.manage(SidecarHandle {
                child: sidecar.child,
                api_port: sidecar.api_port,
                admin_token: sidecar.admin_token,
                proxy_token: sidecar.proxy_token,
            });
            // C1-1: open the observed_keys SQLite pool at
            // app_config_dir()/ai-api-route.db (WAL). The Resin sidecar
            // owns its own state.db/cache.db; this is the shell's key
            // observation log driving the Topology chips. app_config_dir
            // is the same path tauri-plugin-store uses for settings.json
            // so the db sits beside user config.
            let cfg_dir = app.path().app_config_dir().unwrap_or_else(|_| {
                std::env::temp_dir().join("com.ai-api-route.desktop")
            });
            let db_path = cfg_dir.join("ai-api-route.db");
            let db = match DbPool::open(&db_path) {
                Ok(p) => {
                    tracing::info!(path = %db_path.display(), "observed_keys db opened");
                    p
                }
                Err(e) => {
                    tracing::error!(error = %e, path = %db_path.display(), "observed_keys db open failed; falling back to in-memory");
                    DbPool::open_in_memory().expect("in-memory sqlite unavailable")
                }
            };
            app.manage(db.clone());
            // G3: Ghost safety-net - /healthz poll every 3s, 3 consecutive
            // failures flip the tray red, clear OS system proxy if any, and
            // emit a sidecar-status "unhealthy" event to the webview. When
            // the sidecar recovers, tray flips green and a "healthy" event
            // is emitted. Spawn AFTER app.manage so the poll can resolve
            // State<SidecarHandle> immediately on its first iteration.
            spawn_health_poll(app.handle().clone());

            // A4-3: Spawn the local axum interceptor that injects
            // X-Resin-Account (route_id derived from auth+model+path) into
            // every request before forwarding it to the Resin reverse-proxy.
            // setup is sync, so we use tauri::async_runtime::block_on to
            // await the bind (which must complete so we can surface the port).
            // The serve() spawn then runs in the background until app exit.
            // Loopback only; the port is exposed via interceptor_port IPC.
            // proxy_token stays Rust-side (AGENTS §7.6).
            let resin_base = resin_base_for_interceptor;
            let proxy_token_val = proxy_token_for_interceptor;
            let db_for_interceptor = db.clone();
            let interceptor_port_val = tauri::async_runtime::block_on(async move {
                let db_clone = db_for_interceptor.clone();
                let make_cfg = || InterceptorConfig {
                    resin_base: resin_base.clone(),
                    proxy_token: proxy_token_val.clone(),
                    http: reqwest::Client::builder()
                        .timeout(std::time::Duration::from_secs(60))
                        .build()
                        .expect("interceptor: reqwest client"),
                    db: Some(db_clone.clone()),
                };
                // Bind 127.0.0.1:2261 first; fall back to ephemeral if taken.
                // Ponytail ceiling: 3 candidate bind addresses.
                match interceptor_serve(make_cfg(), "127.0.0.1:2261").await {
                    Ok(p) => p,
                    Err(e1) => {
                        tracing::warn!("interceptor: 2261 unavailable ({e1}), trying 2262");
                        match interceptor_serve(make_cfg(), "127.0.0.1:2262").await {
                            Ok(p) => p,
                            Err(e2) => {
                                tracing::warn!("interceptor: 2262 unavailable ({e2}), ephemeral");
                                match interceptor_serve(make_cfg(), "127.0.0.1:0").await {
                                    Ok(p) => p,
                                    Err(e3) => {
                                        tracing::error!("interceptor: bind failed ({e3}); port=0");
                                        0
                                    }
                                }
                            }
                        }
                    }
                }
            });
            tracing::info!("interceptor bound on 127.0.0.1:{}", interceptor_port_val);
            app.manage(InterceptorPort(interceptor_port_val));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building ai-api-route Tauri shell")
        .run(|app_handle, event| {
            // P22 audit fix: the Resin Go sidecar is a child process spawned
            // via tauri_plugin_shell::CommandChild. Its Drop impl in plugin
            // version 2.3.5 does NOT kill the child (you must call .kill()
            // explicitly). Without this hook, app.exit(0) from the tray Quit
            // item leaves resin.exe running as an orphan after the GUI
            // process exits, leaking the port and the SQLite state lock.
            // Tauri 2.11 Builder::run(context) hardcodes an empty closure;
            // we use .build()?.run(closure) instead so we can hook RunEvent::Exit.
            if let tauri::RunEvent::Exit = event {
                if let Some(sidecar) = app_handle.try_state::<SidecarHandle>() {
                    // CommandChild::kill takes self (consumes); wrap child in
                    // Mutex<Option<_>> so we can take() once here. A second
                    // Exit (if ever re-emitted) finds None and no-ops.
                    if let Some(child) = sidecar.child.lock().ok().and_then(|mut g| g.take()) {
                        let _ = child.kill();
                        tracing::info!("sidecar child killed on app exit");
                    }
                }
            }
        });
}
