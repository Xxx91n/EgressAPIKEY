//! EgressAPIKEY headless server.
//!
//! Spawned by `npm`'s `@egressapikey/server` launcher for users who want
//! localhost access to the EgressAPIKEY control surface WITHOUT installing
//! the Tauri desktop shell.
//!
//! Responsibilities (thin, no GUI):
//! 1. Spawns the resin Go sidecar (reuses the same sidecar.rs boot helpers
//!    the Tauri shell uses; no GUI-specific logic).
//! 2. Serves the prebuilt `dist/` static assets on the bind address so
//!    the same React frontend the Tauri webview ships runs in any browser.
//! 3. Proxies `/api/v1/*` and `/metrics/*` requests to the local Resin
//!    control plane (admin token stays Rust-side, never leaks to the
//!    browser request). Mutating routes that the SPA addresses by
//!    business `name` (DELETE/PATCH on `/platforms` and `/subscriptions`)
//!    are translated here to Resin's `/{id}` path-param form BEFORE the
//!    upstream call (industrial BFF pattern; see ADR-0043 followup +
//!    docs/GRILL_T17_HEADLESS_SERVER.md audit). This mirrors the
//!    list+match resolution the Tauri Rust IPC commands (platform_remove,
//!    platform_update, subscription_remove) already perform, so the two
//!    modes stay behaviour-equivalent and TOCTOU-free (resolve + mutate in
//!    one process, single serial connection).
//! 4. Two-phase shutdown: on Ctrl+C/SIGTERM, kills the resin child and
//!    exits. Logs the full sequence via `tracing` to the OS log dir.
//!
//! 5. Enforces the ticket-03 (A-007) control-surface security gate: a shared
//!    `--auth-token` on `/api/v1/*` + `/metrics/*`, and a `Host`/`Origin`
//!    allowlist on every request (DNS-rebinding mitigation). The primitives live
//!    in `headless_security`; the operator-facing threat model is
//!    `docs/how-to/HEADLESS_DEPLOYMENT.md`.
//!
//! This binary does NOT:
//! - Load Tauri plugins or the webview
//! - Manage the OS system HTTP proxy (Ghost safety net feature owned by the
//!   desktop GUI's tray; headless mode never touches the OS proxy).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use clap::Parser;
use egressapikey_app::headless_security::{self, HeadlessGuard};
use egressapikey_app::sidecar::boot_resin_standalone;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Parser, Debug)]
#[command(name = "egressapikey-headless", version, about = "EgressAPIKEY headless server (no Tauri webview).")]
struct Cli {
    /// Bind address for the HTTP control surface.
    #[arg(long, default_value = "127.0.0.1")]
    bind: String,
    /// Bind port for the HTTP control surface.
    #[arg(long, default_value = "14200")]
    port: u16,
    /// Path to the prebuilt `dist/` frontend assets.
    #[arg(long, default_value = "dist")]
    dist: PathBuf,
    /// Resin sidecar state dir (e.g. ~/.local/share/egressapikey). The
    /// sidecar writes state.db here. Created if absent.
    #[arg(long)]
    state_root: Option<PathBuf>,
    /// Resin sidecar log dir (e.g. ~/.local/state/egressapikey/logs).
    /// Created if absent.
    #[arg(long)]
    log_root: Option<PathBuf>,
    /// Directory containing the resin-<triple>[.exe] binary.
    #[arg(long)]
    binary_dir: Option<PathBuf>,
    /// Skip the automatic browser-open. Useful for daemons.
    #[arg(long)]
    no_browser: bool,
    /// Shared secret required on every control-plane request (`/api/v1/*`,
    /// `/metrics/*`). When omitted, a token is generated with the OS CSPRNG if
    /// `--bind` is loopback; startup is REFUSED if `--bind` is not loopback.
    #[arg(long)]
    auth_token: Option<String>,
    /// Extra hostname accepted in the `Host` / `Origin` header (repeatable).
    /// Required when the server is reached through a name other than the bind
    /// address, e.g. a TLS reverse proxy in front of `--bind=0.0.0.0`.
    #[arg(long = "allowed-host", value_name = "HOST")]
    allowed_host: Vec<String>,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Resolve state_root / log_root via the `dirs` crate so the headless
    // binary lives in the SAME OS-standard dirs the Tauri shell uses
    // (`app_data_dir` on Windows = %APPDATA%\com.egressapikey.desktop).
    let state_root = cli.state_root.unwrap_or_else(|| {
        dirs::data_dir()
            .map(|d| d.join("egressapikey"))
            .unwrap_or_else(|| PathBuf::from(".egressapikey-state"))
    });
    let log_root = cli.log_root.unwrap_or_else(|| {
        dirs::state_dir()
            .map(|d| d.join("egressapikey").join("logs"))
            .or_else(|| dirs::data_dir().map(|d| d.join("egressapikey").join("logs")))
            .unwrap_or_else(|| PathBuf::from(".egressapikey-logs"))
    });
    std::fs::create_dir_all(&log_root)
        .with_context(|| format!("headless: cannot create log_root {:?}", log_root))?;
    // Init tracing to a daily-rotating file in log_root + stderr.
    let file_appender = tracing_appender::rolling::daily(&log_root, "headless.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    // Keep guard alive in a side thread for the whole process lifetime
    // (otherwise it would drop and flush on the first tick).
    std::mem::forget(_guard);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "egressapikey=info,resin_core=info,egressapikey_app=info".into()),
        )
        .with_writer(std::io::stderr)
        .with_writer(non_blocking)
        .init();
    tracing::info!(
        "headless: starting (bind={}:{}, dist={:?}, state_root={:?})",
        cli.bind, cli.port, cli.dist, state_root
    );

    // Ticket 03 (A-007) startup gate: refuse to expose the admin control plane
    // off-host without a token; otherwise fall back to a CSPRNG token. Runs
    // BEFORE the resin sidecar is spawned so a refusal leaves no orphan child.
    let resolved = headless_security::resolve_token(cli.auth_token.as_deref(), &cli.bind)
        .map_err(|e| {
            tracing::error!("{e}");
            anyhow::anyhow!(e)
        })?;
    let guard = Arc::new(HeadlessGuard::new(
        headless_security::allowed_hosts(&cli.bind, &cli.allowed_host),
        resolved.token.clone(),
    ));
    if resolved.source == headless_security::TokenSource::Generated {
        tracing::info!("headless: generated a CSPRNG --auth-token for this session");
        eprintln!("headless: auth token (printed once): {}", resolved.token);
        eprintln!(
            "headless: open http://{}:{}/?{}={}",
            cli.bind,
            cli.port,
            headless_security::TOKEN_QUERY,
            resolved.token
        );
    } else {
        tracing::info!("headless: enforcing the operator-supplied --auth-token");
    }
    tracing::info!(
        "headless: Host/Origin allowlist = {:?}",
        guard.allowed_hosts()
    );

    let binary_dir = cli.binary_dir.unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("binaries"))
    });

    let sidecar = boot_resin_standalone(state_root.clone(), log_root.clone(), binary_dir)?;
    let sidecar = Arc::new(sidecar);
    let api_base = format!("http://127.0.0.1:{}/", sidecar.api_port);
    let admin_token = sidecar.admin_token.clone();
    tracing::info!(
        "headless: resin control plane up at {} (api_port={})",
        api_base.trim_end_matches('/'),
        sidecar.api_port
    );

    let app = build_router(
        &cli.dist,
        api_base.clone(),
        admin_token.clone(),
        guard.clone(),
    );

    let addr: SocketAddr = format!("{}:{}", cli.bind, cli.port)
        .parse()
        .with_context(|| format!("headless: invalid bind {:?}:{:?}", cli.bind, cli.port))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("headless: cannot bind {:?}:{:?} (already in use?)", cli.bind, cli.port))?;
    let server_task = tokio::spawn(async move {
        tracing::info!("headless: control surface listening on http://{}", addr);
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("headless: axum server error: {e}");
        }
    });

    if !cli.no_browser {
        // The control plane now requires the token, so the launch URL carries it
        // once; the guard plants the session cookie on that first request.
        let launch_url = format!(
            "http://{}:{}/?{}={}",
            cli.bind,
            cli.port,
            headless_security::TOKEN_QUERY,
            resolved.token
        );
        tracing::info!(
            "headless: opening browser at http://{}:{}/ (token elided from logs)",
            cli.bind,
            cli.port
        );
        let _ = open::that(&launch_url);
    }

    // Await Ctrl+C / SIGTERM, then two-phase shutdown.
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        let mut int = signal(SignalKind::interrupt()).expect("install SIGINT handler");
        tokio::select! {
            _ = term.recv() => tracing::info!("headless: received SIGTERM"),
            _ = int.recv() => tracing::info!("headless: received SIGINT (Ctrl+C)"),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.expect("install ctrl_c handler");
        tracing::info!("headless: received Ctrl+C");
    }

    tracing::info!("headless: shutdown requested; killing resin sidecar");
    {
        let mut guard = sidecar.child.lock().expect("child mutex poisoned");
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            let _ = child.wait();
            tracing::info!("headless: resin sidecar killed + reaped");
        }
    }
    server_task.abort();
    tracing::info!("headless: good bye");
    Ok(())
}

/// Build the axum router. Static assets via ServeDir at `/`; `/api/v1/*`
/// and `/metrics/*` proxied to the resin control plane with the admin
/// bearer token injected (the browser request must NOT carry the token).
fn build_router(
    dist: &PathBuf,
    api_base: String,
    admin_token: String,
    guard: Arc<HeadlessGuard>,
) -> Router {
    let serve_dir = ServeDir::new(dist.clone())
        .append_index_html_on_directories(true)
        .not_found_service(ServeFile::new(dist.join("index.html")));

    let api_base = Arc::new(api_base);
    let admin_token = Arc::new(admin_token);
    let proxy_state = Arc::new((api_base, admin_token));
    let proxy_handler = move |method: Method,
                              orig_uri: axum::http::Uri,
                              headers: HeaderMap,
                              body: Body| {
        let proxy_state = proxy_state.clone();
        async move {
            let (api_base, admin_token) = (&proxy_state.0, &proxy_state.1);
            proxy_to_resin(method, orig_uri, headers, body, api_base.clone(), admin_token.clone()).await
        }
    };

    Router::new()
        .route("/api/v1/*path", any(proxy_handler.clone()))
        .route("/metrics/*path", any(proxy_handler))
        .fallback_service(serve_dir)
        // Ticket 03 (A-007): the Host/Origin + token guard wraps every route and
        // the static fallback. Applied last so it also covers the fallback.
        .layer(middleware::from_fn_with_state(guard, security_guard))
}

/// Ticket 03 (A-007) request guard. Two ordered checks:
///
/// 1. Host / Origin allowlist - rejects DNS-rebinding style requests whose
///    `Host` names a domain that merely resolves to this machine. Applies to
///    every request, static assets included.
/// 2. Shared-secret token - required on the control-plane prefixes
///    `/api/v1/*` and `/metrics/*`. A valid token arriving via `?auth_token=`
///    also plants the session cookie, so the SPA's later same-origin fetches
///    authenticate without ever holding the token in JS.
async fn security_guard(
    State(guard): State<Arc<HeadlessGuard>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    if let Err(reason) =
        headless_security::host_allowed(host.as_deref(), origin.as_deref(), guard.allowed_hosts())
    {
        tracing::warn!(target: "headless.security", "rejected: {reason}");
        return (StatusCode::FORBIDDEN, format!("headless: {reason}")).into_response();
    }

    let path = req.uri().path().to_owned();
    let gated = path.starts_with("/api/v1/") || path == "/api/v1" || path.starts_with("/metrics");
    let mut plant_cookie = false;
    if gated {
        let authorization = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let cookie = req
            .headers()
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let query = req.uri().query().map(str::to_owned);
        match guard.extract_token(authorization.as_deref(), cookie.as_deref(), query.as_deref()) {
            Some((candidate, from_query)) if guard.token_matches(candidate) => {
                plant_cookie = from_query;
            }
            _ => {
                tracing::warn!(
                    target: "headless.security",
                    "rejected: missing or invalid auth token on {path}"
                );
                let mut resp = (
                    StatusCode::UNAUTHORIZED,
                    "headless: missing or invalid auth token",
                )
                    .into_response();
                resp.headers_mut().insert(
                    header::WWW_AUTHENTICATE,
                    HeaderValue::from_static("Bearer"),
                );
                return resp;
            }
        }
    }

    let mut resp = next.run(req).await;
    if plant_cookie {
        if let Ok(value) = HeaderValue::from_str(&guard.session_cookie()) {
            resp.headers_mut().insert(header::SET_COOKIE, value);
        }
    }
    resp
}

/// Pure-proxy to resin with a thin BFF translation layer for the three
/// routes the SPA addresses by business `name` instead of Resin's UUID
/// `{id}` path-param:
///   - DELETE /api/v1/platforms      body {name}   -> DELETE /api/v1/platforms/{id}
///   - PATCH  /api/v1/platforms      body {name,...} -> PATCH /api/v1/platforms/{id} (name stripped)
///   - DELETE /api/v1/subscriptions  body {name}   -> DELETE /api/v1/subscriptions/{id}
/// The list+match resolution happens HERE (not in the SPA) so:
///   1) the frontend contract is name-based in BOTH modes (matches Tauri);
///   2) resolve + mutate are atomic in one process (no client-side TOCTOU);
///   3) the browser never needs to hold Resin UUIDs.
/// See docs/GRILL_T17_HEADLESS_SERVER.md (T17-audit-fix) and ADR-0043 for the
/// industry-template lineage (Kong request-transformer / LiteLLM pattern).
async fn proxy_to_resin(
    method: Method,
    orig_uri: axum::http::Uri,
    mut headers: HeaderMap,
    body: Body,
    api_base: Arc<String>,
    admin_token: Arc<String>,
) -> Response {
    // Aggregate the incoming body once so we can read `name` for the
    // translation routes. Non-translating routes re-wrap the same bytes.
    let body_bytes = match axum::body::to_bytes(body, 64 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("headless: read body: {e}"),
            )
                .into_response();
        }
    };

    // Strip inbound auth headers (never trust client-supplied admin), then
    // inject our server-side admin bearer so Resin accepts the admin call.
    headers.remove("authorization");
    headers.remove("x-api-key");
    headers.remove("x-resin-account");
    let bearer = format!("Bearer {}", admin_token);
    headers.insert(
        HeaderName::from_static("authorization"),
        HeaderValue::from_str(&bearer).unwrap_or_else(|_| HeaderValue::from_static("Bearer")),
    );
    headers.remove("host");

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("headless: client build: {e}"),
            )
                .into_response();
        }
    };

    // Reconstruct upstream URL, applying a path rewrite when the translation
    // layer succeeds. On translation failure we still fall through to the
    // original path_and_query so Resin issues its own 404/405 and the SPA
    // gets a useful error (same as the Tauri mode's "platform not found").
    let raw_path = orig_uri.path_and_query().map(|p| p.as_str()).unwrap_or("");
    let upstream_base = if api_base.ends_with('/') {
        api_base.trim_end_matches('/').to_string()
    } else {
        api_base.as_str().to_string()
    };

    let (final_path, final_body) = match translate_request(
        &method,
        raw_path,
        &body_bytes,
        &client,
        &upstream_base,
        &admin_token,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            // Translation lookup failure -> 404 with a Resin-shaped error so
            // the SPA's existing error mapping (map_resin_error) can parse it.
            return (
                StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({
                    "error": { "code": "NOT_FOUND", "message": format!("headless BFF: {e}") }
                })),
            )
                .into_response();
        }
    };
    let upstream = format!("{}/{}", upstream_base, final_path.trim_start_matches('/'));

    let mut req_headers = reqwest::header::HeaderMap::new();
    for (k, v) in headers.iter() {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::from_bytes(k.as_str().as_bytes()),
            reqwest::header::HeaderValue::from_bytes(v.as_bytes()),
        ) {
            req_headers.insert(name, val);
        }
    }
    let req = client
        .request(reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET), &upstream)
        .headers(req_headers)
        .body(final_body);
    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let mut out_headers = HeaderMap::new();
            for (k, v) in resp.headers().iter() {
                out_headers.insert(k.clone(), v.clone());
            }
            let byte_stream = resp.bytes_stream();
            let body = Body::from_stream(byte_stream);
            let mut out = Response::new(body);
            *out.status_mut() = status;
            *out.headers_mut() = out_headers;
            out
        }
        Err(e) => (StatusCode::BAD_GATEWAY, format!("headless: upstream error: {e}")).into_response(),
    }
}

/// Accept Resin's items-wrapper shape `{"items":[...]}` OR a bare array and
/// return the slice of entries. Mirrors commands::mod::items_arr.
fn items_arr(v: &serde_json::Value) -> &[serde_json::Value] {
    match v {
        serde_json::Value::Array(arr) => arr.as_slice(),
        serde_json::Value::Object(obj) => obj
            .get("items")
            .and_then(|i| i.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]),
        _ => &[],
    }
}

/// Look up an entity by business `name` in a list response and return its `id`.
fn id_for_name(list: &serde_json::Value, want: &str) -> Option<String> {
    for p in items_arr(list) {
        let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if name == want {
            let id = p.get("id").and_then(|n| n.as_str()).unwrap_or("");
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// URL-encode a path segment (RFC 3986 unreserved + tolerate UUID dashes).
/// No new crates — stdlib + a tiny escape table.
fn url_encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// Pure: rewrite a PATCH body by stripping `name` and translating the
/// camelCase IPC keys (ipcPlatformUpdate sends `allocationPolicy`,
/// `regexFilters`, `regionFilters`, `stickyTtl`, etc.) to Resin's
/// snake_case PATCH schema. Drops null fields (the Tauri command's
/// "only provided fields" contract) and validates enum / length bounds as
/// defence in depth (the SPA TS wrapper already validates at the TS boundary
/// per AGENTS s7.5). Pure function — no reqwest, no I/O; unit-testable.
fn rewrite_patch_body_snake_case(body: &serde_json::Value) -> Result<serde_json::Value, String> {
    let mut body_obj = body
        .as_object()
        .ok_or_else(|| "PATCH body must be a JSON object".to_string())?
        .clone();
    let _ = body_obj.remove("name");
    let mut snake = serde_json::Map::new();
    for (k, v) in body_obj.iter() {
        if v.is_null() {
            continue;
        }
        let mapped = match k.as_str() {
            "allocationPolicy" => "allocation_policy",
            "regexFilters" => "regex_filters",
            "regionFilters" => "region_filters",
            "stickyTtl" => "sticky_ttl",
            other => other,
        };
        if mapped == "allocation_policy" {
            if let Some(s) = v.as_str() {
                if !matches!(s, "BALANCED" | "PREFER_LOW_LATENCY" | "PREFER_IDLE_IP") {
                    return Err(format!(
                        "allocation_policy must be BALANCED/PREFER_LOW_LATENCY/PREFER_IDLE_IP, got {s}"
                    ));
                }
            }
        } else if mapped == "regex_filters" {
            if let Some(arr) = v.as_array() {
                if arr.len() > 64 {
                    return Err("regex_filters: too many entries (max 64)".to_string());
                }
                for f in arr.iter() {
                    if let Some(s) = f.as_str() {
                        if s.len() > 253 || s.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
                            return Err("regex_filter invalid (max 253, no control)".to_string());
                        }
                    }
                }
            }
        } else if mapped == "region_filters" {
            if let Some(arr) = v.as_array() {
                if arr.len() > 64 {
                    return Err("region_filters: too many entries (max 64)".to_string());
                }
                for r in arr.iter() {
                    if let Some(s) = r.as_str() {
                        if s.len() > 16
                            || s.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f || b == b' ')
                        {
                            return Err("region_filter invalid (max 16, no control/space)".to_string());
                        }
                    }
                }
            }
        } else if mapped == "sticky_ttl" {
            if let Some(s) = v.as_str() {
                if s.len() > 32 || s.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
                    return Err("sticky_ttl invalid (max 32, no control)".to_string());
                }
            }
        }
        snake.insert(mapped.to_string(), v.clone());
    }
    if snake.is_empty() {
        return Err("platform_update: no fields to update".to_string());
    }
    Ok(serde_json::Value::Object(snake))
}

/// BFF translation: inspect (method, path, body). When the request targets a
/// name-to-id translation route, resolve via a list GET to Resin (admin bearer
/// injected by the caller's headers param) and return the rewritten path +
/// the post-translation body bytes for the upstream call. Non-translation
/// routes pass through unchanged (path, body_bytes as-is).
///
/// Keep this sync-friendly and short — Ponytail: no new abstraction, just a
/// match on the three known routes.
async fn translate_request(
    method: &Method,
    raw_path: &str,
    body_bytes: &bytes::Bytes,
    client: &reqwest::Client,
    upstream_base: &str,
    admin_token: &str,
) -> Result<(String, reqwest::Body), String> {
    let path = raw_path.split('?').next().unwrap_or(raw_path);
    let is_platforms = path == "/api/v1/platforms";
    let is_subscriptions = path == "/api/v1/subscriptions";
    let needs_translate = (method == Method::DELETE || method == Method::PATCH) && is_platforms
        || (method == Method::DELETE && is_subscriptions);
    if !needs_translate {
        return Ok((raw_path.to_string(), reqwest::Body::from(body_bytes.clone())));
    }
    // Parse body for the business `name`.
    let body_val: serde_json::Value = serde_json::from_slice(body_bytes)
        .map_err(|e| format!("invalid JSON body: {e}"))?;
    let name = body_val
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| "body missing \"name\" field".to_string())?
        .to_string();
    if name.is_empty() || name.len() > 128 || name.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
        return Err("name field invalid (1..128 chars, no control)".to_string());
    }
    // List + match exactly as the Tauri Rust IPC commands do.
    let list_path = if is_platforms { "/api/v1/platforms" } else { "/api/v1/subscriptions" };
    let list_url = format!("{}{}", upstream_base, list_path);
    let list_resp = client
        .get(&list_url)
        .header("authorization", format!("Bearer {}", admin_token))
        .send()
        .await
        .map_err(|e| format!("resin list error: {e}"))?;
    if !list_resp.status().is_success() {
        return Err(format!("resin list returned {}", list_resp.status()));
    }
    let list_val: serde_json::Value = list_resp
        .json()
        .await
        .map_err(|e| format!("resin list parse: {e}"))?;
    let id = id_for_name(&list_val, &name)
        .ok_or_else(|| format!("entity not found by name: {}", name))?;
    let enc = url_encode_segment(&id);

    if method == Method::DELETE {
        // Resin expects no body on DELETE /{id}.
        return Ok((
            format!("/api/v1/{}/{}", if is_platforms { "platforms" } else { "subscriptions" }, enc),
            reqwest::Body::from(Vec::<u8>::new()),
        ));
    }
    // Reuse the pure snake_case rewriter so unit tests can lock the contract
    // without spinning up reqwest.
    let new_body_val = rewrite_patch_body_snake_case(&body_val)?;
    let new_body = serde_json::to_vec(&new_body_val)
        .map_err(|e| format!("re-serialize body: {e}"))?;
    Ok((format!("/api/v1/platforms/{}", enc), reqwest::Body::from(new_body)))
}

#[cfg(test)]
mod bff_translate_tests {
    use super::*;

    #[test]
    fn id_for_name_resolves_from_items_wrapper() {
        let v = serde_json::json!({"items":[{"name":"openai","id":"abc-123"},{"name":"anthropic","id":"def-456"}]});
        assert_eq!(id_for_name(&v, "openai").as_deref(), Some("abc-123"));
        assert_eq!(id_for_name(&v, "anthropic").as_deref(), Some("def-456"));
        assert!(id_for_name(&v, "missing").is_none());
    }

    #[test]
    fn id_for_name_resolves_from_bare_array() {
        let v = serde_json::json!([{"name":"openai","id":"abc-123"}]);
        assert_eq!(id_for_name(&v, "openai").as_deref(), Some("abc-123"));
        assert!(id_for_name(&v, "absent").is_none());
    }

    #[test]
    fn id_for_name_rejects_empty_id() {
        let v = serde_json::json!({"items":[{"name":"openai","id":""}]});
        assert!(id_for_name(&v, "openai").is_none());
    }

    #[test]
    fn url_encode_segment_preserves_alphanum_and_dashes() {
        assert_eq!(url_encode_segment("abc-123_xyz"), "abc-123_xyz");
    }

    #[test]
    fn url_encode_segment_escapes_chinese_and_special() {
        let out = url_encode_segment("名前");
        assert!(!out.contains('名'));
        assert!(out.starts_with('%'));
        assert_eq!(out.matches('%').count(), 6);
    }

    #[test]
    fn rewrite_strips_name_and_camelcase_keys() {
        let body = serde_json::json!({
            "name": "openai",
            "allocationPolicy": "BALANCED",
            "regexFilters": ["api.openai.com"],
            "regionFilters": ["hk"],
            "stickyTtl": "30s",
            "passive_circuit_breaker_disabled": false,
            "extra_prop": 42
        });
        let out = rewrite_patch_body_snake_case(&body).unwrap();
        let obj = out.as_object().unwrap();
        assert!(obj.get("name").is_none(), "name was stripped");
        assert_eq!(obj.get("allocation_policy").and_then(|v| v.as_str()), Some("BALANCED"));
        assert!(obj.get("allocationPolicy").is_none());
        assert!(obj.get("regex_filters").is_some());
        assert!(obj.get("regexFilters").is_none());
        assert!(obj.get("region_filters").is_some());
        assert!(obj.get("regionFilters").is_none());
        assert!(obj.get("sticky_ttl").is_some());
        assert!(obj.get("stickyTtl").is_none());
        assert!(obj.get("passive_circuit_breaker_disabled").is_some());
        assert!(obj.get("extra_prop").is_some());
    }

    #[test]
    fn rewrite_drops_null_fields() {
        let body = serde_json::json!({
            "name": "openai",
            "allocationPolicy": "BALANCED",
            "regexFilters": null,
            "regionFilters": null
        });
        let out = rewrite_patch_body_snake_case(&body).unwrap();
        let obj = out.as_object().unwrap();
        assert!(obj.get("name").is_none());
        assert_eq!(obj.len(), 1, "only the non-null allocation_policy survives");
        assert!(obj.get("regex_filters").is_none());
        assert!(obj.get("region_filters").is_none());
    }

    #[test]
    fn rewrite_rejects_invalid_allocation_policy() {
        let body = serde_json::json!({"name":"x","allocationPolicy":"weird"});
        let err = rewrite_patch_body_snake_case(&body).unwrap_err();
        assert!(err.contains("allocation_policy must be"));
    }

    #[test]
    fn rewrite_rejects_too_many_regex_filters() {
        let body = serde_json::json!({
            "name":"x","allocationPolicy":"BALANCED",
            "regexFilters": (0..65).map(|i| format!("rule-{i}")).collect::<Vec<_>>()
        });
        let err = rewrite_patch_body_snake_case(&body).unwrap_err();
        assert!(err.contains("regex_filters"));
    }

    #[test]
    fn rewrite_rejects_too_many_region_filters() {
        let body = serde_json::json!({
            "name":"x","allocationPolicy":"BALANCED",
            "regionFilters": (0..65).map(|i| format!("r{i}")).collect::<Vec<_>>()
        });
        let err = rewrite_patch_body_snake_case(&body).unwrap_err();
        assert!(err.contains("region_filters"));
    }

    #[test]
    fn rewrite_rejects_empty_body_after_strip() {
        let body = serde_json::json!({"name":"openai"});
        let err = rewrite_patch_body_snake_case(&body).unwrap_err();
        assert!(err.contains("no fields to update"));
    }

    #[test]
    fn rewrite_rejects_non_object_body() {
        let body = serde_json::json!(42);
        let err = rewrite_patch_body_snake_case(&body).unwrap_err();
        assert!(err.contains("must be a JSON object"));
    }
}
