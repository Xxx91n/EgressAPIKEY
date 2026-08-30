//! diagnostics domain IPC commands (EgressAPIKEY).
//!
//! Extracted from the former commands/mod.rs monolith by architecture-recovery
//! ticket 08: pure mechanical move - no behavior, naming, or IPC-surface change.
use serde::{Serialize};
use tauri::{State};
use crate::sidecar::SidecarHandle;
use resin_core::DbPool;
use resin_core::IpcError;
use super::common::{map_resin_error, resin_client};
use super::ports::{validate_port_segments};

/// T2-2 (ADR-0016 Q2b): IPC snapshot of the sidecar stderr/stdout ring
/// buffer. Returns the last N lines (oldest still in buffer first) for the
/// Settings > Logs view. Read-only; no input from the webview.
#[tauri::command]
pub fn get_sidecar_logs(sidecar: State<'_, SidecarHandle>) -> Result<Vec<String>, IpcError> {
    Ok(sidecar.log_buf.snapshot())
}

/// T3-A1 (ADR-0012 deep audit): Expose the Resin sidecar's actual runtime
/// port + health status to the frontend as read-only data.
#[derive(serde::Serialize)]
pub struct SidecarStatus {
    pub api_port: u16,
    pub api_base: String,
    pub mode: String,
    /// T6-7: sidecar process PID (0 if not running).
    pub pid: u32,
    /// T6-7: RFC3339 timestamp of the last successful /healthz probe.
    pub healthz_last_check: String,
    /// T6-7: round-trip latency of the get_sidecar_status IPC call (microseconds).
    pub ipc_latency_us: u64,
}

#[tauri::command]
pub fn get_sidecar_status(sidecar: State<'_, SidecarHandle>) -> Result<SidecarStatus, IpcError> {
    let started = std::time::Instant::now();
    let mode = sidecar.mode.read().map(|m| format!("{:?}", *m)).unwrap_or_else(|_| "Unknown".to_string());
    // T6-7: extract PID from the child process
    let pid = sidecar.child.lock().map(|c| {
        c.as_ref().map(|child| child.id()).unwrap_or(0)
    }).unwrap_or(0);
    // T6-7: last healthz check timestamp
    let healthz_last_check = sidecar.healthz_last_check.read()
        .map(|g| g.clone())
        .unwrap_or_default();
    let ipc_latency_us = started.elapsed().as_micros() as u64;
    Ok(SidecarStatus {
        api_port: sidecar.api_port,
        api_base: sidecar.api_base(),
        mode,
        pid,
        healthz_last_check,
        ipc_latency_us,
    })
}

/// T6-5 (rewritten by architecture-recovery ticket 11): Read the last N request
/// log entries via the Resin admin REST API (GET /api/v1/request-logs?limit=N).
/// The previous implementation scanned Resin's private request_logs*.db files
/// (temp-copy + read-only rusqlite query) — that bypassed the ResinClient REST
/// seam; Resin v1.2.0 (docs/RESIN_UPSTREAM_MANIFEST.yaml) exposes the official
/// /api/v1/request-logs endpoint (ADR-0005 Q6: probe-before-code satisfied),
/// so the shell no longer touches Resin's state dir.
/// Response shape from Resin v1.2.0 (verified in the bundled sidecar binary):
/// {"items":[{ts_ns, platform_name, account, target_host, egress_ip,
///   proxy_type, net_ok, http_method, http_status, duration_ns, resin_error}],
///  "cursor": "...", "total": N}, rows ordered by ts_ns DESC.
/// Unknown item fields are tolerated via Value::get; if Resin ever changes the
/// wire shape this returns explicit IpcErrors instead of silently misparsing.
#[derive(Debug, Serialize, Clone)]
pub struct RequestLogEntry {
    pub ts: String,
    pub platform_name: String,
    pub account: String,
    pub target_host: String,
    pub egress_ip: String,
    pub http_method: String,
    pub http_status: i64,
    pub duration_ms: f64,
    pub resin_error: String,
}

/// Format a Resin request-log ts_ns (epoch nanos) as "YYYY-MM-DD HH:MM:SS"
/// UTC. Exposed for unit tests; kept total (falls back to "") like the
/// previous chrono path.
pub fn format_log_ts(ts_ns: i64) -> String {
    let secs = ts_ns / 1_000_000_000;
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

/// Project one item of the Resin /api/v1/request-logs response onto
/// RequestLogEntry. Total over a present-but-malformed field so one bad row
/// never blanks the whole tail (parity with the old row-map semantics).
pub fn request_log_entry_from_value(v: &serde_json::Value) -> RequestLogEntry {
    let ts_ns = v.get("ts_ns").and_then(|x| x.as_i64()).unwrap_or(0);
    let duration_ns = v.get("duration_ns").and_then(|x| x.as_i64()).unwrap_or(0);
    let s = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    };
    RequestLogEntry {
        ts: format_log_ts(ts_ns),
        platform_name: s("platform_name"),
        account: s("account"),
        target_host: s("target_host"),
        egress_ip: s("egress_ip"),
        http_method: s("http_method"),
        http_status: v.get("http_status").and_then(|x| x.as_i64()).unwrap_or(0),
        duration_ms: duration_ns as f64 / 1_000_000.0,
        resin_error: s("resin_error"),
    }
}

#[tauri::command]
pub async fn request_log_tail(
    sidecar: State<'_, SidecarHandle>,
    limit: Option<usize>,
) -> Result<Vec<RequestLogEntry>, IpcError> {
    // ADR-0045: clamp the caller-controlled page size at the IPC boundary.
    let n = limit.unwrap_or(50).min(200).max(1) as u32;
    let client = resin_client(&sidecar)?;
    let resp = client
        .request_logs(n, None)
        .await
        .map_err(|e| map_resin_error(&e.to_string()))?;
    // Resin wraps list endpoints as {items: [...], cursor, total}; a missing or
    // non-array items field is a wire-shape regression, not an empty log.
    let items = resp
        .get("items")
        .and_then(|i| i.as_array())
        .ok_or_else(|| {
            IpcError::internal("resin /api/v1/request-logs: response has no items array")
        })?;
    let entries: Vec<RequestLogEntry> = items.iter().map(request_log_entry_from_value).collect();
    tracing::info!(count = entries.len(), "request_log_tail: read entries via REST");
    Ok(entries)
}

/// T6-5: Check Windows firewall inbound allow status for Resin's listen ports.
/// Read-only: runs `Get-NetFirewallProfile` to check if firewall is on.
#[derive(Debug, Serialize, Clone)]
pub struct FirewallStatus {
    pub platform: String,
    pub firewall_on: bool,
    pub inbound_blocked: bool,
    pub detail: String,
}

#[tauri::command]
pub async fn check_firewall_status() -> Result<FirewallStatus, IpcError> {
    // T7-1: enterprise-grade subprocess spawn - tokio::process::Command +
    // CREATE_NO_WINDOW on Windows + 5s timeout to prevent deadlock and
    // console window flash. Cross-platform: Linux uses systemctl, macOS
    // uses pfctl. Pattern from pwm gpt56_sol research.
    #[cfg(target_os = "windows")]
    {
        use tokio::process::Command;

        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-Command",
            "Get-NetFirewallProfile | Select-Object Name, Enabled | ConvertTo-Json"]);
        cmd.creation_flags(0x08000000u32); // CREATE_NO_WINDOW

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            cmd.output(),
        )
        .await
        .map_err(|_| IpcError::internal("firewall check timed out (5s)"))?
        .map_err(|e| IpcError::internal(&format!("firewall check: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let firewall_on = stdout.contains("true");
        tracing::info!(firewall_on, "check_firewall_status: probed");
        Ok(FirewallStatus {
            platform: "windows".into(),
            firewall_on,
            inbound_blocked: firewall_on,
            detail: if firewall_on {
                "Windows Firewall is ON. If ports are unreachable, add an inbound rule.".into()
            } else {
                "Windows Firewall is OFF.".into()
            },
        })
    }
    #[cfg(target_os = "linux")]
    {
        use tokio::process::Command;

        // Non-root best-effort: systemctl is-active (distro-dependent),
        // fallback to /proc/net/ip_tables_names. pwm sonar: ufw/iptables need root.
        async fn try_detect() -> Option<FirewallStatus> {
            let out = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                Command::new("systemctl").args(["is-active", "ufw", "--quiet"]).output(),
            ).await.ok()?.ok()?;
            if String::from_utf8_lossy(&out.stdout).trim() == "active" {
                return Some(FirewallStatus { platform: "linux".into(), firewall_on: true, inbound_blocked: true, detail: "UFW firewall is active.".into() });
            }
            let out = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                Command::new("systemctl").args(["is-active", "firewalld", "--quiet"]).output(),
            ).await.ok()?.ok()?;
            if String::from_utf8_lossy(&out.stdout).trim() == "active" {
                return Some(FirewallStatus { platform: "linux".into(), firewall_on: true, inbound_blocked: true, detail: "firewalld is active.".into() });
            }
            if std::path::Path::new("/proc/net/ip_tables_names").exists() {
                return Some(FirewallStatus { platform: "linux".into(), firewall_on: true, inbound_blocked: true, detail: "iptables tables detected.".into() });
            }
            None
        }
        match try_detect().await {
            Some(status) => {
                tracing::info!(firewall_on = status.firewall_on, "check_firewall_status: probed linux");
                Ok(status)
            }
            None => Ok(FirewallStatus { platform: "linux".into(), firewall_on: false, inbound_blocked: false, detail: "No firewall detected (or insufficient permissions).".into() }),
        }
    }
    #[cfg(target_os = "macos")]
    {
        use tokio::process::Command;

        let mut cmd = Command::new("pfctl");
        cmd.args(["-s", "info"]);
        match tokio::time::timeout(std::time::Duration::from_secs(5), cmd.output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let firewall_on = stdout.contains("enabled");
                tracing::info!(firewall_on, "check_firewall_status: probed macos pfctl");
                Ok(FirewallStatus { platform: "macos".into(), firewall_on, inbound_blocked: firewall_on, detail: if firewall_on { "pf firewall is enabled.".into() } else { "pf firewall appears disabled.".into() } })
            }
            _ => {
                let pf_conf_exists = std::path::Path::new("/etc/pf.conf").exists();
                Ok(FirewallStatus { platform: "macos".into(), firewall_on: pf_conf_exists, inbound_blocked: pf_conf_exists, detail: if pf_conf_exists { "/etc/pf.conf exists but status uncertain (pfctl needs root).".into() } else { "No pf.conf found; firewall likely disabled.".into() } })
            }
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        Ok(FirewallStatus { platform: std::env::consts::OS.into(), firewall_on: false, inbound_blocked: false, detail: "Firewall check not supported on this platform.".into() })
    }
}

/// T6-4: Probe the exit IP by routing a request to http://1.1.1.1/cdn-cgi/trace
/// through the specified entry port (HTTP or SOCKS5 proxy). Returns the exit
/// IP parsed from the Cloudflare trace body, plus latency. If no active
/// subscription/nodes are available, returns an error so the GUI can show
/// "no exit IP available" rather than a misleading blank.
#[tauri::command]
pub async fn probe_exit_ip(
    sidecar: State<'_, SidecarHandle>,
    db: State<'_, DbPool>,
    port: u16,
    protocol: String,
) -> Result<ExitIpProbe, IpcError> {
    tracing::info!(port, protocol = %protocol, "probe_exit_ip: probing through proxy");
    validate_port_segments(port)?;
    let proto = protocol.to_ascii_lowercase();
    if proto != "http" && proto != "socks5" {
        return Err(IpcError::internal("error.invalidProtocol"));
    }
    let proxy_url = if proto == "http" {
        format!("http://127.0.0.1:{port}")
    } else {
        let mapping = db
            .list_ports()?
            .into_iter()
            .find(|m| m.port == port)
            .ok_or_else(|| format!("port {port} is not configured"))?;
        let username = if mapping.account.trim().is_empty() {
            format!("{}.port-{}", mapping.platform_name, port)
        } else {
            format!("{}.{}", mapping.platform_name, mapping.account)
        };
        let password = &sidecar.proxy_token;
        format!("socks5h://{username}:{password}@127.0.0.1:{port}")
    };
    let proxy = reqwest::Proxy::all(&proxy_url)
        .map_err(|e| IpcError::internal(&format!("proxy build: {e}")))?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| IpcError::internal(&format!("client build: {e}")))?;
    let started = std::time::Instant::now();
    let resp = client
        .get("http://1.1.1.1/cdn-cgi/trace")
        .send()
        .await
        .map_err(|e| IpcError::internal(&format!("probe request: {e}")))?;
    let status = resp.status().as_u16();
    let body = resp
        .text()
        .await
        .map_err(|e| IpcError::internal(&format!("probe body: {e}")))?;
    let latency_ms = started.elapsed().as_millis() as u64;
    let exit_ip = resin_core::parse_trace_body_ip(&body);
    tracing::info!(port, protocol = %proto, exit_ip = %exit_ip, latency_ms, "probe_exit_ip: success");
    Ok(ExitIpProbe {
        port,
        protocol: proto,
        exit_ip,
        latency_ms,
        status,
    })
}

#[derive(Debug, Serialize, Clone)]
pub struct ExitIpProbe {
    pub port: u16,
    pub protocol: String,
    pub exit_ip: String,
    pub latency_ms: u64,
    pub status: u16,
}
