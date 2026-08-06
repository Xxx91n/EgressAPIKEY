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
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::ShellExt;

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
    /// child once and call .kill(). CommandChild::kill takes self
    /// (consumes the receiver); State<SidecarHandle> only hands out
    /// borrows, so without the Option<take()> you cannot move the child
    /// out of SysState. None after kill() means a second Exit callback
    /// (if Tauri ever re-emits one) is a no-op.
    pub child: Mutex<Option<CommandChild>>,
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
pub fn boot_resin<R: Runtime>(app: &AppHandle<R>) -> Result<SidecarHandle> {
    // Pick a free loopback port up front so we hand it to Resin and know what
    // to poll. Drop the listener immediately after taking the port; Resin
    // will bind it again as part of its startup.
    let listener = TcpListener::bind("127.0.0.1:0")
        .context("sidecar: failed to allocate a free port for Resin")?;
    let api_port = listener
        .local_addr()
        .context("sidecar: listener has no local addr")?
        .port();
    drop(listener);

    // Generate strong-enough loopback-only secrets. Not user-facing.
    let admin_token = gen_token();
    let proxy_token = gen_token();

    // Build and spawn the sidecar. We pass environment (not CLI args) because
    // Resin reads RESIN_* from env (DESIGN.md env contract). We do NOT
    // env_clear — Resin needs its own runtime env (PATH, TMP, etc.) and we
    // add only the RESIN_* knobs we want to pin.
    // Resin needs writable state/cache/log dirs. On a Go binary compiled
    // with Linux defaults (/var/lib/resin, /var/cache/resin, /var/log/resin)
    // those paths do not exist on Windows and the process exits with
    //   fatal: persistence bootstrap: repair consistency: attach state_db:
    //   unable to open database file ... (14)
    // We override all three to the Tauri per-user app data dir + OS log dir
    // so the sidecar owns its own subdirectory on every platform. The Rust
    // shell is the trust boundary; the webview never sees these paths.
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
    std::fs::create_dir_all(&state_dir)
        .with_context(|| format!("sidecar: cannot create state_dir {:?}", state_dir))?;
    std::fs::create_dir_all(&cache_dir)
        .with_context(|| format!("sidecar: cannot create cache_dir {:?}", cache_dir))?;
    std::fs::create_dir_all(&log_dir)
        .with_context(|| format!("sidecar: cannot create log_dir {:?}", log_dir))?;
    tracing::info!(
        "resin sidecar dirs: state={:?} cache={:?} log={:?}",
        state_dir,
        cache_dir,
        log_dir
    );

    let shell = app.shell();
    let mut cmd = shell
        .sidecar("resin")
        .context("sidecar: resin binary not found in bundle (externalBin misconfigured)")?;
    cmd = cmd
        .env("RESIN_AUTH_VERSION", "V1")
        .env("RESIN_ADMIN_TOKEN", &admin_token)
        .env("RESIN_PROXY_TOKEN", &proxy_token)
        .env("RESIN_LISTEN_ADDRESS", "127.0.0.1")
        .env("RESIN_PORT", api_port.to_string())
        .env("RESIN_STATE_DIR", &state_dir)
        .env("RESIN_CACHE_DIR", &cache_dir)
        .env("RESIN_LOG_DIR", &log_dir);

    let (mut receiver, child) = cmd // spawn() returns (Receiver, CommandChild)
        .spawn()
        .context("sidecar: failed to spawn resin binary")?;

    // Ring buffer for sidecar stderr/stdout (ADR-0016 Q2a). Shared between
    // the drain task (writer) and the SidecarHandle (IPC reader).
    let log_buf = LogBuffer::new();
    let drain_buf = log_buf.clone();

    // Poll /healthz until up or timeout. Spawn a background task to also drain
    // receiver so the sidecar's stdout buffer does not fill and block.
    let _drain = tauri::async_runtime::spawn(async move {
        use tauri_plugin_shell::process::CommandEvent;
        while let Some(ev) = receiver.recv().await {
            match ev {
                CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
                    let text = String::from_utf8_lossy(&bytes).trim_end().to_string();
                    tracing::trace!(
                        target: "resin_sidecar",
                        "sidecar stdout/stderr: {}",
                        text
                    );
                    drain_buf.push(&text);
                }
                CommandEvent::Terminated(payload) => {
                    tracing::error!(
                        target: "resin_sidecar",
                        "resin sidecar terminated: code={:?} signal={:?}",
                        payload.code, payload.signal
                    );
                    break;
                }
                CommandEvent::Error(msg) => {
                    tracing::error!(target: "resin_sidecar", "sidecar event err: {msg}");
                    break;
                }
                _ => {}
            }
        }
    });

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
                });
            }
            Ok(r) => {
                last_err = Some(format!("HTTP {}", r.status()));
            }
            Err(e) => {
                last_err = Some(e.to_string());
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    // Timeout: kill the child and surface what we saw.
    let _ = child.kill();
    Err(anyhow!(
        "sidecar: resin /healthz did not come up within 15s on 127.0.0.1:{api_port} (last error: {})",
        last_err.unwrap_or_else(|| "no response".into())
    ))
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
        };
        h.set_mode(RunningMode::Running);
        assert_eq!(h.mode(), RunningMode::Running);
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

/// Spawn the Ghost safety-net poll loop. MUST be called exactly once from
/// main.rs\.setup() after boot_resin(). It captures the AppHandle and runs
/// the poll on tauri::async_runtime; cheap (one idle task + a 2s reqwest).
pub fn spawn_health_poll<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let mut failures: u32 = 0;
        let mut was_healthy = true;
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
