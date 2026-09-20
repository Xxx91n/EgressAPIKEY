//! diagnostics domain IPC commands (EgressAPIKEY).
//!
//! Extracted from the former commands/mod.rs monolith by
//! pure mechanical move - no behavior, naming, or IPC-surface change.
use super::common::{map_resin_error, resin_client};
use super::ports::validate_port_segments;
use crate::sidecar::SidecarHandle;
use resin_core::DbPool;
use resin_core::IpcError;
use serde::Serialize;
use tauri::State;

/// (ADR-0016 b): IPC snapshot of the sidecar stderr/stdout ring
/// buffer. Returns the last N lines (oldest still in buffer first) for the
/// Settings > Logs view. Read-only; no input from the webview.
#[tauri::command]
pub fn get_sidecar_logs(sidecar: State<'_, SidecarHandle>) -> Result<Vec<String>, IpcError> {
    Ok(sidecar.log_buf.snapshot())
}

/// (ADR-0012 deep audit): Expose the Resin sidecar's actual runtime
/// port + health status to the frontend as read-only data.
#[derive(serde::Serialize)]
pub struct SidecarStatus {
    pub api_port: u16,
    pub api_base: String,
    pub mode: String,
    /// sidecar process PID (0 if not running).
    pub pid: u32,
    /// RFC3339 timestamp of the last successful /healthz probe.
    pub healthz_last_check: String,
    /// round-trip latency of the get_sidecar_status IPC call (microseconds).
    pub ipc_latency_us: u64,
}

#[tauri::command]
pub fn get_sidecar_status(sidecar: State<'_, SidecarHandle>) -> Result<SidecarStatus, IpcError> {
    Ok(sidecar_status_of(&sidecar))
}

/// Transport-free status read: the headless BFF owns a SidecarHandle
/// too, so both transports read the same fields.
pub fn sidecar_status_of(sidecar: &SidecarHandle) -> SidecarStatus {
    let started = std::time::Instant::now();
    let mode = sidecar
        .mode
        .read()
        .map(|m| format!("{:?}", *m))
        .unwrap_or_else(|_| "Unknown".to_string());
    // extract PID from the child process
    let pid = sidecar
        .child
        .lock()
        .map(|c| c.as_ref().map(|child| child.id()).unwrap_or(0))
        .unwrap_or(0);
    // last healthz check timestamp
    let healthz_last_check = sidecar
        .healthz_last_check
        .read()
        .map(|g| g.clone())
        .unwrap_or_default();
    let ipc_latency_us = started.elapsed().as_micros() as u64;
    SidecarStatus {
        api_port: sidecar.api_port,
        api_base: sidecar.api_base(),
        mode,
        pid,
        healthz_last_check,
        ipc_latency_us,
    }
}

/// (rewritten by): Read the last N request
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
    /// Resin row UUID (requestlog/repo.go:145) — the detail drawer's
    /// key for GET /request-logs/{log_id}. Empty when the wire shape lacks
    /// it (older sidecar), which leaves the row inert.
    pub id: String,
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
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
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
        id: s("id"),
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
    tracing::info!(
        count = entries.len(),
        "request_log_tail: read entries via REST"
    );
    Ok(entries)
}

/// Check Windows firewall inbound allow status for Resin's listen ports.
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
    // enterprise-grade subprocess spawn - tokio::process::Command +
    // CREATE_NO_WINDOW on Windows + 5s timeout to prevent deadlock and
    // console window flash. Cross-platform: Linux uses systemctl, macOS
    // uses pfctl. Pattern from pwm gpt56_sol research.
    #[cfg(target_os = "windows")]
    {
        use tokio::process::Command;

        let mut cmd = Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-Command",
            "Get-NetFirewallProfile | Select-Object Name, Enabled | ConvertTo-Json",
        ]);
        cmd.creation_flags(0x08000000u32); // CREATE_NO_WINDOW

        let output = tokio::time::timeout(std::time::Duration::from_secs(5), cmd.output())
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
                Command::new("systemctl")
                    .args(["is-active", "ufw", "--quiet"])
                    .output(),
            )
            .await
            .ok()?
            .ok()?;
            if String::from_utf8_lossy(&out.stdout).trim() == "active" {
                return Some(FirewallStatus {
                    platform: "linux".into(),
                    firewall_on: true,
                    inbound_blocked: true,
                    detail: "UFW firewall is active.".into(),
                });
            }
            let out = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                Command::new("systemctl")
                    .args(["is-active", "firewalld", "--quiet"])
                    .output(),
            )
            .await
            .ok()?
            .ok()?;
            if String::from_utf8_lossy(&out.stdout).trim() == "active" {
                return Some(FirewallStatus {
                    platform: "linux".into(),
                    firewall_on: true,
                    inbound_blocked: true,
                    detail: "firewalld is active.".into(),
                });
            }
            if std::path::Path::new("/proc/net/ip_tables_names").exists() {
                return Some(FirewallStatus {
                    platform: "linux".into(),
                    firewall_on: true,
                    inbound_blocked: true,
                    detail: "iptables tables detected.".into(),
                });
            }
            None
        }
        match try_detect().await {
            Some(status) => {
                tracing::info!(
                    firewall_on = status.firewall_on,
                    "check_firewall_status: probed linux"
                );
                Ok(status)
            }
            None => Ok(FirewallStatus {
                platform: "linux".into(),
                firewall_on: false,
                inbound_blocked: false,
                detail: "No firewall detected (or insufficient permissions).".into(),
            }),
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
                Ok(FirewallStatus {
                    platform: "macos".into(),
                    firewall_on,
                    inbound_blocked: firewall_on,
                    detail: if firewall_on {
                        "pf firewall is enabled.".into()
                    } else {
                        "pf firewall appears disabled.".into()
                    },
                })
            }
            _ => {
                let pf_conf_exists = std::path::Path::new("/etc/pf.conf").exists();
                Ok(FirewallStatus {
                    platform: "macos".into(),
                    firewall_on: pf_conf_exists,
                    inbound_blocked: pf_conf_exists,
                    detail: if pf_conf_exists {
                        "/etc/pf.conf exists but status uncertain (pfctl needs root).".into()
                    } else {
                        "No pf.conf found; firewall likely disabled.".into()
                    },
                })
            }
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        Ok(FirewallStatus {
            platform: std::env::consts::OS.into(),
            firewall_on: false,
            inbound_blocked: false,
            detail: "Firewall check not supported on this platform.".into(),
        })
    }
}

/// Probe the exit IP by routing a request to http://1.1.1.1/cdn-cgi/trace
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
    probe_exit_ip_impl(&sidecar.proxy_token, &db, port, protocol).await
}

/// Transport-free body: the headless BFF holds the same proxy_token
/// + port DB, so the probe runs identically on either transport.
pub async fn probe_exit_ip_impl(
    proxy_token: &str,
    db: &DbPool,
    port: u16,
    protocol: String,
) -> Result<ExitIpProbe, IpcError> {
    tracing::info!(port, protocol = %protocol, "probe_exit_ip: probing through proxy");
    validate_port_segments(port)?;
    let proto = protocol.to_ascii_lowercase();
    // the closed three-value set. A `mixed` port
    // probes through the SOCKS5 dialect - a dual-flag listener accepts it,
    // which is what the ADR-0068 D4 gate observed live against Resin.
    if !resin_core::entry_protocol::is_valid_protocol(&proto) {
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
        let password = proxy_token;
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

// ── (ADR-0064): Resin metrics minimal set ─────────────────────
// Two endpoints of the 12 registered by resin/internal/api/handler_metrics.go
// (裂痕 #5): GET /metrics/realtime/throughput (#R47) and GET
// /metrics/history/probes (#R53). Pull model (spec: no WebSocket push); the
// Ghost G3 health poll is a separate channel and is untouched. The remaining
// 10 endpoints stay documented as blank in RESIN_API_COVERAGE.md pending a
// full-set round (issue F1/F3).

/// §7.5 input validation for the metrics history window (issue F4). Parsed +
/// bounded at the IPC boundary before Resin ever sees the query string —
/// the TS wrapper applies the same rules first (dual cover), this is the
/// second line. Rules:
///   - from/to parse as RFC3339 (handler_metrics.go:15 parses
///     time.RFC3339Nano; anything else is a 400 upstream — reject earlier).
///     Note: the issue draft said "Unix timestamp"; the upstream Go source is
///     authoritative and takes RFC3339Nano (ADR-0064 deviation note).
///   - both present: from must be strictly before to (handler_metrics.go:38).
///   - to must not be more than METRICS_FUTURE_SKEW in the future (client
///     clock is trusted for `now` defaults; a future `to` is a bug).
///   - window (to - from, or now - from when to is omitted) is capped at
///     METRICS_MAX_WINDOW_SECS so a hostile caller cannot make Resin scan an
///     unbounded metrics.db range.
/// Returns the validated strings ready for ResinClient::probe_history.
pub(crate) const METRICS_FUTURE_SKEW_SECS: i64 = 300;
pub(crate) const METRICS_MAX_WINDOW_SECS: i64 = 7 * 24 * 3600;

pub(crate) fn validate_metrics_range(
    from: Option<&str>,
    to: Option<&str>,
) -> Result<(Option<String>, Option<String>), IpcError> {
    let parse = |s: &str, field: &str| -> Result<chrono::DateTime<chrono::Utc>, IpcError> {
        if s.len() > 64 {
            return Err(IpcError::invalid_input(&format!(
                "metrics '{field}' exceeds 64 chars"
            )));
        }
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&chrono::Utc))
            .map_err(|_| IpcError::invalid_input(&format!("metrics '{field}' must be RFC3339")))
    };
    let from_dt = from.map(|s| parse(s, "from")).transpose()?;
    let to_dt = to.map(|s| parse(s, "to")).transpose()?;
    let now = chrono::Utc::now();
    let skew = chrono::Duration::seconds(METRICS_FUTURE_SKEW_SECS);
    let window_cap = chrono::Duration::seconds(METRICS_MAX_WINDOW_SECS);
    if let Some(t) = to_dt {
        if t > now + skew {
            return Err(IpcError::invalid_input("metrics 'to' is in the future"));
        }
    }
    match (from_dt, to_dt) {
        (Some(f), Some(t)) => {
            if f >= t {
                return Err(IpcError::invalid_input(
                    "metrics 'from' must be before 'to'",
                ));
            }
            if t - f > window_cap {
                return Err(IpcError::invalid_input("metrics window exceeds 7 days"));
            }
        }
        (Some(f), None) => {
            if now - f > window_cap {
                return Err(IpcError::invalid_input("metrics window exceeds 7 days"));
            }
        }
        _ => {}
    }
    Ok((from.map(str::to_string), to.map(str::to_string)))
}

/// (ADR-0064): GET /api/v1/metrics/history/probes — probe-count history
/// buckets for the Diagnostics metrics card. from/to are RFC3339 strings,
/// boundary-validated per §7.5 (see validate_metrics_range); both optional,
/// in which case Resin applies its own defaults (to=now, from=to-1h).
/// Response shape (handler_metrics.go:351): {"bucket_seconds": N,
/// "items": [{"bucket_start","bucket_end","total_count"}]}. Treated as
/// untrusted wire data downstream (TS coerces items to an array).
#[tauri::command]
pub async fn metrics_probe_history(
    sidecar: State<'_, SidecarHandle>,
    from: Option<String>,
    to: Option<String>,
) -> Result<serde_json::Value, IpcError> {
    let (from, to) = validate_metrics_range(from.as_deref(), to.as_deref())?;
    let client = resin_client(&sidecar)?;
    tracing::debug!(
        ?from,
        ?to,
        "metrics_probe_history: querying Resin history buckets"
    );
    client
        .probe_history(from.as_deref(), to.as_deref())
        .await
        .map_err(|e| map_resin_error(&e.to_string()))
}

/// (ADR-0064): GET /api/v1/metrics/realtime/throughput — realtime
/// ingress/egress ring samples for the Diagnostics metrics card. No input
/// params (issue F4: realtime 无入参) — Resin's parseMetricsTimeRange defaults
/// apply upstream (last hour). Response shape (handler_metrics.go:147):
/// {"step_seconds": N, "items": [{"ts","ingress_bps","egress_bps"}]}.
#[tauri::command]
pub async fn metrics_realtime_throughput(
    sidecar: State<'_, SidecarHandle>,
) -> Result<serde_json::Value, IpcError> {
    let client = resin_client(&sidecar)?;
    client
        .realtime_throughput()
        .await
        .map_err(|e| map_resin_error(&e.to_string()))
}

/// §7.5 boundary for the request-log single-entry commands. Resin
/// generates the row id as a UUID (requestlog/repo.go:145
/// `uuid.NewString()`), so the shape is checked strictly — 1..=64 chars of
/// ASCII hex digits and hyphens — which also rejects NUL/control characters,
/// path separators and non-ASCII BEFORE any HTTP call leaves the shell.
/// The 64-char cap (vs UUID's 36) is the issue-drafted extension room for a
/// future upstream id scheme.
pub(crate) const REQUEST_LOG_ID_MAX: usize = 64;

pub(crate) fn validate_log_id(log_id: &str) -> Result<(), IpcError> {
    if log_id.is_empty() {
        return Err(IpcError::invalid_input("log_id must not be empty"));
    }
    if log_id.len() > REQUEST_LOG_ID_MAX {
        return Err(IpcError::invalid_input(&format!(
            "log_id exceeds {} chars",
            REQUEST_LOG_ID_MAX
        )));
    }
    if !log_id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err(IpcError::invalid_input(
            "log_id must be a UUID (hex digits and hyphens only)",
        ));
    }
    Ok(())
}

/// GET /api/v1/request-logs/{log_id} — single request-log
/// entry for the DiagnosticsView detail drawer (RESIN_API_COVERAGE #R45;
/// handler_requestlog.go:174). §7.5 log_id boundary above. The wire Value
/// is returned untrusted — the TS side coerces before rendering.
#[tauri::command]
pub async fn request_log_detail(
    sidecar: State<'_, SidecarHandle>,
    log_id: String,
) -> Result<serde_json::Value, IpcError> {
    validate_log_id(&log_id)?;
    let client = resin_client(&sidecar)?;
    tracing::debug!(%log_id, "request_log_detail: fetching single entry via REST");
    client
        .get_request_log(&log_id)
        .await
        .map_err(|e| map_resin_error(&e.to_string()))
}

/// GET /api/v1/request-logs/{log_id}/payloads — captured
/// request/response payloads for the DiagnosticsView detail drawer
/// (RESIN_API_COVERAGE #R46; handler_requestlog.go:198). Same §7.5 log_id
/// boundary. Upstream returns base64 bodies ({req_headers_b64, req_body_b64,
/// resp_headers_b64, resp_body_b64, truncated{...}} per
/// handler_requestlog.go:355-368) and always answers 200 with empty strings
/// when payload logging is off — the TS side decodes + display-truncates at
/// 1 MB. Treated as untrusted wire data downstream.
#[tauri::command]
pub async fn request_log_payloads(
    sidecar: State<'_, SidecarHandle>,
    log_id: String,
) -> Result<serde_json::Value, IpcError> {
    validate_log_id(&log_id)?;
    let client = resin_client(&sidecar)?;
    tracing::debug!(%log_id, "request_log_payloads: fetching payloads via REST");
    client
        .get_request_log_payloads(&log_id)
        .await
        .map_err(|e| map_resin_error(&e.to_string()))
}

#[cfg(test)]
mod t19_metrics_range_tests {
    use super::validate_metrics_range;

    #[test]
    fn accepts_none_none() {
        assert!(validate_metrics_range(None, None).is_ok());
    }

    #[test]
    fn accepts_valid_one_hour_window() {
        let (from, to) =
            validate_metrics_range(Some("2026-09-04T00:00:00Z"), Some("2026-09-04T01:00:00Z"))
                .expect("valid window must pass");
        assert_eq!(from.as_deref(), Some("2026-09-04T00:00:00Z"));
        assert_eq!(to.as_deref(), Some("2026-09-04T01:00:00Z"));
    }

    #[test]
    fn rejects_non_rfc3339_from() {
        // Unix seconds are NOT accepted (upstream parses RFC3339Nano).
        assert!(validate_metrics_range(Some("1727654400"), None).is_err());
        assert!(validate_metrics_range(Some("not-a-time"), None).is_err());
        // chrono RFC3339 accepts space-as-separator, so use an out-of-range hour
        assert!(validate_metrics_range(None, Some("2026-09-04T25:00:00Z")).is_err());
    }

    #[test]
    fn rejects_from_after_to() {
        assert!(
            validate_metrics_range(Some("2026-09-04T01:00:00Z"), Some("2026-09-04T00:00:00Z"),)
                .is_err()
        );
        assert!(
            validate_metrics_range(Some("2026-09-04T01:00:00Z"), Some("2026-09-04T01:00:00Z"),)
                .is_err()
        );
    }

    #[test]
    fn rejects_future_to() {
        let far = chrono::Utc::now() + chrono::Duration::hours(2);
        assert!(validate_metrics_range(None, Some(far.to_rfc3339().as_str())).is_err());
    }

    #[test]
    fn rejects_window_over_seven_days() {
        assert!(
            validate_metrics_range(Some("2026-08-01T00:00:00Z"), Some("2026-09-04T00:00:00Z"),)
                .is_err()
        );
        // from-only window (to defaults to now upstream) is capped the same.
        assert!(validate_metrics_range(Some("2026-08-01T00:00:00Z"), None).is_err());
    }

    #[test]
    fn rejects_oversized_timestamp() {
        let long = format!("2026-09-04T00:00:00{}Z", "0".repeat(80));
        assert!(validate_metrics_range(Some(long.as_str()), None).is_err());
    }
}

#[cfg(test)]
mod t21_log_id_tests {
    use super::{validate_log_id, REQUEST_LOG_ID_MAX};

    #[test]
    fn accepts_standard_uuid() {
        assert!(validate_log_id("0b7fd2a8-1f3e-4c5d-9a6b-7c8d9e0f1a2b").is_ok());
        assert!(validate_log_id("ABC-123").is_ok());
    }

    #[test]
    fn accepts_uppercase_hex_and_short_ids() {
        assert!(validate_log_id("ABCDEF0123456789ABCDEF0123456789").is_ok());
        assert!(validate_log_id("a-b-c").is_ok());
    }

    #[test]
    fn rejects_empty() {
        assert!(validate_log_id("").is_err());
    }

    #[test]
    fn rejects_over_64_chars() {
        let long = "a".repeat(REQUEST_LOG_ID_MAX + 1);
        assert!(validate_log_id(&long).is_err());
        let edge = "a".repeat(REQUEST_LOG_ID_MAX);
        assert!(validate_log_id(&edge).is_ok());
    }

    #[test]
    fn rejects_non_uuid_charset() {
        // NUL + control characters, path separators, spaces, non-ASCII.
        assert!(validate_log_id("abc\0def").is_err());
        assert!(validate_log_id("abc\u{7f}def").is_err());
        assert!(validate_log_id("../etc/passwd").is_err());
        assert!(validate_log_id("has space").is_err());
        assert!(validate_log_id("汉字").is_err());
    }
}
