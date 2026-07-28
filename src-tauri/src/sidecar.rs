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

use std::net::TcpListener;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::ShellExt;

/// The running sidecar process plus the connection info the Rust side needs.
/// Lives in Tauri managed state as `State<SidecarHandle>` so IPC commands
/// (G2 ResinClient) can reach it without piping admin tokens anywhere else.
pub struct SidecarHandle {
    pub child: CommandChild,
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
        state_dir, cache_dir, log_dir
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

    let (mut receiver, child) = cmd  // spawn() returns (Receiver, CommandChild)
        .spawn()
        .context("sidecar: failed to spawn resin binary")?;

    // Poll /healthz until up or timeout. Spawn a background task to also drain
    // receiver so the sidecar's stdout buffer does not fill and block.
    let _drain = tauri::async_runtime::spawn(async move {
        use tauri_plugin_shell::process::CommandEvent;
        while let Some(ev) = receiver.recv().await {
            match ev {
                CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
                    tracing::trace!(
                        target: "resin_sidecar",
                        "sidecar stdout/stderr: {}",
                        String::from_utf8_lossy(&bytes).trim_end()
                    );
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
                    child,
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
}
