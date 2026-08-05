//! Binary entrypoint for the EgressAPIKEY desktop shell. Runs the full
//! Tauri 2 Builder pipeline: React frontend, resin-core gateway as managed
//! state, the plugin set the GUI uses, and a system tray.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use egressapikey_app::{
    build_shared_gateway, build_shared_registry, commands,
    sidecar::{boot_resin, spawn_health_poll, SidecarHandle},
    tray::build_tray,
};
use resin_core::{CoreConfig, DEFAULT_LANES};
use resin_core::{
    DbPool, PortForwarder, WhiteboxConfig, WhiteboxConfigStore, WHITEBOX_CONFIG_FILE,
};
use tauri::{Emitter, Manager, WindowEvent};
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
            commands::lease_map,
            commands::ip_reputation_snapshot,
            commands::port_list,
            commands::port_upsert,
            commands::port_remove,
            commands::port_running,
            commands::port_reload,
            commands::whitebox_path,
            commands::whitebox_get,
            commands::whitebox_reload,
            commands::stream_sensor_snapshot,
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
                "EgressAPIKEY config: lanes={}, bind={}, mihomo_api={}",
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
            let api_port = sidecar.api_port;
            let proxy_token = sidecar.proxy_token;
            app.manage(SidecarHandle {
                child: sidecar.child,
                api_port,
                admin_token: sidecar.admin_token,
                proxy_token: proxy_token.clone(),
            });
            // Port->platform mapping SQLite store (ADR-0012)
            // The multi-port listener reads this to inject X-Resin-Account
            // based on which port the request arrived on.
            let cfg_dir = app.path().app_config_dir().unwrap_or_else(|_| {
                std::env::temp_dir().join("com.egressapikey.desktop")
            });
            let db_path = cfg_dir.join("egressapikey.db");
            let db = match DbPool::open(&db_path) {
                Ok(p) => {
                    tracing::info!(path = %db_path.display(), "port_mappings db opened");
                    p
                }
                Err(e) => {
                    tracing::error!(error = %e, path = %db_path.display(), "port_mappings db open failed; falling back to in-memory");
                    DbPool::open_in_memory().expect("in-memory sqlite unavailable")
                }
            };
            app.manage(db.clone());
            // Phase 2: multi-port thin forwarder (ADR-0012). Port = identity.
            // Listens on each enabled port_mappings row and rewrites proxy-auth
            // to Platform.Account:proxy_token before tunneling to Resin.
            let forwarder = PortForwarder::new(
                db.clone(),
                "127.0.0.1",
                api_port,
                proxy_token,
            );
            // Phase 3 / NEW-7: whitebox entry-port JSON + hotswap-config.
            // Seed from SQLite so first boot materializes the file from DB.
            let whitebox_path = cfg_dir.join(WHITEBOX_CONFIG_FILE);
            let seed = WhiteboxConfig::from_ports(db.list_ports().unwrap_or_default());
            let store = tauri::async_runtime::block_on(async {
                match WhiteboxConfigStore::open(whitebox_path.clone(), seed.clone()).await {
                    Ok(s) => Ok(s),
                    Err(e) => {
                        // Corrupt/hand-broken JSON: quarantine and reseed from DB so
                        // State<WhiteboxConfigStore> always exists for port_* IPC.
                        tracing::error!(error = %e, "whitebox config open failed; reseeding from DB");
                        let bak = whitebox_path.with_extension("json.bad");
                        let _ = std::fs::rename(&whitebox_path, &bak);
                        WhiteboxConfigStore::open(whitebox_path, seed).await
                    }
                }
            });
            match store {
                Ok(store) => {
                    let db_wb = db.clone();
                    let fwd_wb = forwarder.clone();
                    let store_watch = store.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = store_watch.reload_file(&db_wb, &fwd_wb).await {
                            tracing::warn!(error = %e, "whitebox: initial apply failed; reloading listeners from DB");
                            if let Err(e2) = fwd_wb.reload().await {
                                tracing::warn!(error = %e2, "port_forwarder: initial reload failed");
                            }
                        }
                        store_watch.watch_apply(db_wb, fwd_wb).await;
                    });
                    tracing::info!(path = %store.path().display(), "whitebox config opened");
                    app.manage(store);
                }
                Err(e) => {
                    // Last-resort: still manage a store under temp so IPC does not panic.
                    tracing::error!(error = %e, "whitebox config unrecoverable; using temp store");
                    let tmp = std::env::temp_dir().join(format!(
                        "egressapikey-ports-{}.json",
                        std::process::id()
                    ));
                    let seed2 = WhiteboxConfig::from_ports(db.list_ports().unwrap_or_default());
                    match tauri::async_runtime::block_on(WhiteboxConfigStore::open(tmp, seed2)) {
                        Ok(store) => {
                            let db_wb = db.clone();
                            let fwd_wb = forwarder.clone();
                            let store_watch = store.clone();
                            tauri::async_runtime::spawn(async move {
                                let _ = store_watch.reload_file(&db_wb, &fwd_wb).await;
                                store_watch.watch_apply(db_wb, fwd_wb).await;
                            });
                            app.manage(store);
                        }
                        Err(e2) => {
                            tracing::error!(error = %e2, "temp whitebox open failed");
                            let fwd = forwarder.clone();
                            tauri::async_runtime::spawn(async move {
                                if let Err(e3) = fwd.reload().await {
                                    tracing::warn!(error = %e3, "port_forwarder: initial reload failed");
                                }
                            });
                        }
                    }
                }
            }
            app.manage(forwarder);
            // G3: Ghost safety-net - /healthz poll every 3s, 3 consecutive
            // failures flip the tray red, clear OS system proxy if any, and
            // emit a sidecar-status "unhealthy" event to the webview. When
            // the sidecar recovers, tray flips green and a "healthy" event
            // is emitted. Spawn AFTER app.manage so the poll can resolve
            // State<SidecarHandle> immediately on its first iteration.
            spawn_health_poll(app.handle().clone());

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building EgressAPIKEY Tauri shell")
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
