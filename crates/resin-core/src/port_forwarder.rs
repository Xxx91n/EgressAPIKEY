//! Thin multi-port identity layer (ADR-0012 / ADR-0015).
//!
//! Each Entry Port is an identity. The shell delegates port listener lifecycle
//! to the Resin v1.2.0 sidecar via the /api/v1/endpoints API. The shell keeps:
//!   - PortMapping persistence (port -> platform_name binding metadata;
//!     Resin endpoint schema has no platform_name field, so the shell owns it)
//!   - StreamSensor AI flow classification (SSE/WS/unary) — shell-unique value
//!   - resin_identity() — builds the Resin V1 proxy auth string
//!   - Input validation constants (MAX_ENTRY_PORTS, MIN_USER_PORT)
//!
//! Ponytail: no TcpListener, no SOCKS5/HTTP protocol handling. Resin's
//! production protocol stack owns listener + connection lifecycle. The shell
//! only forwards CRUD to the Resin endpoint API and observes AI stream headers
//! when they pass through the Rust interceptor layer (future wiring).

use std::sync::Arc;

use tokio::sync::watch;

use crate::db::DbPool;
use crate::stream_sensor::{StreamSensor, StreamSensorSnapshot};

/// Max concurrent entry ports the shell will bind (industrial safety).
pub const MAX_ENTRY_PORTS: usize = 256;
/// Reserved / privileged ports rejected at the IPC boundary.
pub const MIN_USER_PORT: u16 = 1024;

/// Build the Resin V1 identity string `Platform.Account`.
/// Account defaults to `port-<n>` when empty so two ports never collapse.
pub fn resin_identity(platform_name: &str, account: &str, port: u16) -> String {
    let platform = platform_name.trim();
    let platform = if platform.is_empty() {
        "Default"
    } else {
        platform
    };
    let account = account.trim();
    let account = if account.is_empty() {
        format!("port-{port}")
    } else {
        account.to_string()
    };
    format!("{platform}.{account}")
}

/// First-byte protocol detect: 0x05 = SOCKS5, otherwise treat as HTTP.
pub fn detect_protocol(first: u8) -> &'static str {
    if first == 0x05 {
        "socks5"
    } else {
        "http"
    }
}

#[cfg(test)]
fn basic_proxy_auth(identity: &str, proxy_token: &str) -> String {
    let raw = format!("{identity}:{proxy_token}");
    b64_encode(raw.as_bytes())
}

#[cfg(test)]
fn b64_encode(input: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(((input.len() + 2) / 3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((triple >> 18) & 63) as usize] as char);
        out.push(CHARS[((triple >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((triple >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(triple & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Runtime port-identity + stream-sensor holder.
///
/// Owns the StreamSensor (shell-unique AI flow classification) and the
/// port-to-platform binding metadata (shell-side, complements Resin's
/// endpoint API which has no platform_name field). Port listener lifecycle
/// is delegated to the Resin sidecar — this struct no longer binds any
/// TcpListener.
#[derive(Clone)]
pub struct PortForwarder {
    inner: Arc<PortForwarderInner>,
}

struct PortForwarderInner {
    db: DbPool,
    resin_host: String,
    resin_port: u16,
    proxy_token: String,
    stream_sensor: StreamSensor,
    /// Shutdown signal — kept for API compatibility; no longer used to
    /// stop accept loops (Resin owns listener lifecycle now).
    _cancel: watch::Sender<bool>,
}

impl PortForwarder {
    pub fn new(
        db: DbPool,
        resin_host: impl Into<String>,
        resin_port: u16,
        proxy_token: impl Into<String>,
    ) -> Self {
        let (_cancel, _rx) = watch::channel(true);
        let inner = Arc::new(PortForwarderInner {
            db,
            resin_host: resin_host.into(),
            resin_port,
            proxy_token: proxy_token.into(),
            stream_sensor: StreamSensor::new(),
            _cancel,
        });
        Self { inner }
    }

    /// Resin sidecar address (for IPC layer to build ResinClient).
    pub fn resin_addr(&self) -> String {
        format!("http://{}:{}", self.inner.resin_host, self.inner.resin_port)
    }

    pub fn proxy_token(&self) -> &str {
        &self.inner.proxy_token
    }

    /// Header-only AI stream observations from plain HTTP proxy traffic.
    pub fn stream_snapshot(&self) -> StreamSensorSnapshot {
        self.inner.stream_sensor.snapshot()
    }

    /// Returns the list of ports that have a shell-side PortMapping.
    /// The actual listener status is owned by Resin; this is the shell's
    /// metadata view (port -> platform binding).
    pub fn running_ports(&self) -> Vec<u16> {
        self.inner
            .db
            .list_ports()
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.enabled)
            .map(|m| m.port)
            .collect()
    }

    /// No-op shutdown — kept for API compatibility. Resin owns listener
    /// lifecycle; the shell no longer manages accept loops.
    pub fn shutdown(&self) {
        // Resin endpoint listeners are managed via the sidecar API.
        // The shell's StreamSensor has no background tasks to stop.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_defaults_account_to_port() {
        let id = resin_identity("MyPlatform", "", 17990);
        assert_eq!(id, "MyPlatform.port-17990");
    }

    #[test]
    fn distinct_ports_distinct_identities() {
        let a = resin_identity("P", "", 17990);
        let b = resin_identity("P", "", 17991);
        assert_ne!(a, b);
        assert!(a.contains("17990"));
        assert!(b.contains("17991"));
    }

    #[test]
    fn detect_socks_vs_http() {
        assert_eq!(detect_protocol(0x05), "socks5");
        assert_eq!(detect_protocol(0x47), "http");
        assert_eq!(detect_protocol(0x00), "http");
    }

    #[test]
    fn basic_auth_roundtrip_shape() {
        let identity = "Default.port-17990";
        let token = "testtok";
        let encoded = basic_proxy_auth(identity, token);
        assert!(!encoded.is_empty());
        assert!(encoded.len() % 4 == 0);
    }

    #[test]
    fn b64_padding() {
        let one = b64_encode(b"a");
        assert_eq!(one, "YQ==");
        let two = b64_encode(b"ab");
        assert_eq!(two, "YWI=");
        let three = b64_encode(b"abc");
        assert_eq!(three, "YWJj");
    }
}


/// Parse the exit IP from a Cloudflare cdn-cgi/trace response body.
/// Returns the value after "ip=" on the first matching line, trimmed.
/// Returns empty string if no "ip=" line is found.
pub fn parse_trace_body_ip(body: &str) -> String {
    body.lines()
        .find_map(|l| l.strip_prefix("ip=").map(|s| s.trim().to_string()))
        .unwrap_or_default()
}

#[cfg(test)]
mod trace_tests {
    use super::parse_trace_body_ip;

    #[test]
    fn parses_ip_from_cloudflare_trace() {
        let body = "fl=123f\nh=cloudflare.com\nip=203.0.113.50\ntls=TLSv1.3\n";
        assert_eq!(parse_trace_body_ip(body), "203.0.113.50");
    }

    #[test]
    fn returns_empty_when_no_ip_line() {
        let body = "fl=123f\nh=cloudflare.com\nvisited=2026-08-12\n";
        assert_eq!(parse_trace_body_ip(body), "");
    }

    #[test]
    fn trims_whitespace_around_ip() {
        let body = "ip=  198.51.100.1  \n";
        assert_eq!(parse_trace_body_ip(body), "198.51.100.1");
    }
}
