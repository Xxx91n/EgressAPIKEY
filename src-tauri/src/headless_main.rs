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
//!    upstream call (industrial BFF pattern; see ADR-0043). This mirrors the
//!    list+match resolution the Tauri Rust IPC commands (platform_remove,
//!    platform_update, subscription_remove) already perform, so the two
//!    modes stay behaviour-equivalent and TOCTOU-free (resolve + mutate in
//!    one process, single serial connection).
//! 4. Two-phase shutdown: on Ctrl+C/SIGTERM, kills the resin child and
//!    exits. Logs the full sequence via `tracing` to the OS log dir.
//!
//! 5. Enforces the control-surface security gate: a shared
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
    body::{Body, Bytes},
    extract::{ConnectInfo, Path, Query, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, patch, put},
    Router,
};
use clap::Parser;
use egressapikey_app::headless_security::{self, HeadlessGuard};
use egressapikey_app::sidecar::{boot_resin_standalone, SidecarHandle};
use resin_core::encoding::encode_path_segment;

mod headless_shell;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Parser, Debug)]
#[command(
    name = "egressapikey-headless",
    version,
    about = "EgressAPIKEY headless server (no Tauri webview)."
)]
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
    /// Reverse-proxy peer trusted to assert X-Forwarded-Proto (repeatable;
    /// exact IP or CIDR, e.g. 127.0.0.1 or 10.8.0.0/16). Without this flag
    /// XFP is ignored entirely - it is a client-writable header (R12-D1).
    /// Wildcard CIDRs (0.0.0.0/0, ::/0) refuse startup unless the second
    /// explicit `--trusted-proxy-unrestricted` switch is also given
    /// (r12-wave-i D-002); see docs/how-to/HEADLESS_DEPLOYMENT.md.
    #[arg(long = "trusted-proxy", value_name = "IP_OR_CIDR", value_parser = parse_trusted_proxy)]
    trusted_proxy: Vec<ipnet::IpNet>,
    /// Explicit acknowledgement that `--trusted-proxy` may carry a wildcard
    /// CIDR (0.0.0.0/0 or ::/0): every reachable peer is then trusted to
    /// assert X-Forwarded-Proto, which disables the boundary the flag
    /// exists to keep (r12-wave-i D-002). Only meaningful on networks where
    /// every possible client hop is already inside the trusted set.
    #[arg(long = "trusted-proxy-unrestricted")]
    trusted_proxy_unrestricted: bool,
}

/// clap value parser for --trusted-proxy: an exact IP becomes a host net
/// (/32 or /128); a CIDR is taken verbatim.
fn parse_trusted_proxy(s: &str) -> Result<ipnet::IpNet, String> {
    if let Ok(net) = s.parse::<ipnet::IpNet>() {
        return Ok(net);
    }
    s.parse::<std::net::IpAddr>()
        .map(ipnet::IpNet::from)
        .map_err(|_| format!("not an IP or CIDR: {s}"))
}

/// True when `net` matches every peer address (`0.0.0.0/0`, `::/0`, or any
/// /0-prefix CIDR - host bits do not narrow a zero-length prefix). A
/// wildcard --trusted-proxy entry accepts XFP assertions from the whole
/// reachable network, silently disabling the R12-D1 boundary (r12-wave-i
/// D-002).
fn is_wildcard_proxy_net(net: &ipnet::IpNet) -> bool {
    net.prefix_len() == 0
}

/// Startup gate for the wildcard case above: returns the refusal reason
/// when a wildcard --trusted-proxy net is configured WITHOUT the explicit
/// `--trusted-proxy-unrestricted` acknowledgement flag (Prometheus
/// --web.enable-* style: two deliberate switches for a boundary-disabling
/// posture).
fn trusted_proxy_scope_error(nets: &[ipnet::IpNet], unrestricted: bool) -> Option<String> {
    if unrestricted || !nets.iter().any(is_wildcard_proxy_net) {
        return None;
    }
    Some(
        "--trusted-proxy 0.0.0.0/0 or ::/0 would trust X-Forwarded-Proto from every reachable peer; refusing to start (pass --trusted-proxy-unrestricted to accept that posture)"
            .to_string(),
    )
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    // r12-wave-i D-002: refuse a wildcard --trusted-proxy unless the second
    // explicit switch acknowledged it (before any dir/tracing/sidecar work).
    if let Some(e) = trusted_proxy_scope_error(&cli.trusted_proxy, cli.trusted_proxy_unrestricted) {
        anyhow::bail!("{e}");
    }

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
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "egressapikey=info,resin_core=info,egressapikey_app=info".into()
            }),
        )
        .with_writer(std::io::stderr)
        .with_writer(non_blocking)
        .init();
    tracing::info!(
        "headless: starting (bind={}:{}, dist={:?}, state_root={:?})",
        cli.bind,
        cli.port,
        cli.dist,
        state_root
    );

    // startup gate: refuse to expose the admin control plane
    // off-host without a token; otherwise fall back to a CSPRNG token. Runs
    // BEFORE the resin sidecar is spawned so a refusal leaves no orphan child.
    let resolved =
        headless_security::resolve_token(cli.auth_token.as_deref(), &cli.bind).map_err(|e| {
            tracing::error!("{e}");
            anyhow::anyhow!(e)
        })?;
    let guard = Arc::new(HeadlessGuard::new(
        headless_security::allowed_hosts(&cli.bind, &cli.allowed_host),
        resolved.token.clone(),
        cli.trusted_proxy.clone(),
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
    tracing::info!(
        "headless: trusted-proxy peers = {} (--trusted-proxy)",
        cli.trusted_proxy.len()
    );
    if cli.trusted_proxy.iter().any(is_wildcard_proxy_net) {
        tracing::warn!(
            "headless: --trusted-proxy includes a wildcard CIDR - X-Forwarded-Proto is trusted from EVERY reachable peer (--trusted-proxy-unrestricted was given)"
        );
    }

    let binary_dir = cli.binary_dir.unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("binaries"))
    });

    // Resolved once here so both boot and the restart seam reuse
    // the exact same binary path the running child was spawned from.
    let resin_binary = egressapikey_app::sidecar::resolve_resin_binary(Some(&binary_dir))?;
    // boot_resin_standalone -> spawn_resin_inner drives a reqwest::blocking
    // healthz loop; reqwest::blocking REFUSES to run on any thread that
    // carries a tokio runtime context (spawn_blocking threads included —
    // "Cannot drop a runtime in a context where blocking is not allowed").
    // The desktop path is legal because Tauri runs sync commands on plain
    // threads. Boot on a bare std::thread and join: startup is synchronous
    // anyway, so this changes no scheduling semantics.
    let sidecar = {
        let (sr, lr, bd) = (state_root.clone(), log_root.clone(), binary_dir.clone());
        std::thread::spawn(move || boot_resin_standalone(sr, lr, bd))
            .join()
            .map_err(|_| anyhow::anyhow!("headless: resin boot thread panicked"))??
    };
    let sidecar = Arc::new(sidecar);
    let api_base = format!("http://127.0.0.1:{}/", sidecar.api_port);
    let admin_token = sidecar.admin_token.clone();
    tracing::info!(
        "headless: resin control plane up at {} (api_port={})",
        api_base.trim_end_matches('/'),
        sidecar.api_port
    );

    // (option C, user-ruled 2026-09-14): initialise the SAME L2 stores
    // the desktop shell owns (WhiteboxConfigStore + DbPool) at this process state
    // root, so the port family has reachable HTTP semantics instead of being
    // desktop-only. Stores, write entry and validation are shared with the shell
    // through resin-core / commands::ports - only the storage ROOT differs, and a
    // given host runs either the shell or the headless server, never both.
    std::fs::create_dir_all(&state_root)
        .with_context(|| format!("headless: cannot create state_root {:?}", state_root))?;
    // Same audit sink as the GUI shell (main.rs) rooted at state_root: the
    // orchestration driver's signal-plane bookkeeping appends through the
    // global sink and is a silent no-op until this init lands - on headless
    // the environment-suspect evidence row was being dropped entirely.
    resin_core::audit::init(state_root.join(resin_core::audit::AUDIT_LOG_FILE));
    let port_db = resin_core::DbPool::open(&state_root.join("egressapikey.db"))
        .map_err(|e| anyhow::anyhow!("headless: open egressapikey.db: {e}"))?;
    let initial_ports = port_db
        .list_ports()
        .map_err(|e| anyhow::anyhow!("headless: seed whitebox from db: {e}"))?;
    let port_whitebox = resin_core::WhiteboxConfigStore::open(
        state_root.join(resin_core::WHITEBOX_CONFIG_FILE),
        resin_core::WhiteboxConfig::from_ports(initial_ports),
    )
    .await
    .map_err(|e| anyhow::anyhow!("headless: open whitebox store: {e}"))?;
    // Mode B: Resin listens natively, so this forwarder binds no
    // listener; it carries the sidecar proxy token and the running-port view the
    // shared write path expects.
    let port_forwarder = resin_core::PortForwarder::new(
        port_db.clone(),
        "127.0.0.1",
        sidecar.api_port,
        sidecar.proxy_token.clone(),
    );
    // The strategy L2 service roots at state_root here — the desktop
    // shell roots the identical store at app_config_dir (option C).
    let strategy_svc = resin_core::StrategyService::new(resin_core::FsStrategyStore::new(
        state_root.join("egressapikey-strategy.json"),
    ));
    let port_ctx = Arc::new(PortCtx {
        db: port_db,
        whitebox: port_whitebox,
        forwarder: port_forwarder,
        api_base: api_base.clone(),
        admin_token: admin_token.clone(),
        sidecar: sidecar.clone(),
        strategy: strategy_svc,
        settings_path: state_root.join("settings.json"),
        state_root: state_root.clone(),
        restart: headless_shell::RestartDirs {
            state_dir: state_root.join("resin-state"),
            cache_dir: state_root.join("resin-cache"),
            log_dir: log_root.join("resin"),
            binary_path: resin_binary,
        },
    });
    tracing::info!(
        "headless: L2 whitebox + egressapikey.db opened at {:?} ({} port row(s))",
        state_root,
        port_ctx.db.list_ports().map(|r| r.len()).unwrap_or(0)
    );

    let app = build_router(
        &cli.dist,
        api_base.clone(),
        admin_token.clone(),
        guard.clone(),
        port_ctx.clone(),
    );

    let addr: SocketAddr = format!("{}:{}", cli.bind, cli.port)
        .parse()
        .with_context(|| format!("headless: invalid bind {:?}:{:?}", cli.bind, cli.port))?;
    let listener = tokio::net::TcpListener::bind(addr).await.with_context(|| {
        format!(
            "headless: cannot bind {:?}:{:?} (already in use?)",
            cli.bind, cli.port
        )
    })?;
    let server_task = tokio::spawn(async move {
        tracing::info!("headless: control surface listening on http://{}", addr);
        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        {
            tracing::error!("headless: axum server error: {e}");
        }
    });

    // Orchestration driver: 60s tick cadence; inert unless the strategy
    // whitebox's orchestration section is enabled. Headless is the
    // unattended transport, so its resolved default tier is auto.
    let orch_ctx = port_ctx.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let Ok(client) = orch_ctx.client() else {
                continue;
            };
            if let Err(e) = egressapikey_app::commands::orchestration_tick_impl(
                &orch_ctx.strategy,
                &client,
                &orch_ctx.db,
                resin_core::orchestration::Autonomy::Auto,
                &orch_ctx.sidecar.proxy_token,
            )
            .await
            {
                tracing::warn!(error = %e, "orchestration tick failed");
            }
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
        tokio::signal::ctrl_c()
            .await
            .expect("install ctrl_c handler");
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
    port_ctx: Arc<PortCtx>,
) -> Router {
    let serve_dir = ServeDir::new(dist.clone())
        .append_index_html_on_directories(true)
        .not_found_service(ServeFile::new(dist.join("index.html")));

    let api_base = Arc::new(api_base);
    let admin_token = Arc::new(admin_token);
    let proxy_state = Arc::new((api_base, admin_token));
    let proxy_handler =
        move |method: Method, orig_uri: axum::http::Uri, headers: HeaderMap, body: Body| {
            let proxy_state = proxy_state.clone();
            async move {
                let (api_base, admin_token) = (&proxy_state.0, &proxy_state.1);
                proxy_to_resin(
                    method,
                    orig_uri,
                    headers,
                    body,
                    api_base.clone(),
                    admin_token.clone(),
                )
                .await
            }
        };

    // (option C): headless owns the same L2 stores the desktop shell
    // owns, so port management is not desktop-only. These are
    // BFF-native routes: Resin has no /ports resource (its listener face is
    // /api/v1/endpoints, driven here as a side effect exactly as
    // commands/ports.rs does).
    let r_list = {
        let c = port_ctx.clone();
        move || {
            let c = c.clone();
            async move { ports_list_h(c).await }
        }
    };
    let r_suggest = {
        let c = port_ctx.clone();
        move || {
            let c = c.clone();
            async move { ports_suggest_h(c).await }
        }
    };
    let r_running = {
        let c = port_ctx.clone();
        move || {
            let c = c.clone();
            async move { ports_running_h(c).await }
        }
    };
    let r_upsert = {
        let c = port_ctx.clone();
        move |Path(port): Path<u16>, body: Bytes| {
            let c = c.clone();
            async move { ports_upsert_h(c, port, body).await }
        }
    };
    let r_remove = {
        let c = port_ctx.clone();
        move |Path(port): Path<u16>| {
            let c = c.clone();
            async move { ports_remove_h(c, port).await }
        }
    };
    let r_toggle = {
        let c = port_ctx.clone();
        move |Path(port): Path<u16>, body: Bytes| {
            let c = c.clone();
            async move { ports_toggle_h(c, port, body).await }
        }
    };
    let r_bind = {
        let c = port_ctx.clone();
        move |Path(port): Path<u16>, body: Bytes| {
            let c = c.clone();
            async move { ports_bind_platform_h(c, port, body).await }
        }
    };
    let r_auth = {
        let c = port_ctx.clone();
        move |Path(port): Path<u16>| {
            let c = c.clone();
            async move { ports_auth_info_h(c, port).await }
        }
    };
    let r_health = {
        let c = port_ctx.clone();
        move |Path(port): Path<u16>, Query(q): Query<std::collections::HashMap<String, String>>| {
            let c = c.clone();
            let protocol = q.get("protocol").cloned();
            async move { ports_health_h(c, port, protocol).await }
        }
    };

    Router::new()
        .route("/api/v1/ports", get(r_list))
        .route("/api/v1/ports/suggest", get(r_suggest))
        .route("/api/v1/ports/running", get(r_running))
        // axum panics on two .route() calls for one path, so the three verbs on
        // /api/v1/ports/:port are combined into one MethodRouter. (axum 0.7
        // param syntax is ":name" — "{name}" is axum 0.8 and would be a
        // LITERAL segment here, silently unreachable.)
        .route(
            "/api/v1/ports/:port",
            put(r_upsert).delete(r_remove).patch(r_toggle),
        )
        .route("/api/v1/ports/:port/platform", patch(r_bind))
        .route("/api/v1/ports/:port/auth", get(r_auth))
        .route("/api/v1/ports/:port/health", get(r_health))
        // BFF-native shell routes (/api/v1/shell/* + /api/v1/capabilities).
        // Merged before the wildcard; axum 0.7 prefers static segments so the
        // Resin proxy keeps owning every unmatched /api/v1/* path.
        .merge(headless_shell::shell_routes(port_ctx.clone()))
        .route("/api/v1/*path", any(proxy_handler.clone()))
        .route("/metrics/*path", any(proxy_handler))
        .fallback_service(serve_dir)
        // The Host/Origin + token guard wraps every route and
        // the static fallback. Applied last so it also covers the fallback.
        .layer(middleware::from_fn_with_state(guard, security_guard))
}

/// request guard. Two ordered checks:
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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
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
        return with_security_headers(
            (StatusCode::FORBIDDEN, format!("headless: {reason}")).into_response(),
        );
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
        match guard.extract_token(
            authorization.as_deref(),
            cookie.as_deref(),
            query.as_deref(),
        ) {
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
                resp.headers_mut()
                    .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
                return with_security_headers(resp);
            }
        }
    }

    // R12-D1 (ADR-0071 errata): the XFP read lives next to the cookie plant
    // that consumes it - next.run consumes the request, so the header values
    // are captured here and the trust decision (socket peer vs the
    // --trusted-proxy set) is evaluated at plant time. r12-wave-i D-002:
    // EVERY header instance is captured in wire order (get_all, not get - a
    // sender may split the list across headers); the merged rightmost
    // segment is what trusted_forwarded_https consults.
    let x_forwarded_proto: Vec<String> = req
        .headers()
        .get_all("x-forwarded-proto")
        .iter()
        .filter_map(|v| v.to_str().ok().map(str::to_owned))
        .collect();
    let mut resp = next.run(req).await;
    if plant_cookie {
        let mut cookie = guard.session_cookie();
        if headless_security::trusted_forwarded_https(
            peer.ip(),
            guard.trusted_proxies(),
            x_forwarded_proto.iter().map(String::as_str),
        ) {
            cookie.push_str("; Secure");
        }
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            resp.headers_mut().insert(header::SET_COOKIE, value);
        }
    }
    with_security_headers(resp)
}

/// ADR-0071 D3: baseline response headers for every headless response —
/// self-only CSP (no framing anywhere), referrer + nosniff. Applied inside
/// the guard so static assets, BFF routes and proxy responses all carry it.
fn with_security_headers(mut resp: Response) -> Response {
    resp.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
             img-src 'self' data: blob:; connect-src 'self'; font-src 'self' data:; \
             frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    );
    resp.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    resp
}

/// Shared upstream client for every proxy hop. Building a reqwest::Client
/// per request creates a fresh connection pool per call, losing keep-alive
/// reuse to the Resin control plane. once_cell::OnceCell keeps the same
/// build-error surface: a failed init stores nothing and the next call
/// retries, exactly like the old per-request build.
fn proxy_client() -> Result<&'static reqwest::Client, reqwest::Error> {
    static CLIENT: once_cell::sync::OnceCell<reqwest::Client> = once_cell::sync::OnceCell::new();
    CLIENT.get_or_try_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
    })
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
/// See ADR-0043 for the
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
            return (StatusCode::BAD_REQUEST, format!("headless: read body: {e}")).into_response();
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

    let client = match proxy_client() {
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
        client,
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
        .request(
            reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET),
            &upstream,
        )
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
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            format!("headless: upstream error: {e}"),
        )
            .into_response(),
    }
}

/// Look up an entity by business `name` in a list response and return its `id`.
/// List-shape tolerance comes from the shared canonical
/// `egressapikey_app::commands::items_arr`.
fn id_for_name(list: &serde_json::Value, want: &str) -> Option<String> {
    for p in egressapikey_app::commands::items_arr(list) {
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
                // Reuse the SAME allow-list the Tauri command
                // validates against (commands/platform.rs ALLOWED_ALLOCATION_POLICIES)
                // so the enum has ONE definition for both transports.
                if !egressapikey_app::commands::ALLOWED_ALLOCATION_POLICIES.contains(&s) {
                    return Err(format!(
                        "allocation_policy must be {}, got {s}",
                        egressapikey_app::commands::ALLOWED_ALLOCATION_POLICIES.join("/")
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
                            || s.bytes()
                                .any(|b| b == 0 || b < 0x20 || b == 0x7f || b == b' ')
                        {
                            return Err(
                                "region_filter invalid (max 16, no control/space)".to_string()
                            );
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

/// Split a raw path?query into its halves (query without the leading ?).
fn split_query(raw: &str) -> (&str, &str) {
    match raw.split_once('?') {
        Some((p, q)) => (p, q),
        None => (raw, ""),
    }
}

/// Percent-decode one query component. Minimal by design, no new crate:
/// every value the SPA puts in a translated query is an identifier, so the
/// plus-as-space rule is deliberately not applied.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hi = (b[i + 1] as char).to_digit(16);
            let lo = (b[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Read one query parameter from a raw query string, percent-decoding the
/// key and value. Distinct from headless_security::query_param, which
/// deliberately does NOT decode (the bootstrap token is hex and arrives
/// verbatim, so decoding there would corrupt nothing but imply a contract
/// the guard does not have).
fn query_param_decoded(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode(k) == key {
            return Some(percent_decode(v));
        }
    }
    None
}

/// Parse a request body as JSON (every translation route needs an object).
fn parse_body(body_bytes: &bytes::Bytes) -> Result<serde_json::Value, String> {
    serde_json::from_slice(body_bytes).map_err(|e| format!("invalid JSON body: {e}"))
}

/// Read + validate the business name shared by every name-keyed route.
/// Reuses the SAME validator the Tauri commands use, so
/// the bound is defined once and effective on both transports.
fn read_name(body_val: &serde_json::Value) -> Result<String, String> {
    let name = body_val
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| r#"body missing "name" field"#.to_string())?
        .to_string();
    egressapikey_app::commands::validate_short_name(&name, "name")?;
    Ok(name)
}

/// List a collection and resolve a business name to a Resin UUID. Resolve +
/// mutate stay in ONE process, so there is no client-side TOCTOU and the
/// browser never has to hold a UUID.
async fn resolve_id(
    client: &reqwest::Client,
    upstream_base: &str,
    admin_token: &str,
    collection: &str,
    name: &str,
) -> Result<String, String> {
    let list_url = format!("{}/api/v1/{}", upstream_base, collection);
    let resp = client
        .get(&list_url)
        .header("authorization", format!("Bearer {}", admin_token))
        .send()
        .await
        .map_err(|e| format!("resin list error: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("resin list returned {}", resp.status()));
    }
    let val: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("resin list parse: {e}"))?;
    id_for_name(&val, name).ok_or_else(|| format!("entity not found by name: {}", name))
}

/// BFF translation. Four rewrite families, all serving the name-based SPA
/// contract (the browser never holds a Resin UUID):
///
/// 1. Collection + body name -> /{id} path param (DELETE/PATCH platforms,
///    DELETE subscriptions).
/// 2. GET /platforms?leases_for=<name> -> /platforms/{id}/leases:
///    platform_leases was mapped at the bare collection, so headless handed
///    the SPA the platform LIST where it expected the lease set.
/// 3. POST /subscriptions?refresh=<name> -> /subscriptions/{id}/actions/refresh.
///    Refresh had no bare-collection mapping, so it was desktop-only.
/// 4. PUT/DELETE /account-header-rules with body url_prefix ->
///    /account-header-rules/{prefix}. Rules are addressed by prefix in the
///    PATH and a prefix may contain "/" (its %2F must survive), so the SPA never
///    builds that segment itself.
///
/// Anything else passes through unchanged (path + body bytes as-is).
async fn translate_request(
    method: &Method,
    raw_path: &str,
    body_bytes: &bytes::Bytes,
    client: &reqwest::Client,
    upstream_base: &str,
    admin_token: &str,
) -> Result<(String, reqwest::Body), String> {
    let (path, query) = split_query(raw_path);
    let is_platforms = path == "/api/v1/platforms";
    let is_subscriptions = path == "/api/v1/subscriptions";
    let is_rules = path == "/api/v1/account-header-rules";

    // --- 1. Collection + body name -> /{id} ---
    if ((method == Method::DELETE || method == Method::PATCH) && is_platforms)
        || (method == Method::DELETE && is_subscriptions)
    {
        let body_val = parse_body(body_bytes)?;
        let name = read_name(&body_val)?;
        let collection = if is_platforms {
            "platforms"
        } else {
            "subscriptions"
        };
        let id = resolve_id(client, upstream_base, admin_token, collection, &name).await?;
        let enc = encode_path_segment(&id);
        if method == Method::DELETE {
            // Resin expects no body on DELETE /{id}.
            return Ok((
                format!("/api/v1/{}/{}", collection, enc),
                reqwest::Body::from(Vec::<u8>::new()),
            ));
        }
        // Reuse the pure snake_case rewriter so unit tests can lock the
        // contract without spinning up reqwest.
        let new_body_val = rewrite_patch_body_snake_case(&body_val)?;
        let new_body =
            serde_json::to_vec(&new_body_val).map_err(|e| format!("re-serialize body: {e}"))?;
        return Ok((
            format!("/api/v1/platforms/{}", enc),
            reqwest::Body::from(new_body),
        ));
    }

    // --- 2. GET /platforms?leases_for=<name> -> /platforms/{id}/leases ---
    if method == Method::GET && is_platforms {
        if let Some(name) = query_param_decoded(query, "leases_for") {
            egressapikey_app::commands::validate_short_name(&name, "platform")?;
            let id = resolve_id(client, upstream_base, admin_token, "platforms", &name).await?;
            return Ok((
                format!("/api/v1/platforms/{}/leases", encode_path_segment(&id)),
                reqwest::Body::from(Vec::<u8>::new()),
            ));
        }
    }

    // --- 3. POST /subscriptions?refresh=<name> -> actions/refresh ---
    if method == Method::POST && is_subscriptions {
        if let Some(name) = query_param_decoded(query, "refresh") {
            egressapikey_app::commands::validate_short_name(&name, "subscription")?;
            let id = resolve_id(client, upstream_base, admin_token, "subscriptions", &name).await?;
            return Ok((
                format!(
                    "/api/v1/subscriptions/{}/actions/refresh",
                    encode_path_segment(&id)
                ),
                reqwest::Body::from(Vec::<u8>::new()),
            ));
        }
    }

    // --- 4. PUT/DELETE /account-header-rules + body url_prefix -> /{prefix} ---
    if is_rules && (method == Method::PUT || method == Method::DELETE) {
        let body_val = parse_body(body_bytes)?;
        let prefix = body_val
            .get("url_prefix")
            .and_then(|v| v.as_str())
            .ok_or_else(|| r#"body missing "url_prefix" field"#.to_string())?;
        if prefix.is_empty()
            || prefix.len() > 512
            || prefix.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f)
        {
            return Err("url_prefix invalid (1..512 chars, no control)".to_string());
        }
        let enc = encode_path_segment(prefix);
        if method == Method::DELETE {
            return Ok((
                format!("/api/v1/account-header-rules/{}", enc),
                reqwest::Body::from(Vec::<u8>::new()),
            ));
        }
        // Resin PUT body is {"headers": [...]}; url_prefix lives in the path.
        let out = serde_json::json!({
            "headers": body_val.get("headers").cloned().unwrap_or(serde_json::Value::Null)
        });
        let new_body = serde_json::to_vec(&out).map_err(|e| format!("re-serialize body: {e}"))?;
        return Ok((
            format!("/api/v1/account-header-rules/{}", enc),
            reqwest::Body::from(new_body),
        ));
    }

    // --- 5. POST /platforms validates the Resin V1 name rule. ---
    // The desktop command (create_platform_from_name) rejects names containing
    // . : | / \ @ ? # % ~ or any whitespace; the BFF must enforce the SAME rule
    // or a browser caller could create a name the desktop would have refused
    // One validation, effective in both places). Validates only -
    // the body still passes through unchanged below.
    if method == Method::POST && is_platforms {
        let body_val = parse_body(body_bytes)?;
        let name = body_val
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| r#"body missing "name" field"#.to_string())?;
        resin_core::validate_platform_name(name)
            .map_err(|e| format!("platform name rejected: {e}"))?;
    }
    Ok((
        raw_path.to_string(),
        reqwest::Body::from(body_bytes.clone()),
    ))
}

/// Headless L2 context (option C). The desktop shell holds the same
/// stores as Tauri managed state; headless builds them once at startup so the
/// port family is not a desktop-only capability.
struct PortCtx {
    db: resin_core::DbPool,
    whitebox: resin_core::WhiteboxConfigStore,
    forwarder: resin_core::PortForwarder,
    api_base: String,
    admin_token: String,
    /// The Resin sidecar this process booted — the BFF shell routes share it
    /// for status/logs/restart and ResinClient construction.
    sidecar: Arc<SidecarHandle>,
    /// L2 strategy service rooted at state_root — the same store shape the
    /// desktop opens at app_config_dir (option C: same stores, different root).
    strategy: resin_core::StrategyService<resin_core::FsStrategyStore>,
    /// L1 preferences document (<state_root>/settings.json) backing the
    /// settings KV surface the SPA's store() shim writes through.
    settings_path: PathBuf,
    /// L2/L1 root (egressapikey.db, whitebox file, settings.json, backups/).
    state_root: PathBuf,
    /// Directories the sidecar restart seam re-enters (resolved at boot).
    restart: headless_shell::RestartDirs,
}

impl PortCtx {
    /// Same constructor the Tauri commands use (commands::common::resin_client),
    /// so the loopback SSRF guard and the shared reqwest pool apply here too.
    fn client(&self) -> Result<resin_core::ResinClient, String> {
        resin_core::ResinClient::new(&self.api_base, self.admin_token.clone())
            .map_err(|e| format!("sidecar client: {e:?}"))
    }
}

/// Error shape mirrors the BFF translation failure so the SPA error mapping
/// stays uniform across proxied and BFF-native routes.
fn port_err(status: StatusCode, msg: &str) -> Response {
    (
        status,
        axum::Json(serde_json::json!({
            "error": { "code": "HEADLESS_PORT", "message": msg }
        })),
    )
        .into_response()
}

fn read_json_body(body: &Bytes) -> Result<serde_json::Value, String> {
    serde_json::from_slice(body).map_err(|e| format!("invalid JSON body: {e}"))
}

async fn ports_list_h(ctx: Arc<PortCtx>) -> Response {
    match ctx.db.list_ports() {
        Ok(rows) => axum::Json(rows).into_response(),
        Err(e) => port_err(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn ports_suggest_h(ctx: Arc<PortCtx>) -> Response {
    // ADR-0031 lives in resin-core so the GUI and the establish cascade suggest
    // the same port; headless calls that one implementation.
    match resin_core::subscription_pipeline::suggest_free_entry_port(&ctx.db) {
        Ok(port) => axum::Json(serde_json::json!({ "port": port })).into_response(),
        Err(e) => port_err(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn ports_running_h(ctx: Arc<PortCtx>) -> Response {
    axum::Json(ctx.forwarder.running_ports()).into_response()
}
/// Read one port_mapping row, or a NOT_FOUND response.
fn port_row(ctx: &PortCtx, port: u16) -> Result<resin_core::PortMapping, Response> {
    match ctx.db.list_ports() {
        Ok(rows) => rows.into_iter().find(|m| m.port == port).ok_or_else(|| {
            port_err(
                StatusCode::NOT_FOUND,
                &format!("port {port} is not configured"),
            )
        }),
        Err(e) => Err(port_err(StatusCode::INTERNAL_SERVER_ERROR, &e)),
    }
}

/// ADR-0069 D1 / option C: the headless port family calls the SAME
/// shared implementation as the desktop IPC command, so the L2-first write
/// order and the domain validation cannot drift between the two surfaces
/// . Only the transport-facing error mapping differs.
impl egressapikey_app::commands::ResinEndpointSource for PortCtx {
    fn endpoint_client(&self) -> Result<resin_core::ResinClient, String> {
        self.client()
    }
}

/// Translate the shared implementation
/// 's typed error into a headless HTTP
/// status. Validation-class rejections stay 400; a bind conflict is 409;
/// everything the shared implementation reports around the L3 (Resin) step is
/// a 502 on the upstream engine.
fn port_ipc_err(e: &resin_core::IpcError) -> Response {
    match e {
        resin_core::IpcError::InvalidInput { .. }
        | resin_core::IpcError::NotFound { .. }
        | resin_core::IpcError::InvalidStrategy { .. } => {
            port_err(StatusCode::BAD_REQUEST, &format!("{e:?}"))
        }
        resin_core::IpcError::BindConflict { .. } => {
            port_err(StatusCode::CONFLICT, &format!("{e:?}"))
        }
        _ => port_err(StatusCode::BAD_GATEWAY, &format!("{e:?}")),
    }
}

async fn ports_upsert_h(ctx: Arc<PortCtx>, port: u16, body: Bytes) -> Response {
    let v = match read_json_body(&body) {
        Ok(v) => v,
        Err(e) => return port_err(StatusCode::BAD_REQUEST, &e),
    };
    let protocol = v
        .get("protocol")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let platform_name = v
        .get("platform_name")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let account = v
        .get("account")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let label = v
        .get("label")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
    let auth_required = v
        .get("auth_required")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    match egressapikey_app::commands::port_upsert_impl(
        &ctx.db,
        &*ctx,
        &ctx.forwarder,
        &ctx.whitebox,
        port,
        protocol,
        platform_name,
        account,
        label,
        enabled,
        auth_required,
    )
    .await
    {
        Ok(m) => axum::Json(m).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn ports_remove_h(ctx: Arc<PortCtx>, port: u16) -> Response {
    match egressapikey_app::commands::port_remove_impl(
        &ctx.db,
        &*ctx,
        &ctx.forwarder,
        &ctx.whitebox,
        port,
    )
    .await
    {
        Ok(removed) => axum::Json(removed).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn ports_toggle_h(ctx: Arc<PortCtx>, port: u16, body: Bytes) -> Response {
    let v = match read_json_body(&body) {
        Ok(v) => v,
        Err(e) => return port_err(StatusCode::BAD_REQUEST, &e),
    };
    let enabled = match v.get("enabled").and_then(|x| x.as_bool()) {
        Some(b) => b,
        None => return port_err(StatusCode::BAD_REQUEST, "body missing boolean enabled"),
    };
    match egressapikey_app::commands::port_toggle_impl(
        &ctx.db,
        &*ctx,
        &ctx.forwarder,
        &ctx.whitebox,
        port,
        enabled,
    )
    .await
    {
        Ok(m) => axum::Json(m).into_response(),
        Err(e) => port_ipc_err(&e),
    }
}

async fn ports_bind_platform_h(ctx: Arc<PortCtx>, port: u16, body: Bytes) -> Response {
    if let Err(e) = egressapikey_app::commands::validate_port_segments(port) {
        return port_err(StatusCode::BAD_REQUEST, &e);
    }
    let v = match read_json_body(&body) {
        Ok(v) => v,
        Err(e) => return port_err(StatusCode::BAD_REQUEST, &e),
    };
    let platform_name = v
        .get("platform_name")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    if let Err(e) = egressapikey_app::commands::validate_short_name(&platform_name, "platform_name")
    {
        return port_err(StatusCode::BAD_REQUEST, &e);
    }
    let mut next = ctx.whitebox.snapshot();
    let out = match next.entry_ports.iter_mut().find(|r| r.port == port) {
        Some(row) => {
            row.platform_name = platform_name;
            row.clone()
        }
        None => {
            return port_err(
                StatusCode::NOT_FOUND,
                &format!("port {port} not in whitebox"),
            )
        }
    };
    match ctx.whitebox.apply(&ctx.db, &ctx.forwarder, next).await {
        Ok(_) => axum::Json(out).into_response(),
        Err(e) => port_err(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

/// Mode B: headless has no shell-side credential injection -
/// Resin listens natively and the CLIENT supplies the Platform.Account
/// credential once. The field shape stays identical to the desktop answer
/// (PortAuthInfo) so the view needs no branch; `data_plane_mode` tells the
/// truth about WHERE the credential comes from.
async fn ports_auth_info_h(ctx: Arc<PortCtx>, port: u16) -> Response {
    if let Err(e) = egressapikey_app::commands::validate_port_segments(port) {
        return port_err(StatusCode::BAD_REQUEST, &e);
    }
    let m = match port_row(&ctx, port) {
        Ok(m) => m,
        Err(resp) => return resp,
    };
    let username = if m.account.trim().is_empty() {
        format!("{}.port-{}", m.platform_name, port)
    } else {
        format!("{}.{}", m.platform_name, m.account)
    };
    axum::Json(serde_json::json!({
        "username": username,
        "password": ctx.forwarder.proxy_token(),
        "auth_required": m.auth_required,
        "platform_name": m.platform_name,
        "port": port,
        "data_plane_mode": "B",
    }))
    .into_response()
}

async fn ports_health_h(_ctx: Arc<PortCtx>, port: u16, protocol: Option<String>) -> Response {
    // commands::port_health_check takes NO Tauri State, so headless calls the
    // SAME function the desktop IPC uses - one implementation, two transports
    // ("one validation effective in both places"). It probes
    // 127.0.0.1:{port}, the same host this process runs on.
    match egressapikey_app::commands::port_health_check(port, protocol).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => port_err(StatusCode::BAD_REQUEST, &format!("{e:?}")),
    }
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
    fn encode_path_segment_preserves_alphanum_and_dashes() {
        assert_eq!(encode_path_segment("abc-123_xyz"), "abc-123_xyz");
    }

    #[test]
    fn encode_path_segment_escapes_chinese_and_special() {
        let out = encode_path_segment("名前");
        assert!(!out.contains('名'));
        assert!(out.starts_with('%'));
        assert_eq!(out.matches('%').count(), 6);
    }

    #[test]
    fn encode_path_segment_percent_decode_round_trip() {
        // The BFF encodes with the shared strict set and decodes with the
        // local percent_decode; the pair must round-trip arbitrary UTF-8
        // (including %20 for space - never the form-urlencoded +).
        for s in ["abc-123_xyz", "a b+c/d", "名前", "a%20b"] {
            assert_eq!(percent_decode(&encode_path_segment(s)), s);
        }
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
        assert_eq!(
            obj.get("allocation_policy").and_then(|v| v.as_str()),
            Some("BALANCED")
        );
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

    // --- the new BFF translation helpers (pure, no reqwest). ---

    #[test]
    fn split_query_separates_path_and_query() {
        assert_eq!(split_query("/api/v1/platforms"), ("/api/v1/platforms", ""));
        assert_eq!(
            split_query("/api/v1/platforms?leases_for=openai"),
            ("/api/v1/platforms", "leases_for=openai")
        );
    }

    #[test]
    fn query_param_decoded_reads_and_percent_decodes() {
        assert_eq!(
            query_param_decoded("leases_for=openai", "leases_for").as_deref(),
            Some("openai")
        );
        assert_eq!(
            query_param_decoded("a=1&refresh=my%20sub", "refresh").as_deref(),
            Some("my sub")
        );
        assert_eq!(query_param_decoded("a=1", "missing"), None);
        // A bare key (no "=") reads as the empty string, not a panic.
        assert_eq!(query_param_decoded("flag", "flag").as_deref(), Some(""));
    }

    #[test]
    fn percent_decode_handles_multibyte_and_leaves_plus_literal() {
        assert_eq!(percent_decode("%E4%B8%AD"), "中");
        // Deliberate: "+" is NOT treated as space (identifiers only here).
        assert_eq!(percent_decode("a+b"), "a+b");
        // A trailing/invalid % sequence falls through untouched.
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn read_name_reuses_the_shared_short_name_validator() {
        assert_eq!(
            read_name(&serde_json::json!({ "name": "openai" })).unwrap(),
            "openai"
        );
        assert!(read_name(&serde_json::json!({})).is_err(), "missing name");
        assert!(
            read_name(&serde_json::json!({ "name": "" })).is_err(),
            "empty"
        );
        assert!(
            read_name(&serde_json::json!({ "name": "a".repeat(129) })).is_err(),
            "over the shared NAME_MAX bound"
        );
    }

    #[test]
    fn allocation_policy_allow_list_is_the_shared_command_constant() {
        // Locks the "one validation, two transports" contract: the BFF
        // rewriter must accept exactly what the Tauri command accepts.
        assert!(egressapikey_app::commands::ALLOWED_ALLOCATION_POLICIES.contains(&"BALANCED"));
        assert!(!egressapikey_app::commands::ALLOWED_ALLOCATION_POLICIES.contains(&"random"));
        let body = serde_json::json!({ "name": "x", "allocationPolicy": "BALANCED" });
        assert!(rewrite_patch_body_snake_case(&body).is_ok());
        let bad = serde_json::json!({ "name": "x", "allocationPolicy": "random" });
        assert!(rewrite_patch_body_snake_case(&bad).is_err());
    }

    #[test]
    fn proxy_client_is_one_shared_instance() {
        // Regression pin: proxy_to_resin must reuse ONE pooled client for
        // every upstream hop, not build a fresh pool per request.
        let a = proxy_client().expect("reqwest client builds");
        let b = proxy_client().expect("reqwest client builds");
        assert!(std::ptr::eq(a, b));
    }
}

#[cfg(test)]
mod guard_wiring_tests {
    use super::*;
    use std::sync::Mutex;

    const TEST_TOKEN: &str = "s3cret-token";
    const TEST_ADMIN_TOKEN: &str = "test-admin-token";

    /// One request as the mock resin control plane saw it.
    #[derive(Debug, Clone)]
    struct Seen {
        method: String,
        uri: String,
        body: String,
    }

    /// Fresh throwaway L2 state root for one test.
    fn temp_state_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "egressapikey-guard-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp state root");
        dir
    }

    /// Serve the REAL router - the one `main` serves - over a throwaway L2
    /// context, on an ephemeral loopback port. Returns (base_url, state_root).
    ///
    /// This is the regression lock the guard was missing: it drives
    /// `build_router` end to end, guard layer included, instead of the
    /// `headless_security` primitives alone. The router lives in the headless
    /// BIN rather than the lib, so the lock lives here too.
    async fn serve_router(api_base: &str) -> (String, PathBuf) {
        let dir = temp_state_root();
        let db = resin_core::DbPool::open(&dir.join("egressapikey.db")).expect("open db");
        let rows = db.list_ports().expect("list_ports");
        let whitebox = resin_core::WhiteboxConfigStore::open(
            dir.join(resin_core::WHITEBOX_CONFIG_FILE),
            resin_core::WhiteboxConfig::from_ports(rows),
        )
        .await
        .expect("open whitebox");
        let forwarder = resin_core::PortForwarder::new(
            db.clone(),
            "127.0.0.1",
            1,
            "test-proxy-token".to_string(),
        );
        // A stub sidecar handle (no child): enough for the guard/route wiring
        // these tests exercise; shell handlers that need a live sidecar are
        // covered by the smoke script, not this fixture.
        let stub_sidecar = Arc::new(SidecarHandle {
            child: std::sync::Mutex::new(None),
            mode: std::sync::RwLock::new(egressapikey_app::sidecar::RunningMode::Running),
            log_buf: egressapikey_app::sidecar::LogBuffer::new(),
            api_port: 0,
            admin_token: String::new(),
            proxy_token: String::new(),
            healthz_last_check: std::sync::RwLock::new(String::new()),
            #[cfg(target_os = "windows")]
            job_handle: None,
        });
        let port_ctx = Arc::new(PortCtx {
            db,
            whitebox,
            forwarder,
            api_base: api_base.to_string(),
            admin_token: TEST_ADMIN_TOKEN.to_string(),
            sidecar: stub_sidecar,
            strategy: resin_core::StrategyService::new(resin_core::FsStrategyStore::new(
                dir.join("egressapikey-strategy.json"),
            )),
            settings_path: dir.join("settings.json"),
            state_root: dir.clone(),
            restart: headless_shell::RestartDirs {
                state_dir: dir.join("resin-state"),
                cache_dir: dir.join("resin-cache"),
                log_dir: dir.join("resin-logs"),
                binary_path: dir.join("resin"),
            },
        });
        let guard = Arc::new(HeadlessGuard::new(
            headless_security::allowed_hosts("127.0.0.1", &[]),
            TEST_TOKEN.to_string(),
            Vec::new(),
        ));
        let app = build_router(
            &dir.join("dist"),
            api_base.to_string(),
            TEST_ADMIN_TOKEN.to_string(),
            guard,
            port_ctx,
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let addr = listener.local_addr().expect("local_addr");
        tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await;
        });
        (format!("http://{addr}"), dir)
    }

    /// Mock resin control plane: answers 200 JSON and records what it saw.
    async fn serve_mock_upstream() -> (String, Arc<Mutex<Vec<Seen>>>) {
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = seen.clone();
        let echo = move |method: Method, uri: axum::http::Uri, body: Bytes| {
            let recorder = recorder.clone();
            async move {
                {
                    let mut sink = recorder.lock().expect("mock recorder poisoned");
                    sink.push(Seen {
                        method: method.to_string(),
                        uri: uri.to_string(),
                        body: String::from_utf8_lossy(&body).to_string(),
                    });
                }
                axum::Json(serde_json::json!({ "ok": true })).into_response()
            }
        };
        let app = Router::new()
            .route("/api/v1/*path", any(echo.clone()))
            .route("/metrics/*path", any(echo.clone()))
            .fallback(echo);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock port");
        let addr = listener.local_addr().expect("mock local_addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}"), seen)
    }

    /// Send a raw HTTP/1.1 request and read the whole response. Raw bytes keep
    /// the Host header under test control (an HTTP client would rewrite it).
    async fn raw_request(base: &str, request: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let addr = base.trim_start_matches("http://").trim_end_matches('/');
        let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut buf = Vec::new();
        let _ = stream.read_to_end(&mut buf).await;
        String::from_utf8_lossy(&buf).to_string()
    }

    /// A token-bearing GET through the guard.
    fn with_token(path_and_query: &str) -> String {
        format!(
            "GET {path_and_query} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {TEST_TOKEN}\r\nConnection: close\r\n\r\n"
        )
    }

    #[tokio::test]
    async fn guard_layer_rejects_control_plane_without_token() {
        let (base, dir) = serve_router("http://127.0.0.1:1/").await;
        let resp = raw_request(
            &base,
            "GET /api/v1/platforms HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(
            resp.starts_with("HTTP/1.1 401"),
            "a tokenless control-plane call must be 401, got: {resp}"
        );
        assert!(
            resp.to_ascii_lowercase()
                .contains("www-authenticate: bearer"),
            "the 401 must advertise the Bearer scheme, got: {resp}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Regression lock for the axum-0.7 literal-path bug — under
    /// "{port}" syntax these URIs silently fell through to the /api/v1/*path
    /// proxy. Every :port route must reach the BFF-native handler instead of
    /// being proxied verbatim (the mock upstream always answers {"ok":true},
    /// so a passthrough is instantly recognisable).
    #[tokio::test]
    async fn bff_port_param_routes_reach_native_handlers_not_proxy() {
        let (mock_base, _seen) = serve_mock_upstream().await;
        let (base, dir) = serve_router(&mock_base).await;
        for (method, path) in [
            ("PUT", "/api/v1/ports/8080"),
            ("DELETE", "/api/v1/ports/8080"),
            ("PATCH", "/api/v1/ports/8080"),
            ("PATCH", "/api/v1/ports/8080/platform"),
            ("GET", "/api/v1/ports/8080/auth"),
            ("GET", "/api/v1/ports/8080/health"),
        ] {
            let req = format!(
                "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {TEST_TOKEN}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
            );
            let resp = raw_request(&base, &req).await;
            assert!(
                !resp.contains("\"ok\":true"),
                "{method} {path} must NOT pass through to the mock upstream, got: {resp}"
            );
            assert!(
                !resp.starts_with("HTTP/1.1 405"),
                "{method} {path} must match a registered route, got: {resp}"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn guard_layer_rejects_unexpected_host_on_api_and_static_routes() {
        let (base, dir) = serve_router("http://127.0.0.1:1/").await;
        for path in ["/api/v1/platforms", "/", "/index.html"] {
            let resp = raw_request(
                &base,
                &format!(
                    "GET {path} HTTP/1.1\r\nHost: evil.example.com\r\nConnection: close\r\n\r\n"
                ),
            )
            .await;
            assert!(
                resp.starts_with("HTTP/1.1 403"),
                "a DNS-rebinding Host must be 403 on {path}, got: {resp}"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn guard_layer_plants_the_session_cookie_on_query_bootstrap() {
        let (mock_base, _seen) = serve_mock_upstream().await;
        let (base, dir) = serve_router(&mock_base).await;
        let resp = raw_request(
            &base,
            &format!(
                "GET /api/v1/platforms?{}={} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
                headless_security::TOKEN_QUERY,
                TEST_TOKEN
            ),
        )
        .await;
        assert!(
            resp.starts_with("HTTP/1.1 200"),
            "the query bootstrap must pass the guard, got: {resp}"
        );
        let lower = resp.to_ascii_lowercase();
        assert!(
            lower.contains(&format!(
                "set-cookie: {}={}",
                headless_security::TOKEN_COOKIE,
                TEST_TOKEN
            )),
            "the bootstrap request must plant the session cookie, got: {resp}"
        );
        assert!(
            lower.contains("httponly"),
            "the session cookie must be HttpOnly, got: {resp}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn guard_layer_admits_the_token_bearing_bff_ports_route() {
        let (base, dir) = serve_router("http://127.0.0.1:1/").await;
        let resp = raw_request(&base, &with_token("/api/v1/ports")).await;
        assert!(
            resp.starts_with("HTTP/1.1 200"),
            "the BFF ports list route must stay reachable behind the guard, got: {resp}"
        );
        assert!(
            resp.contains("[]"),
            "the throwaway state root holds no port rows, got: {resp}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn proxy_preserves_the_get_query_and_forwards_the_body_once() {
        let (mock_base, seen) = serve_mock_upstream().await;
        let (base, dir) = serve_router(&mock_base).await;

        let get = raw_request(&base, &with_token("/api/v1/nodes?limit=5&cursor=abc")).await;
        assert!(
            get.starts_with("HTTP/1.1 200"),
            "the GET must be proxied to resin, got: {get}"
        );

        let body = r#"{"name":"openai"}"#;
        let post = raw_request(
            &base,
            &format!(
                "POST /api/v1/platforms HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {TEST_TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        )
        .await;
        assert!(
            post.starts_with("HTTP/1.1 200"),
            "the POST must be proxied to resin, got: {post}"
        );

        let recorded = seen.lock().expect("mock recorder poisoned").clone();
        let get_seen = recorded
            .iter()
            .find(|s| s.method == "GET")
            .expect("the GET must reach resin");
        assert_eq!(
            get_seen.uri, "/api/v1/nodes?limit=5&cursor=abc",
            "the query string must survive the BFF verbatim"
        );
        let posts: Vec<&Seen> = recorded.iter().filter(|s| s.method == "POST").collect();
        assert_eq!(
            posts.len(),
            1,
            "the body must be forwarded exactly once (no double wrap)"
        );
        assert_eq!(posts[0].uri, "/api/v1/platforms");
        assert_eq!(
            posts[0].body, body,
            "the forwarded body must be byte-identical to the request body"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// r12-wave-i D-002: the wildcard --trusted-proxy startup gate.
#[cfg(test)]
mod trusted_proxy_scope_tests {
    use super::*;

    #[test]
    fn wildcard_cidrs_refuse_startup_without_the_unrestricted_flag() {
        for s in ["0.0.0.0/0", "::/0"] {
            let net = parse_trusted_proxy(s).expect("a wildcard CIDR parses");
            assert!(is_wildcard_proxy_net(&net), "{s} must read as a wildcard");
            let err = trusted_proxy_scope_error(&[net], false)
                .expect("wildcard without the flag must refuse startup");
            assert!(err.contains("--trusted-proxy-unrestricted"));
        }
    }

    #[test]
    fn wildcard_cidr_passes_with_the_unrestricted_flag() {
        let net = parse_trusted_proxy("0.0.0.0/0").expect("a wildcard CIDR parses");
        assert!(trusted_proxy_scope_error(&[net], true).is_none());
    }

    #[test]
    fn concrete_proxies_never_need_the_flag() {
        let nets = [
            parse_trusted_proxy("127.0.0.1").unwrap(),
            parse_trusted_proxy("10.8.0.0/16").unwrap(),
            parse_trusted_proxy("::1").unwrap(),
        ];
        assert!(!nets.iter().any(is_wildcard_proxy_net));
        assert!(trusted_proxy_scope_error(&nets, false).is_none());
    }
}
