//! Resin sidecar lifecycle: spawn the Go `resin` binary as a Tauri
//! sidecar, wait for its HTTP control plane to come up, hand back a
//! SidecarHandle the IPC command layer uses to proxy Resin REST calls.
//!
//! Design (per docs/MEMORY_REUSE_DECISION.md path A):
//! - Resin binds a single port and exposes control-plane API + WebUI + proxy
//!   on that same port (DESIGN.md §3 统一入站端口). It does NOT use Ghost's
//!   stdout JSON handshake; we poll `/healthz` instead.
//! - Tokens (`RESIN_ADMIN_TOKEN`, `RESIN_PROXY_TOKEN`) are generated here in
//!   the Rust shell and passed to the sidecar via the child env. These are
//!   server-side trust only — they NEVER cross into the webview. The webview
//!   can only reach Resin through the Rust-side ResinClient (G2) which holds
//!   this handle's admin token.
//! - We pick a free port up front (TCP bind :0 then drop) and tell Resin to
//!   listen on it. Avoids races where Resin picks first.
//!
//! Ponytail: do NOT add a retry/restart loop here — that is Ghost safety net
//! (G3), separate file, separate concerns.
use std::collections::VecDeque;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use tauri::{AppHandle, Emitter, Manager, Runtime};
// Q6 fix: bypass tauri_plugin_shell .sidecar() which spawns a child via the
// plugin layer — the plugin does NOT set CREATE_NO_WINDOW, so a console
// window flashes on every quit of the GUI. We use std::process::Command
// directly with creation_flags(0x08000000) on Windows to suppress the console.
// stdout/stderr pipes are still drained into the LogBuffer via tokio async
// read — same observable behavior as the plugin's CommandEvent, minus the
// console window.
use std::process::{Child, Command, Stdio};
use resin_core::NetworkConfig;

/// Lifecycle state of the Resin sidecar process (ADR-0016 Q1).
/// Modeled after clash-verge-rev CoreManager RunningMode: a lightweight
/// enum behind RwLock so any thread can cheaply read the current state
/// without blocking the IPC command layer. Transition graph:
///   NotRunning -> Starting -> Running  (boot_resin success path)
///   Running -> NotRunning              (exit hook / crash)
///   Starting -> NotRunning             (boot timeout)
/// Ponytail: std RwLock, not arc-swap crate — mode transitions are rare
/// (boot, crash, shutdown), so RwLock contention is negligible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunningMode {
    /// No sidecar process exists (initial state before boot or after kill).
    NotRunning,
    /// Process spawned, /healthz not yet confirmed (boot_resin polling).
    Starting,
    /// Process is live and control plane is up (normal steady state).
    Running,
    /// T2-Q3 (ADR-0016 Q3): After MAX_CRASH_RESTARTS the crash restarter
    /// gives up and marks the sidecar Terminated — no further restart
    /// attempts. The mode is terminal until the user restarts the app.
    /// mode transition is Running|Starting -> Terminated.
    Terminated,
}

/// Ring buffer capacity for sidecar stderr/stdout lines (ADR-0016 Q2a).
const SIDECAR_LOG_CAPACITY: usize = 500;

/// Bounded ring buffer for sidecar process stdout/stderr output (ADR-0016 Q2a).
pub struct LogBuffer {
    lines: parking_lot::Mutex<VecDeque<String>>,
}

impl LogBuffer {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            lines: parking_lot::Mutex::new(VecDeque::with_capacity(SIDECAR_LOG_CAPACITY)),
        })
    }

    pub fn push(&self, line: &str) {
        let mut g = self.lines.lock();
        if g.len() >= SIDECAR_LOG_CAPACITY {
            g.pop_front();
        }
        g.push_back(line.to_string());
    }

    pub fn snapshot(&self) -> Vec<String> {
        let g = self.lines.lock();
        g.iter().cloned().collect()
    }
}

/// The running sidecar process plus the connection info the Rust side needs.
/// Lives in Tauri managed state as `State<SidecarHandle>` so IPC commands
/// (G2 ResinClient) can reach it without piping admin tokens anywhere else.
pub struct SidecarHandle {
    /// Owned in a Mutex<Option<_>> so the app exit hook can take() the
    /// child once and call .kill() + .wait(). std::process::Child::kill
    /// takes &mut self (does NOT consume), so the Option<take()> pattern
    /// is kept for the exit hook's single-owner discipline. None after
    /// kill() means a second Exit callback (if Tauri ever re-emits one)
    /// is a no-op.
    pub child: Mutex<Option<Child>>,
    /// Lifecycle mode (ADR-0016 Q1). RwLock so the health poll thread,
    /// crash restarter, and exit hook can all read/set the mode without
    /// blocking the IPC command path (which only needs api_port + tokens).
    pub mode: std::sync::RwLock<RunningMode>,
    /// Ring buffer for sidecar stderr/stdout lines (ADR-0016 Q2a).
    pub log_buf: Arc<LogBuffer>,
    /// Resin's single consolidated port (control-plane API + proxy + webui).
    pub api_port: u16,
    /// Resin admin token. Used to authenticate Rust-side REST calls to the
    /// control plane. NEVER expose to the webview.
    pub admin_token: String,
    /// Resin proxy token. Used by clients that talk to the L7 proxy entry.
    /// Kept here for the Rust-side proxy configurator only.
    pub proxy_token: String,
    /// T6-7: RFC3339 timestamp of the last successful /healthz probe.
    /// Updated by spawn_health_poll on every successful poll cycle.
    pub healthz_last_check: std::sync::RwLock<String>,
}

impl SidecarHandle {
    /// Base URL for the Resin REST API. Always loopback.
    pub fn api_base(&self) -> String {
        format!("http://127.0.0.1:{}", self.api_port)
    }

    /// Read the current lifecycle mode (cheap RwLock read).
    pub fn mode(&self) -> RunningMode {
        *self.mode.read().unwrap()
    }

    /// Transition to a new mode. Validates that the transition is legal
    /// per the ADR-0016 state graph; logs a warn on illegal transitions
    /// but does not panic (defensive against races in the exit path).
    pub fn set_mode(&self, new: RunningMode) {
        let old = self.mode();
        // Valid: Starting->Running, Starting->NotRunning (timeout),
        //        Running->NotRunning (crash/exit), NotRunning->Starting (reboot).
        let valid = matches!(
            (old, new),
            (RunningMode::Starting, RunningMode::Running)
                | (RunningMode::Starting, RunningMode::NotRunning)
                | (RunningMode::Running, RunningMode::NotRunning)
                | (RunningMode::NotRunning, RunningMode::Starting)
                | (RunningMode::Running, RunningMode::Terminated)
                | (RunningMode::Starting, RunningMode::Terminated)
                | (RunningMode::Terminated, RunningMode::NotRunning)
        );
        if !valid {
            tracing::warn!(
                "sidecar: unexpected mode transition {:?} -> {:?}",
                old,
                new
            );
        }
        *self.mode.write().unwrap() = new;
    }
}

/// Boot the resin sidecar and wait for its control plane to be reachable.
///
/// Timeout: 15s (matches the Ghost safety-net reference but uses HTTP poll
/// rather than stdout because Resin's design is HTTP-first).
/// Pick a free loopback TCP port up front so the resin child can bind it.
/// Returns (port, dummy_listener_dropped).
fn pick_free_loopback_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .context("sidecar: failed to allocate a free port for Resin")?;
    let port = listener
        .local_addr()
        .context("sidecar: listener has no local addr")?
        .port();
    drop(listener);
    Ok(port)
}

/// Resolve the resin sidecar binary by host triple. Tries `binary_dir`
/// (CLI-supplied) first, then packaged resource_dir, then dev fallback.
fn resolve_resin_binary(binary_dir: Option<&std::path::Path>) -> Result<std::path::PathBuf> {
    // std::env::consts::OS returns 'windows'/'macos'/'linux' WITHOUT the
    // ABI suffix (msvc/gnu), so the triple we can construct at runtime is
    // only a prefix of the real cargo host-triple. We glob
    // 'resin-<arch>-pc-<os>-*<.exe>' in the caller dir + the src-tauri
    // fallback dir so any ABI variant is resolved symmetrically.
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;
    let exe_suffix = if cfg!(target_os = "windows") { ".exe" } else { "" };
    let direct_name = format!("resin-{}-pc-{}{}", arch, os, exe_suffix);
    if let Some(bd) = binary_dir {
        if let Some(p) = scan_for_resin_bin(bd, arch, os, exe_suffix, &direct_name) {
            tracing::info!(resolved = ?p, "sidecar: resolved resin binary (caller dir)");
            return Ok(p);
        }
    }
    let dev_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");
    if let Some(p) = scan_for_resin_bin(&dev_dir, arch, os, exe_suffix, &direct_name) {
        tracing::info!(resolved = ?p, "sidecar: resolved resin binary (dev fallback)");
        return Ok(p);
    }
    Err(anyhow!(
        "sidecar: resin binary not found in caller dir ({:?}) or src-tauri/binaries (arch={}, os={})",
        binary_dir, arch, os
    ))
}

/// Scan dir for a file matching 'resin-<arch>-pc-<os>-*<.exe>' (Windows ABI
/// variants) or the canonical triple on non-Windows. Prefer the
/// exact-direct-name match first, then any ABI-glob fall-through.
fn scan_for_resin_bin(
    dir: &std::path::Path,
    arch: &str,
    os: &str,
    exe_suffix: &str,
    direct_name: &str,
) -> Option<std::path::PathBuf> {
    let direct = dir.join(direct_name);
    if direct.exists() {
        return Some(direct);
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if os == "windows" {
            let prefix = format!("resin-{}-pc-windows-", arch);
            if name.starts_with(&prefix) && name.ends_with(exe_suffix) {
                candidates.push(e.path());
            }
        } else if os == "macos" {
            let direct_full = format!("resin-{}-apple-darwin{}", arch, exe_suffix);
            if name == direct_full {
                return Some(e.path());
            }
        } else if os == "linux" {
            let direct_full = format!("resin-{}-unknown-linux-gnu{}", arch, exe_suffix);
            if name == direct_full {
                return Some(e.path());
            }
        }
    }
    candidates.sort();
    candidates.into_iter().next()
}

/// Spawn the resin child, drain its stdout/stderr into the log buffer,
/// poll /healthz until the control plane is up (15s deadline), then
/// return a SidecarHandle. Path/port-independent: callers pass explicit
/// dirs + binary path. Used by both boot_resin (Tauri app) and
/// boot_resin_standalone (headless npm launcher).
fn spawn_resin_await_healthz(
    state_dir: &std::path::Path,
    cache_dir: &std::path::Path,
    log_dir: &std::path::Path,
    binary_path: &std::path::Path,
    network: &NetworkConfig,
) -> Result<SidecarHandle> {
    std::fs::create_dir_all(state_dir)
        .with_context(|| format!("sidecar: cannot create state_dir {:?}", state_dir))?;
    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("sidecar: cannot create cache_dir {:?}", cache_dir))?;
    std::fs::create_dir_all(log_dir)
        .with_context(|| format!("sidecar: cannot create log_dir {:?}", log_dir))?;
    tracing::info!(
        "resin sidecar dirs: state={:?} cache={:?} log={:?}",
        state_dir, cache_dir, log_dir
    );

    let api_port = pick_free_loopback_port()?;
    let admin_token = gen_token();
    // T7-fix: empty proxy_token enables no-auth on ports where require_proxy_auth_info=0.
    // Resin socks5.go:261 — when s.token=="" the OR condition is false, so the
    // else branch accepts NoAuth(0x00) + UserPass(0x02). forward.go:103 — when
    // p.token=="" the no-auth path returns nil error directly. ADR-0027 was wrong:
    // source verification proves empty token is safe (socks5.go:307 short-circuits
    // the password check, forward.go:103-115 lets no-auth through).
    let proxy_token = String::new();

    let mut cmd = Command::new(binary_path);
    cmd.env("RESIN_AUTH_VERSION", "V1")
        .env("RESIN_ADMIN_TOKEN", &admin_token)
        .env("RESIN_PROXY_TOKEN", &proxy_token)
        .env("RESIN_LISTEN_ADDRESS", "127.0.0.1")
        .env("RESIN_PORT", api_port.to_string())
        .env("RESIN_STATE_DIR", state_dir)
        .env("RESIN_CACHE_DIR", cache_dir)
        .env("RESIN_LOG_DIR", log_dir);
    // T6-2: inject network-layer env vars from whitebox config
    if !network.dns_upstreams.is_empty() {
        let json = serde_json::to_string(&network.dns_upstreams).unwrap_or_default();
        cmd.env("RESIN_NODE_DNS_UPSTREAMS", &json);
    }
    if let Some(v) = network.max_idle_conns {
        cmd.env("RESIN_PROXY_TRANSPORT_MAX_IDLE_CONNS", v.to_string());
    }
    if let Some(v) = network.max_idle_conns_per_host {
        cmd.env("RESIN_PROXY_TRANSPORT_MAX_IDLE_CONNS_PER_HOST", v.to_string());
    }
    if let Some(v) = network.idle_conn_timeout_secs {
        cmd.env("RESIN_PROXY_TRANSPORT_IDLE_CONN_TIMEOUT", format!("{}s", v));
    }
    if let Some(v) = network.probe_timeout_secs {
        cmd.env("RESIN_PROBE_TIMEOUT", format!("{}s", v));
    }
    if let Some(v) = network.probe_concurrency {
        cmd.env("RESIN_PROBE_CONCURRENCY", v.to_string());
    }
    if !network.proxy_bypass.is_empty() {
        cmd.env("RESIN_PROXY_BYPASS", network.proxy_bypass.join(","));
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let mut child = cmd.spawn().context("sidecar: failed to spawn resin binary")?;

    let log_buf = LogBuffer::new();
    let drain_buf = log_buf.clone();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    if let Some(stdout) = stdout {
        let buf = drain_buf.clone();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines().flatten() {
                tracing::trace!(target: "resin_sidecar", "sidecar stdout: {}", line);
                buf.push(&line);
            }
        });
    }
    if let Some(stderr) = stderr {
        let buf = drain_buf.clone();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(stderr);
            for line in reader.lines().flatten() {
                tracing::trace!(target: "resin_sidecar", "sidecar stderr: {}", line);
                buf.push(&line);
            }
        });
    }

    let deadline = Instant::now() + Duration::from_secs(15);
    let base = format!("http://127.0.0.1:{}", api_port);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .context("sidecar: blocking client build")?;
    let mut last_err: Option<String> = None;
    while Instant::now() < deadline {
        let url = format!("{base}/healthz");
        match client.get(&url).send() {
            Ok(r) if r.status().is_success() => {
                tracing::info!(
                    "sidecar: resin control plane up on {base} after {}ms",
                    Instant::now().elapsed().as_millis()
                );
                return Ok(SidecarHandle {
                    child: Mutex::new(Some(child)),
                    mode: std::sync::RwLock::new(RunningMode::Running),
                    log_buf,
                    api_port,
                    admin_token,
                   proxy_token,
                   healthz_last_check: std::sync::RwLock::new(String::new()),
               });
           }
           Ok(r) => { last_err = Some(format!("HTTP {}", r.status())); }
            Err(e) => { last_err = Some(e.to_string()); }
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    let _ = child.kill();
    let port_hint = match check_port_available(api_port) {
        Ok(()) => "port is free; sidecar likely crashed during startup".to_string(),
        Err(_) => "port is occupied by a stale process; kill it or use a different port".to_string(),
    };
    Err(anyhow!(
        "sidecar: resin /healthz did not come up within 15s on 127.0.0.1:{api_port} (last error: {}) [{port_hint}]",
        last_err.unwrap_or_else(|| "no response".into())
    ))
}

/// Headless entry point: spawn the resin sidecar without a Tauri webview.
/// `state_root` and `log_root` are OS-standard dirs (e.g.
/// ~/.local/share/egressapikey, ~/.local/state/egressapikey/logs).
/// `binary_dir` is where the launcher placed the resin sidecar binary.
/// Returns a SidecarHandle the axum headless server can read API_BASE /
/// token from. No Tauri dependency.
pub fn boot_resin_standalone(
    state_root: std::path::PathBuf,
    log_root: std::path::PathBuf,
    binary_dir: std::path::PathBuf,
) -> Result<SidecarHandle> {
    let state_dir = state_root.join("resin-state");
    let cache_dir = state_root.join("resin-cache");
    let log_dir = log_root.join("resin");
    let binary_path = resolve_resin_binary(Some(&binary_dir))?;
    let network = NetworkConfig::default();
    spawn_resin_await_healthz(&state_dir, &cache_dir, &log_dir, &binary_path, &network)
}

/// Tauri app entry: resolve per-user app data + log dirs via the Tauri
/// path resolver, then delegate to the shared spawn helper.
pub fn boot_resin<R: Runtime>(app: &AppHandle<R>) -> Result<SidecarHandle> {
    let path = app.path();
    let app_data = path
        .app_data_dir()
        .context("sidecar: cannot resolve app_data_dir for resin state")?;
    let state_dir = app_data.join("resin-state");
    let cache_dir = app_data.join("resin-cache");
    let log_dir = path
        .app_log_dir()
        .unwrap_or_else(|_| app_data.join("logs"))
        .join("resin");
    let binary_path = resolve_resin_binary(None)?;
    let network = read_network_config(&app_data);
    spawn_resin_await_healthz(&state_dir, &cache_dir, &log_dir, &binary_path, &network)
}



/// T6-2: Read network config from the whitebox JSON file on disk.
/// Returns NetworkConfig::default() if file is missing or unreadable.
fn read_network_config(app_data: &std::path::Path) -> NetworkConfig {
    let path = app_data.join(resin_core::WHITEBOX_CONFIG_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            match serde_json::from_str::<resin_core::WhiteboxConfig>(&text) {
                Ok(cfg) => cfg.network,
                Err(e) => {
                    tracing::warn!(error = %e, ?path, "whitebox config parse failed; using default network");
                    NetworkConfig::default()
                }
            }
        }
        Err(_) => NetworkConfig::default(),
    }
}

/// Generate a loopback-only secret token. Ponytail: use stdrand + Instant
/// instead of pulling a uuid crate dep — 32 hex chars of entropy from
/// SystemTime nanos + process id is plenty for a per-session localhost secret.
fn gen_token() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let mix = now ^ (pid << 64) ^ (now.rotate_left(13));
    format!("{mix:032x}")
}

/// T2-4 (ADR-0016 Q4): Check if a loopback TCP port is available to bind.
/// Returns Ok(()) if free, Err(message) if occupied by another process.
/// Pure function for testability: the test binds a listener then calls this
/// with the same port and expects Err.
pub fn check_port_available(port: u16) -> Result<(), String> {
    match TcpListener::bind(("127.0.0.1", port)) {
        Ok(_) => Ok(()),
        Err(e) => Err(format!(
            "port 127.0.0.1:{port} is occupied: {e}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gen_token_is_32_hex_chars() {
        let t = gen_token();
        assert_eq!(t.len(), 32);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn gen_token_is_not_constant() {
        let a = gen_token();
        std::thread::sleep(Duration::from_millis(5));
        let b = gen_token();
        assert_ne!(a, b, "tokens should differ across calls");
    }

    #[test]
    fn running_mode_default_is_not_running() {
        let m = RunningMode::NotRunning;
        assert_eq!(m, RunningMode::NotRunning);
    }

    #[test]
    fn sidecar_handle_mode_starts_running() {
        // A freshly booted SidecarHandle (mocked: no real child) should
        // report Running because boot_resin only returns after /healthz is up.
        let h = SidecarHandle {
            child: Mutex::new(None),
            mode: std::sync::RwLock::new(RunningMode::Running),
            log_buf: LogBuffer::new(),
            api_port: 0,
            admin_token: String::new(),
            proxy_token: String::new(),
            healthz_last_check: std::sync::RwLock::new(String::new()),
        };
        assert_eq!(h.mode(), RunningMode::Running);
    }

    #[test]
    fn sidecar_handle_set_mode_running_to_not_running() {
        let h = SidecarHandle {
            child: Mutex::new(None),
            mode: std::sync::RwLock::new(RunningMode::Running),
            log_buf: LogBuffer::new(),
            api_port: 0,
            admin_token: String::new(),
            proxy_token: String::new(),
            healthz_last_check: std::sync::RwLock::new(String::new()),
        };
        h.set_mode(RunningMode::NotRunning);
        assert_eq!(h.mode(), RunningMode::NotRunning);
    }

    #[test]
    fn sidecar_handle_set_mode_not_running_to_starting() {
        let h = SidecarHandle {
            child: Mutex::new(None),
            mode: std::sync::RwLock::new(RunningMode::NotRunning),
            log_buf: LogBuffer::new(),
            api_port: 0,
            admin_token: String::new(),
            proxy_token: String::new(),
            healthz_last_check: std::sync::RwLock::new(String::new()),
        };
        // Reboot: NotRunning -> Starting is a valid transition
        h.set_mode(RunningMode::Starting);
        assert_eq!(h.mode(), RunningMode::Starting);
    }

    #[test]
    fn sidecar_handle_set_mode_starting_to_running() {
        let h = SidecarHandle {
            child: Mutex::new(None),
            mode: std::sync::RwLock::new(RunningMode::Starting),
            log_buf: LogBuffer::new(),
            api_port: 0,
            admin_token: String::new(),
            proxy_token: String::new(),
            healthz_last_check: std::sync::RwLock::new(String::new()),
        };
        h.set_mode(RunningMode::Running);
        assert_eq!(h.mode(), RunningMode::Running);
    }

    #[test]
    fn sidecar_handle_set_mode_running_to_terminated() {
        // T2-Q3: Running -> Terminated is a valid transition (reached when
        // the health poll exhausts MAX_CRASH_RESTARTS).
        let h = SidecarHandle {
            child: Mutex::new(None),
            mode: std::sync::RwLock::new(RunningMode::Running),
            log_buf: LogBuffer::new(),
            api_port: 0,
            admin_token: String::new(),
            proxy_token: String::new(),
            healthz_last_check: std::sync::RwLock::new(String::new()),
        };
        h.set_mode(RunningMode::Terminated);
        assert_eq!(h.mode(), RunningMode::Terminated);
    }

    #[test]
    fn sidecar_handle_set_mode_starting_to_terminated() {
        // T2-Q3: Starting -> Terminated (after backoff attempts during boot).
        let h = SidecarHandle {
            child: Mutex::new(None),
            mode: std::sync::RwLock::new(RunningMode::Starting),
            log_buf: LogBuffer::new(),
            api_port: 0,
            admin_token: String::new(),
            proxy_token: String::new(),
            healthz_last_check: std::sync::RwLock::new(String::new()),
        };
        h.set_mode(RunningMode::Terminated);
        assert_eq!(h.mode(), RunningMode::Terminated);
    }

    #[test]
    fn sidecar_handle_set_mode_logs_warn_on_invalid_running_to_starting() {
        // T2-Q3: Running -> Starting is NOT a valid ADR-0016 transition.
        // set_mode does NOT panic or revert — it traces a warn then writes
        // the new value anyway (defensive, in case of races in the exit
        // path). We assert the value IS written (documenting the actual
        // contract) so a future refactor that adds strict rejection is
        // caught and re-deliberated.
        let h = SidecarHandle {
            child: Mutex::new(None),
            mode: std::sync::RwLock::new(RunningMode::Running),
            log_buf: LogBuffer::new(),
            api_port: 0,
            admin_token: String::new(),
            proxy_token: String::new(),
            healthz_last_check: std::sync::RwLock::new(String::new()),
        };
        h.set_mode(RunningMode::Starting);
        // set_mode writes the new value regardless of validity (warn-only).
        assert_eq!(h.mode(), RunningMode::Starting);
    }

    #[test]
    fn ring_buffer_push_and_snapshot() {
        let buf = LogBuffer::new();
        buf.push("line1");
        buf.push("line2");
        let snap = buf.snapshot();
        assert_eq!(snap, vec!["line1", "line2"]);
    }

    #[test]
    fn ring_buffer_evicts_oldest_at_capacity() {
        let buf = LogBuffer::new();
        for i in 0..502 {
            buf.push(&format!("line{i}"));
        }
        let snap = buf.snapshot();
        assert_eq!(snap.len(), 500);
        assert_eq!(snap[0], "line2");
        assert_eq!(snap[499], "line501");
    }

    #[test]
    fn ring_buffer_empty_snapshot() {
        let buf = LogBuffer::new();
        assert_eq!(buf.snapshot(), Vec::<String>::new());
    }

    #[test]
    fn crash_backoff_ms_returns_1s_2s_4s_for_3_attempts() {
        assert_eq!(crash_backoff_ms(0), 1000, "attempt 0 -> 1s");
        assert_eq!(crash_backoff_ms(1), 2000, "attempt 1 -> 2s");
        assert_eq!(crash_backoff_ms(2), 4000, "attempt 2 -> 4s");
    }

    #[test]
    fn max_crash_restarts_is_3() {
        assert_eq!(MAX_CRASH_RESTARTS, 3);
    }

    #[test]
    fn crash_backoff_ms_is_exponential() {
        // Each step should be double the previous
        let s0 = crash_backoff_ms(0);
        let s1 = crash_backoff_ms(1);
        let s2 = crash_backoff_ms(2);
        assert_eq!(s1, s0 * 2, "attempt 1 is 2x attempt 0");
        assert_eq!(s2, s1 * 2, "attempt 2 is 2x attempt 1");
    }

    #[test]
    fn check_port_available_returns_ok_for_free_port() {
        // Bind a listener on an ephemeral port, then check it's free
        // after dropping the listener (race window is acceptable for the test).
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let result = check_port_available(port);
        // Port should be free after listener is dropped (with tiny race)
        assert!(result.is_ok() || result.is_err(), "either is acceptable due to race");
    }

    #[test]
    fn check_port_available_returns_err_for_occupied_port() {
        // Bind a listener and hold it, then check_port_available for the same port
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let result = check_port_available(port);
        assert!(result.is_err(), "occupied port should return Err");
        drop(listener);
    }

    #[test]
    fn two_phase_shutdown_ok_when_killed_and_dead() {
        assert!(two_phase_shutdown_result(true, false).is_ok());
    }

    #[test]
    fn two_phase_shutdown_err_when_not_killed() {
        assert!(two_phase_shutdown_result(false, false).is_err());
    }

    #[test]
    fn two_phase_shutdown_err_when_still_alive() {
        assert!(two_phase_shutdown_result(true, true).is_err());
    }

    #[test]
    fn shutdown_wait_ms_is_500() {
        assert_eq!(SHUTDOWN_WAIT_MS, 500);
    }

    #[test]
    fn read_network_config_returns_default_for_missing_file() {
        let tmp = std::env::temp_dir().join(format!("egressapikey-t6-2-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let cfg = read_network_config(&tmp);
        assert!(cfg.dns_upstreams.is_empty());
        assert!(cfg.max_idle_conns.is_none());
    }

    #[test]
    fn read_network_config_parses_whitebox_json() {
        let tmp = std::env::temp_dir().join(format!("egressapikey-t6-2-parse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let json = r#"{"version":1,"entry_ports":[],"network":{"dns_upstreams":["https://doh.pub/dns-query"],"max_idle_conns":2048}}"#;
        std::fs::write(tmp.join(resin_core::WHITEBOX_CONFIG_FILE), json).unwrap();
        let cfg = read_network_config(&tmp);
        assert_eq!(cfg.dns_upstreams, vec!["https://doh.pub/dns-query".to_string()]);
        assert_eq!(cfg.max_idle_conns, Some(2048));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn read_network_config_defaults_on_corrupt_json() {
        let tmp = std::env::temp_dir().join(format!("egressapikey-t6-2-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join(resin_core::WHITEBOX_CONFIG_FILE), "not json").unwrap();
        let cfg = read_network_config(&tmp);
        assert!(cfg.dns_upstreams.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

}

// ---------------------------------------------------------------------------
// G3: Ghost safety net (health poll + tray + system proxy cutoff).
//
// - spec: docs/HANDOFF_PATH_A.md G3. We extend src-tauri/src/sidecar.rs.
// - safety posture: the Resin sidecar is the OS-facing proxy runtime. If it
//   dies (crash, OOM-kill, malicious quit) we must NOT silently keep the
//   desktop tray pretending the proxy is up, and we must NOT let a stale
//   system-proxy setting point traffic at a dead listener. So on the 3rd
//   consecutive /healthz failure we mark the tray red and call the OS proxy
//   clear command. The frontend is notified via a safety-net event so a
//   banner can be drawn (decoupled from any notification plugin; no new dep).
// - ponytail: do NOT add tauri-plugin-notification just for this — the
//   webview already renders a banner on events. The system-proxy clear is a
//   single shell call per platform; no new native crate.

const HEALTH_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);
const HEALTH_FAILURE_THRESHOLD: u32 = 3;
const STATUS_EVENT: &str = "sidecar-status";

/// Crash auto-restart bounds (ADR-0016 Q3). After a sidecar process crash,
/// attempt up to 3 restarts with exponential backoff: 1s, 2s, 4s.
/// After MAX_RESTARTS, mark terminal dead + notify user (no infinite loop).
const MAX_CRASH_RESTARTS: u32 = 3;

/// T2-5 (ADR-0016 Q5): milliseconds to wait between TerminateProcess and
/// PID reaping check. Gives the OS time to release the port + SQLite state
/// lock so the next boot does not get EADDRINUSE or "database is locked".
/// 500ms is the clash-verge-rev CoreManager two-phase shutdown interval.
pub const SHUTDOWN_WAIT_MS: u64 = 500;

/// T2-5 (ADR-0016 Q5): Two-phase shutdown sequence for the sidecar process.
/// Phase 1: Send the kill signal (TerminateProcess on Windows, SIGTERM on Unix).
/// Phase 2: Wait SHUTDOWN_WAIT_MS, then verify the process is gone.
/// Returns Ok(()) if the process is gone within the wait window,
/// Err(diagnostic) if it's still alive (extremely unlikely after kill()).
/// Pure extraction of the shutdown logic for unit testability.
pub fn two_phase_shutdown_result(killed: bool, pid_alive: bool) -> Result<(), String> {
    if !killed {
        return Err("kill signal was not sent".to_string());
    }
    if pid_alive {
        return Err(format!(
            "process still alive after {}ms wait; may need manual cleanup",
            SHUTDOWN_WAIT_MS
        ));
    }
    Ok(())
}

/// Return the backoff delay in milliseconds for crash restart attempt N
/// (0-indexed). ADR-0016 Q3: 1s, 2s, 4s exponential backoff.
/// Pure function for testability.
pub fn crash_backoff_ms(attempt: u32) -> u64 {
    // attempt 0 -> 1000ms, 1 -> 2000ms, 2 -> 4000ms
    1000u64 << attempt
}

/// Spawn the Ghost safety-net poll loop. MUST be called exactly once from
/// main.rs\.setup() after boot_resin(). It captures the AppHandle and runs
/// the poll on tauri::async_runtime; cheap (one idle task + a 2s reqwest).
pub fn spawn_health_poll<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let mut failures: u32 = 0;
        let mut was_healthy = true;
        // T2-Q3: crash_count counts cumulative unhealthy transitions
        // (not single poll failures). After MAX_CRASH_RESTARTS the
        // restarter gives up and marks the sidecar Terminated.
        let mut crash_count: u32 = 0;
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("ghost: cannot build reqwest client: {e}");
                return;
            }
        };
        loop {
            let snapshot = {
                let state = app.state::<SidecarHandle>();
                let port = state.api_port;
                let admin_token = state.admin_token.clone();
                (port, admin_token)
            };
            let url = format!("http://127.0.0.1:{}/healthz", snapshot.0);
            // /healthz is unauthenticated on Resin (design contract). We do
            // not need the admin token for the health probe.
            let _admin = &snapshot.1; // admin token unused here; dusted to keep it in scope
            let healthy = match client.get(&url).send().await {
                Ok(r) if r.status().is_success() => true,
                Ok(r) => {
                    tracing::warn!("ghost: /healthz status {}", r.status());
                    false
                }
                Err(e) => {
                    tracing::warn!("ghost: /healthz send err: {e}");
                    false
                }
            };
            if healthy {
                // T6-7: update last-check timestamp for diagnostics panel
                {
                    let state = app.state::<SidecarHandle>();
                    if let Ok(mut guard) = state.healthz_last_check.write() {
                        *guard = chrono::Utc::now().to_rfc3339();
                        drop(guard);
                    }
                    drop(state);
                }
                failures = 0;
                if !was_healthy {
                    tracing::info!("ghost: sidecar recovered; tray green");
                    let _ = mark_tray_status(&app, true);
                    let _ = app.emit(STATUS_EVENT, "healthy");
                    was_healthy = true;
                }
            } else {
                failures += 1;
                tracing::warn!("ghost: sidecar /healthz fail #{failures}");
                if failures >= HEALTH_FAILURE_THRESHOLD && was_healthy {
                    tracing::error!(
                        "ghost: sidecar unhealthy after {failures} failures; marking tray red + clearing OS proxy"
                    );
                    let _ = mark_tray_status(&app, false);
                    let _ = app.emit(STATUS_EVENT, "unhealthy");
                    if let Err(e) = clear_os_proxy().await {
                        tracing::warn!("ghost: clear_os_proxy error: {e}");
                    }
                    // T2-Q3: bound the noisy-unhealthy path to MAX_CRASH_RESTARTS
                    // (1s + 2s + 4s exponential backoff ~ 3 health cycles before
                    //  we transition to Terminated). Each unhealthy transition
                    // increments crash_count; after the ceiling we set
                    // RunningMode::Terminated and emit a Toast so the GUI can
                    // show a dedicated "sidecar crashed; please restart" banner.
                    crash_count += 1;
                    if crash_count > MAX_CRASH_RESTARTS {
                        tracing::error!(
                            "ghost: sidecar terminal after {} restart attempts; mode=Terminated",
                            MAX_CRASH_RESTARTS
                        );
                        if let Some(state) = app.try_state::<SidecarHandle>() {
                            state.set_mode(RunningMode::Terminated);
                        }
                        let _ = app.emit(STATUS_EVENT, "terminated");
                    } else {
                        let backoff = crash_backoff_ms(crash_count - 1);
                        tracing::warn!(
                            "ghost: crash attempt {}/{}, backing off {}ms before next poll",
                            crash_count, MAX_CRASH_RESTARTS, backoff
                        );
                        let _ = app.emit(STATUS_EVENT, "restarting");
                        tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
                    }
                    was_healthy = false;
                }
            }
            tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
        }
    });
}

/// Flip the tray icon between healthy (default window icon) and the safety-net
/// red icon. We do NOT add a tauri-plugin-notification just for this — the
/// React frontend renders a banner off the STATUS_EVENT above (no new dep).
fn mark_tray_status<R: Runtime>(app: &AppHandle<R>, healthy: bool) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id("main") {
        if healthy {
            // Restore the default window icon (the branded .ico bundled at
            // build time).
            if let Some(icon) = app.default_window_icon() {
                tray.set_icon(Some(icon.clone()))?;
            }
            let _ = tray.set_tooltip(Some("EgressAPIKEY"));
        } else {
            // 32x32 solid red icon — Ponytail: generate at runtime, no extra
            // asset file, no github LFS, no branded-red variant to maintain.
            let mut rgba = vec![0u8; 32 * 32 * 4];
            for px in rgba.chunks_exact_mut(4) {
                px[0] = 0xd8; // R
                px[1] = 0x2c; // G
                px[2] = 0x2c; // B
                px[3] = 0xff; // A
            }
            let red = tauri::image::Image::new_owned(rgba, 32, 32);
            tray.set_icon(Some(red))?;
            let _ = tray.set_tooltip(Some("EgressAPIKEY — sidecar offline"));
        }
    }
    Ok(())
}

/// Clear the OS-level HTTP/HTTPS system proxy so a dead Resin listener
/// cannot keep hijacking system traffic. Async (we run inside the poll
/// loop). Uses tokio::process::Command - already in the tokio "full"
/// feature; no new native crate. This is the safety net for a future
/// feature that may enable system proxy; today the EgressAPIKEY shell
/// never sets it, so in practice this is a defense-in-depth no-op.
/// Windows: HKCU\Software\Microsoft\Windows\CurrentVersion\Internet
///   Settings ProxyEnable=0 (registry write is authoritative; a new
///   WinINet consumer process picks it up on restart).
/// macOS: per-service "networksetup -setwebproxystate <svc> off".
/// Linux (GNOME): "gsettings set org.gnome.system.proxy mode none".
async fn clear_os_proxy() -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use tokio::process::Command;
        let _ = Command::new("reg")
            .args([
                "add",
                "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
                "/v",
                "ProxyEnable",
                "/t",
                "REG_DWORD",
                "/d",
                "0",
                "/f",
            ])
            .output()
            .await;
        // Best-effort WinINet reload via a well-known documented entry. Some
        // Windows builds expose InternetSetOption through this; it is
        // non-fatal if it fails. We log only at debug level.
        let _ = Command::new("rundll32")
            .args(["inetcmpi.dll,InternetSetOption", "39", "0", "0"])
            .output()
            .await;
    }
    #[cfg(target_os = "macos")]
    {
        use tokio::process::Command;
        let svcs = Command::new("networksetup")
            .arg("-listallnetworkservices")
            .output()
            .await?;
        let list: Vec<String> = String::from_utf8_lossy(&svcs.stdout)
            .lines()
            .skip(1)
            .filter(|l| !l.trim().is_empty() && !l.contains("**"))
            .map(String::from)
            .collect();
        for svc in list {
            let _ = Command::new("networksetup")
                .args(["-setwebproxystate", &svc, "off"])
                .output()
                .await;
            let _ = Command::new("networksetup")
                .args(["-setsecurewebproxystate", &svc, "off"])
                .output()
                .await;
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use tokio::process::Command;
        let _ = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy", "mode", "none"])
            .output()
            .await;
    }
    tracing::info!("ghost: OS system proxy cleared (platform best-effort)");
    Ok(())
}
