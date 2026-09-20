//! BFF-native shell routes (R11-03, ADR-0071 option C).
//!
//! Every command that reads or writes control-plane state gets a real
//! endpoint here — NOT a proxy pass-through and NOT a disabled stub. The
//! handlers reuse the same `*_impl` bodies the Tauri commands call; only the
//! dependency roots differ (state_root vs Tauri app dirs). Commands that
//! genuinely assume a desktop environment (tray, OS dialogs, Tauri Channel
//! push streams, local-path pickers) stay off this table and are reported
//! disabled by /api/v1/capabilities.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::Router;

use egressapikey_app::commands;
use resin_core::IpcError;

use crate::{port_err, port_ipc_err, read_json_body, PortCtx};

/// Directories the sidecar restart seam needs (R11-03): identical to the set
/// `boot_resin_standalone` resolved at startup — kept on the ctx so
/// close_all_connections / reset_kernel can re-enter it without re-deriving.
#[derive(Clone)]
pub struct RestartDirs {
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub log_dir: PathBuf,
    pub binary_path: PathBuf,
}

/// Machine-readable capability list served at GET /api/v1/capabilities. The
/// JSON document is the SINGLE source of truth for both sides of the seam:
/// the SPA derives its disabled-command registry from the same file, and the
/// contract check (scripts/headless-capability-check.cjs) asserts the union
/// covers every registered command exactly once.
pub const CAPABILITIES_JSON: &str = include_str!("../../src/lib/headless-capabilities.json");

async fn json_body(body: &Bytes) -> Result<serde_json::Value, Response> {
    read_json_body(body).map_err(|e| port_err(StatusCode::BAD_REQUEST, &e))
}

fn str_arg<'v>(v: &'v serde_json::Value, key: &str) -> Result<String, Response> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            port_err(
                StatusCode::BAD_REQUEST,
                &format!("missing string arg {key}"),
            )
        })
}

fn str_arg_opt(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(str::to_string)
}

// ── Settings KV (L1 preferences) ─────────────────────────────────────────
// The headless settings document lives at <state_root>/settings.json — the
// same flat key/value shape tauri-plugin-store writes on the desktop. Reads
// are whole-document GETs; writes merge the posted keys over the live doc.

fn read_settings_doc(path: &Path) -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn write_settings_doc(
    path: &Path,
    doc: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), IpcError> {
    let bytes = serde_json::to_vec_pretty(doc).map_err(|e| IpcError::from(e.to_string()))?;
    resin_core::whitebox_backup::atomic_write_bytes(path, &bytes).map_err(IpcError::from)
}

async fn settings_get_h(ctx: Arc<PortCtx>) -> Response {
    axum::Json(serde_json::Value::Object(read_settings_doc(
        &ctx.settings_path,
    )))
    .into_response()
}

async fn settings_put_h(ctx: Arc<PortCtx>, body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(patch) = v.as_object() else {
        return port_err(
            StatusCode::BAD_REQUEST,
            "settings PUT body must be a JSON object",
        );
    };
    let mut doc = read_settings_doc(&ctx.settings_path);
    for (k, val) in patch {
        doc.insert(k.clone(), val.clone());
    }
    match write_settings_doc(&ctx.settings_path, &doc) {
        Ok(()) => axum::Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn diag_poll_get_h(ctx: Arc<PortCtx>) -> Response {
    let v = read_settings_doc(&ctx.settings_path)
        .get("diagPollInterval")
        .and_then(|v| v.as_u64())
        .unwrap_or(5000);
    axum::Json(v).into_response()
}

async fn diag_poll_put_h(ctx: Arc<PortCtx>, body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let interval_ms = v.get("interval_ms").and_then(|x| x.as_u64()).unwrap_or(0);
    if let Err(e) = commands::validate_diag_poll_interval(interval_ms) {
        return port_ipc_err(&e);
    }
    let mut doc = read_settings_doc(&ctx.settings_path);
    doc.insert(
        "diagPollInterval".to_string(),
        serde_json::Value::from(interval_ms),
    );
    match write_settings_doc(&ctx.settings_path, &doc) {
        Ok(()) => axum::Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn log_level_get_h() -> Response {
    match commands::get_log_level().await {
        Ok(level) => axum::Json(level).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn log_level_put_h(body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let level = match str_arg(&v, "level") {
        Ok(s) => s,
        Err(r) => return r,
    };
    // The command takes a LogLevel enum — the wire arg is the lowercase name.
    let parsed: commands::LogLevel = match serde_json::from_value(serde_json::Value::String(level))
    {
        Ok(l) => l,
        Err(e) => return port_err(StatusCode::BAD_REQUEST, &format!("bad log level: {e}")),
    };
    match commands::set_log_level(parsed).await {
        Ok(applied) => axum::Json(applied).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn ip_reputation_h(ctx: Arc<PortCtx>) -> Response {
    let client = match ctx.client() {
        Ok(c) => c,
        Err(e) => return port_err(StatusCode::BAD_GATEWAY, &e),
    };
    let settings_path = ctx.settings_path.clone();
    match commands::ip_reputation_snapshot_impl(
        move |k| read_settings_doc(&settings_path).get(k).cloned(),
        &client,
    )
    .await
    {
        Ok(snap) => axum::Json(snap).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

// ── L2 whitebox (ports document) ─────────────────────────────────────────

async fn whitebox_get_h(ctx: Arc<PortCtx>) -> Response {
    axum::Json(ctx.whitebox.snapshot()).into_response()
}

async fn whitebox_reload_h(ctx: Arc<PortCtx>) -> Response {
    match ctx
        .whitebox
        .reload_file(&ctx.db, &ctx.forwarder)
        .await
        .map_err(IpcError::from)
    {
        Ok(n) => axum::Json(n).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn whitebox_network_h(ctx: Arc<PortCtx>, body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let network: resin_core::NetworkConfig =
        match serde_json::from_value(v.get("network").cloned().unwrap_or(v)) {
            Ok(n) => n,
            Err(e) => {
                return port_err(StatusCode::BAD_REQUEST, &format!("bad network config: {e}"))
            }
        };
    let mut current = ctx.whitebox.snapshot();
    current.network = network;
    match ctx
        .whitebox
        .apply(&ctx.db, &ctx.forwarder, current)
        .await
        .map_err(IpcError::from)
    {
        Ok(n) => axum::Json(n).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn whitebox_backups_h(ctx: Arc<PortCtx>) -> Response {
    match ctx.whitebox.list_backups().map_err(IpcError::from) {
        Ok(list) => axum::Json(list).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn whitebox_rollback_h(ctx: Arc<PortCtx>, body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let backup_name = match str_arg(&v, "backup_name") {
        Ok(s) => s,
        Err(r) => return r,
    };
    match commands::whitebox_rollback_impl(
        ctx.sidecar.as_ref(),
        &ctx.db,
        &ctx.forwarder,
        &ctx.whitebox,
        backup_name,
    )
    .await
    {
        Ok(n) => axum::Json(n).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

// ── L2 strategy document ─────────────────────────────────────────────────

async fn strategy_config_get_h(ctx: Arc<PortCtx>) -> Response {
    match ctx.strategy.get().map_err(IpcError::from) {
        Ok(cfg) => axum::Json(cfg).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn strategy_config_put_h(ctx: Arc<PortCtx>, body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let cfg: resin_core::StrategyConfig =
        match serde_json::from_value(v.get("config").cloned().unwrap_or(v)) {
            Ok(c) => c,
            Err(e) => {
                return port_err(
                    StatusCode::BAD_REQUEST,
                    &format!("bad strategy config: {e}"),
                )
            }
        };
    match ctx.strategy.store(cfg).map_err(IpcError::from) {
        Ok(stored) => axum::Json(stored).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn strategy_regions_h(ctx: Arc<PortCtx>, body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let platform_name = match str_arg(&v, "platform_name") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let regions: Vec<String> = v
        .get("regions")
        .and_then(|r| serde_json::from_value(r.clone()).ok())
        .unwrap_or_default();
    match ctx
        .strategy
        .set_platform_regions(&platform_name, regions)
        .map_err(IpcError::from)
    {
        Ok(cfg) => axum::Json(cfg).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn strategy_backups_h(ctx: Arc<PortCtx>) -> Response {
    match ctx
        .strategy
        .store_ref()
        .list_backups()
        .map_err(IpcError::from)
    {
        Ok(list) => axum::Json(list).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn strategy_rollback_h(ctx: Arc<PortCtx>, body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let backup_name = match str_arg(&v, "backup_name") {
        Ok(s) => s,
        Err(r) => return r,
    };
    if backup_name.is_empty() || backup_name.len() > 200 {
        return port_err(StatusCode::BAD_REQUEST, "backup_name invalid");
    }
    // ADR-0059: scope a rollback audit context so the row emitted by
    // FsStrategyStore::store carries op:"rollback" + source_backup.
    let audit_ctx = resin_core::audit::AuditCtx {
        op: Some("rollback".into()),
        actor: Some("headless:strategy_rollback".into()),
        source_backup: Some(backup_name.clone()),
        reason: None,
    };
    let rollback = resin_core::audit::AUDIT_CTX
        .scope(audit_ctx, async {
            ctx.strategy
                .store_ref()
                .rollback(&backup_name)
                .map_err(IpcError::from)
        })
        .await;
    if let Err(e) = rollback {
        return port_ipc_err(&e);
    }
    let client = match ctx.client() {
        Ok(c) => c,
        Err(e) => return port_err(StatusCode::BAD_GATEWAY, &e),
    };
    match ctx
        .strategy
        .apply(&client, resin_core::resolve_id_in)
        .await
        .map_err(IpcError::from)
    {
        Ok(report) => match serde_json::to_value(&report) {
            Ok(v) => axum::Json(v).into_response(),
            Err(e) => port_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        },
        Err(e) => port_ipc_err(&e),
    }
}

async fn strategy_verify_h(ctx: Arc<PortCtx>, body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let platform_name = match str_arg(&v, "platform_name") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let sample_count = v.get("sample_count").and_then(|x| x.as_u64()).unwrap_or(10) as u32;
    let client = match ctx.client() {
        Ok(c) => c,
        Err(e) => return port_err(StatusCode::BAD_GATEWAY, &e),
    };
    match commands::strategy_verify_impl(ctx.sidecar.api_port, &client, platform_name, sample_count)
        .await
    {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn strategy_apply_h(ctx: Arc<PortCtx>) -> Response {
    let client = match ctx.client() {
        Ok(c) => c,
        Err(e) => return port_err(StatusCode::BAD_GATEWAY, &e),
    };
    match ctx
        .strategy
        .apply(&client, resin_core::resolve_id_in)
        .await
        .map_err(IpcError::from)
    {
        Ok(report) => match serde_json::to_value(&report) {
            Ok(v) => axum::Json(v).into_response(),
            Err(e) => port_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        },
        Err(e) => port_ipc_err(&e),
    }
}

// ── Authoritative snapshot + reconcile ───────────────────────────────────

async fn snapshot_h(ctx: Arc<PortCtx>) -> Response {
    match commands::authoritative_snapshot_core(
        &ctx.strategy,
        ctx.sidecar.as_ref(),
        &ctx.whitebox,
        &ctx.db,
        &ctx.forwarder,
    )
    .await
    {
        Ok(snap) => axum::Json(snap).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn reconcile_h(ctx: Arc<PortCtx>) -> Response {
    match commands::reconcile_now_impl(
        &ctx.strategy,
        ctx.sidecar.as_ref(),
        &ctx.whitebox,
        &ctx.forwarder,
    )
    .await
    {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

// ── Process routes (ADR-0055 D2: L2 whitebox family) ─────────────────────

async fn process_route_list_h(ctx: Arc<PortCtx>) -> Response {
    axum::Json(ctx.whitebox.snapshot().process_routes).into_response()
}

async fn process_route_add_h(ctx: Arc<PortCtx>, body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let process = match str_arg(&v, "process") {
        Ok(s) => s,
        Err(r) => return r,
    };
    let target_port = v.get("target_port").and_then(|x| x.as_u64()).unwrap_or(0) as u16;
    match commands::process_route_add_impl(
        &ctx.db,
        &ctx.forwarder,
        &ctx.whitebox,
        process,
        target_port,
    )
    .await
    {
        Ok(()) => axum::Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn process_route_remove_h(ctx: Arc<PortCtx>, body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let process = match str_arg(&v, "process") {
        Ok(s) => s,
        Err(r) => return r,
    };
    match commands::process_route_remove_impl(&ctx.db, &ctx.forwarder, &ctx.whitebox, process).await
    {
        Ok(removed) => axum::Json(removed).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

// ── Config transfer + backups ────────────────────────────────────────────

async fn config_export_h(ctx: Arc<PortCtx>) -> Response {
    match commands::config_export_impl(&ctx.strategy, &ctx.whitebox).await {
        Ok(doc) => axum::Json(doc).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn config_import_h(ctx: Arc<PortCtx>, body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let doc = v.get("config").cloned().unwrap_or(v);
    match commands::config_import_impl(
        &ctx.strategy,
        ctx.sidecar.as_ref(),
        &ctx.whitebox,
        &ctx.db,
        &ctx.forwarder,
        doc,
    )
    .await
    {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn backup_create_h(ctx: Arc<PortCtx>, body: Bytes) -> Response {
    let v = match read_json_body(&body) {
        Ok(v) => v,
        Err(_) => serde_json::Value::Null,
    };
    let passphrase = str_arg_opt(&v, "passphrase");
    let cfg = ctx.state_root.clone();
    let out_dir = ctx.state_root.join("backups");
    match commands::backup_create_impl(
        &cfg,
        &ctx.state_root,
        &out_dir,
        &ctx.strategy,
        &ctx.whitebox,
        passphrase,
    )
    .await
    {
        Ok(path) => axum::Json(path).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn backup_list_h(body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let (Ok(url), Ok(username)) = (str_arg(&v, "url"), str_arg(&v, "username")) else {
        return port_err(StatusCode::BAD_REQUEST, "url and username are required");
    };
    let password = str_arg_opt(&v, "password").unwrap_or_default();
    match commands::backup_list(url, username, password).await {
        Ok(list) => axum::Json(list).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn backup_upload_h(ctx: Arc<PortCtx>, body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let (Ok(url), Ok(username), Ok(zip_path)) = (
        str_arg(&v, "url"),
        str_arg(&v, "username"),
        str_arg(&v, "zip_path"),
    ) else {
        return port_err(
            StatusCode::BAD_REQUEST,
            "url, username and zip_path are required",
        );
    };
    let password = str_arg_opt(&v, "password").unwrap_or_default();
    let backups_dir = ctx.state_root.join("backups");
    match commands::backup_upload_impl(&backups_dir, url, username, password, zip_path).await {
        Ok(()) => axum::Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn backup_restore_h(ctx: Arc<PortCtx>, body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let (Ok(url), Ok(username), Ok(zip_name)) = (
        str_arg(&v, "url"),
        str_arg(&v, "username"),
        str_arg(&v, "zip_name"),
    ) else {
        return port_err(
            StatusCode::BAD_REQUEST,
            "url, username and zip_name are required",
        );
    };
    let password = str_arg_opt(&v, "password").unwrap_or_default();
    let passphrase = str_arg_opt(&v, "passphrase");
    let settings_path = ctx.settings_path.clone();
    match commands::backup_restore_impl(
        &ctx.state_root,
        &ctx.strategy,
        ctx.sidecar.as_ref(),
        &ctx.whitebox,
        &ctx.db,
        &ctx.forwarder,
        url,
        username,
        password,
        zip_name,
        passphrase,
        move |obj| {
            let mut doc = read_settings_doc(&settings_path);
            for (k, val) in obj {
                doc.insert(k.clone(), val.clone());
            }
            write_settings_doc(&settings_path, &doc)
        },
    )
    .await
    {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

// ── Sidecar lifecycle + host diagnostics ─────────────────────────────────

async fn sidecar_status_h(ctx: Arc<PortCtx>) -> Response {
    axum::Json(commands::sidecar_status_of(ctx.sidecar.as_ref())).into_response()
}

async fn sidecar_logs_h(ctx: Arc<PortCtx>) -> Response {
    axum::Json(ctx.sidecar.log_buf.snapshot()).into_response()
}

/// close_all_connections + reset_kernel share one transport: a sidecar
/// restart through the same `restart_into_slot` seam the desktop uses. The
/// `reason` field is kept for audit parity with the two command names.
async fn sidecar_restart_h(ctx: Arc<PortCtx>, body: Bytes) -> Response {
    let v = match read_json_body(&body) {
        Ok(v) => v,
        Err(_) => serde_json::Value::Null,
    };
    let reason = str_arg_opt(&v, "reason").unwrap_or_else(|| "restart".to_string());
    if reason != "close_all" && reason != "reset_kernel" && reason != "restart" {
        return port_err(StatusCode::BAD_REQUEST, "unknown restart reason");
    }
    let handle = ctx.sidecar.clone();
    let dirs = ctx.restart.clone();
    let network = ctx.whitebox.snapshot().network;
    let result = tokio::task::spawn_blocking(move || {
        egressapikey_app::sidecar::restart_into_slot(
            &handle,
            dirs.state_dir,
            dirs.cache_dir,
            dirs.log_dir,
            dirs.binary_path,
            network,
        )
    })
    .await;
    match result {
        Ok(Ok(())) => {
            axum::Json(serde_json::json!({ "ok": true, "reason": reason })).into_response()
        }
        Ok(Err(e)) => port_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        Err(e) => port_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("restart join: {e}"),
        ),
    }
}

async fn firewall_h() -> Response {
    match commands::check_firewall_status().await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn probe_exit_ip_h(ctx: Arc<PortCtx>, body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let port = v.get("port").and_then(|x| x.as_u64()).unwrap_or(0) as u16;
    let protocol = str_arg_opt(&v, "protocol").unwrap_or_else(|| "mixed".to_string());
    match commands::probe_exit_ip_impl(&ctx.sidecar.proxy_token, &ctx.db, port, protocol).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

// ── Deprecated echo stubs (ADR-0050 contract compatibility) ──────────────
// Reachable for parity: they validate input and echo the same fixed
// responses the desktop commands produce (no semantics — Resin owns
// accounts since ADR-0050).

async fn account_add_h(body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let platform = str_arg_opt(&v, "platform").unwrap_or_default();
    let id = str_arg_opt(&v, "id").unwrap_or_default();
    let lane = v.get("lane").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
    if let Err(e) = commands::validate_short_name(&platform, "platform")
        .and_then(|_| commands::validate_short_name(&id, "account"))
    {
        return port_err(StatusCode::BAD_REQUEST, &e);
    }
    if lane >= resin_core::MAX_LANES {
        return port_err(
            StatusCode::BAD_REQUEST,
            &format!(
                "lane {lane} out of range (max {})",
                resin_core::MAX_LANES - 1
            ),
        );
    }
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

async fn account_bind_ip_h(body: Bytes) -> Response {
    let v = match json_body(&body).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let platform = str_arg_opt(&v, "platform").unwrap_or_default();
    let account = str_arg_opt(&v, "account").unwrap_or_default();
    let ip = str_arg_opt(&v, "ip").unwrap_or_default();
    if let Err(e) = commands::validate_short_name(&platform, "platform")
        .and_then(|_| commands::validate_short_name(&account, "account"))
        .and_then(|_| commands::validate_ip(&ip))
    {
        return port_err(StatusCode::BAD_REQUEST, &e);
    }
    axum::Json(true).into_response()
}

// ── Capabilities ─────────────────────────────────────────────────────────

async fn capabilities_h() -> Response {
    // Serve the registry verbatim — it is already the machine-readable
    // contract the SPA consumes; no re-serialisation drift.
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        CAPABILITIES_JSON,
    )
        .into_response()
}

/// All BFF-native shell routes, registered under /api/v1/shell/* plus the
/// /api/v1/capabilities read. Merge order note: axum 0.7 matches static
/// segments before the /api/v1/*path wildcard, so these shadow nothing that
/// belongs to the Resin proxy.
pub fn shell_routes(ctx: Arc<PortCtx>) -> Router {
    macro_rules! h {
        ($f:ident) => {{
            let c = ctx.clone();
            move |body: Bytes| $f(c.clone(), body)
        }};
        ($f:ident, no_body) => {{
            let c = ctx.clone();
            move || $f(c.clone())
        }};
    }
    Router::new()
        // capabilities + L1 settings
        .route("/api/v1/capabilities", get(capabilities_h))
        .route(
            "/api/v1/shell/settings",
            get(h!(settings_get_h, no_body)).put(h!(settings_put_h)),
        )
        .route(
            "/api/v1/shell/settings/diag-poll-interval",
            get(h!(diag_poll_get_h, no_body)).put(h!(diag_poll_put_h)),
        )
        .route(
            "/api/v1/shell/log-level",
            get(log_level_get_h).put(log_level_put_h),
        )
        .route(
            "/api/v1/shell/ip-reputation",
            get(h!(ip_reputation_h, no_body)),
        )
        // L2 whitebox
        .route("/api/v1/shell/whitebox", get(h!(whitebox_get_h, no_body)))
        .route(
            "/api/v1/shell/whitebox/reload",
            post(h!(whitebox_reload_h, no_body)),
        )
        .route(
            "/api/v1/shell/whitebox/network",
            patch(h!(whitebox_network_h)),
        )
        .route(
            "/api/v1/shell/whitebox/backups",
            get(h!(whitebox_backups_h, no_body)),
        )
        .route(
            "/api/v1/shell/whitebox/rollback",
            post(h!(whitebox_rollback_h)),
        )
        // L2 strategy
        .route(
            "/api/v1/shell/strategy/config",
            get(h!(strategy_config_get_h, no_body)).put(h!(strategy_config_put_h)),
        )
        .route(
            "/api/v1/shell/strategy/regions",
            patch(h!(strategy_regions_h)),
        )
        .route(
            "/api/v1/shell/strategy/backups",
            get(h!(strategy_backups_h, no_body)),
        )
        .route(
            "/api/v1/shell/strategy/rollback",
            post(h!(strategy_rollback_h)),
        )
        .route("/api/v1/shell/strategy/verify", post(h!(strategy_verify_h)))
        .route(
            "/api/v1/shell/strategy/apply",
            post(h!(strategy_apply_h, no_body)),
        )
        .route("/api/v1/shell/snapshot", get(h!(snapshot_h, no_body)))
        .route("/api/v1/shell/reconcile", post(h!(reconcile_h, no_body)))
        // process routes
        .route(
            "/api/v1/shell/process-routes",
            get(h!(process_route_list_h, no_body))
                .post(h!(process_route_add_h))
                .delete(h!(process_route_remove_h)),
        )
        // config transfer + backups
        .route(
            "/api/v1/shell/config/export",
            get(h!(config_export_h, no_body)),
        )
        .route("/api/v1/shell/config/import", post(h!(config_import_h)))
        .route("/api/v1/shell/backups", post(h!(backup_create_h)))
        .route("/api/v1/shell/backups/list", post(backup_list_h))
        .route("/api/v1/shell/backups/upload", post(h!(backup_upload_h)))
        .route("/api/v1/shell/backups/restore", post(h!(backup_restore_h)))
        // sidecar lifecycle + host diagnostics
        .route(
            "/api/v1/shell/sidecar/status",
            get(h!(sidecar_status_h, no_body)),
        )
        .route(
            "/api/v1/shell/sidecar/logs",
            get(h!(sidecar_logs_h, no_body)),
        )
        .route("/api/v1/shell/sidecar/restart", post(h!(sidecar_restart_h)))
        .route("/api/v1/shell/firewall", get(firewall_h))
        .route("/api/v1/shell/probe-exit-ip", post(h!(probe_exit_ip_h)))
        // deprecated echo stubs (ADR-0050 contract parity)
        .route("/api/v1/shell/accounts", post(account_add_h))
        .route("/api/v1/shell/accounts/bind-ip", post(account_bind_ip_h))
}
