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
//!    browser request).
//! 4. Two-phase shutdown: on Ctrl+C/SIGTERM, kills the resin child and
//!    exits. Logs the full sequence via `tracing` to the OS log dir.
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
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use clap::Parser;
use egressapikey_app::sidecar::boot_resin_standalone;
use tower_http::services::ServeDir;

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

    let app = build_router(&cli.dist, api_base.clone(), admin_token.clone());

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
        let launch_url = format!("http://{}:{}/", cli.bind, cli.port);
        tracing::info!("headless: opening browser at {launch_url}");
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
fn build_router(dist: &PathBuf, api_base: String, admin_token: String) -> Router {
    let serve_dir = ServeDir::new(dist.clone()).append_index_html_on_directories(true);

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
}

/// Pure-proxy to resin: forwards method, headers (with admin bearer injected),
/// and streamed body. Resin returns the response and we stream it back to
/// the browser. SSE/WebSocket streams pass through via `Body::from_stream`.
async fn proxy_to_resin(
    method: Method,
    orig_uri: axum::http::Uri,
    mut headers: HeaderMap,
    body: Body,
    api_base: Arc<String>,
    admin_token: Arc<String>,
) -> Response {

    // Reconstruct the upstream URL: api_base + path + query.
    let path_and_query = orig_uri.path_and_query().map(|p| p.as_str()).unwrap_or("");
    let upstream = if api_base.ends_with('/') {
        format!("{}{}", api_base.trim_end_matches('/'), path_and_query)
    } else {
        format!("{}/{}", api_base, path_and_query)
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
        .body(reqwest::Body::wrap_stream(body.into_data_stream()));
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
