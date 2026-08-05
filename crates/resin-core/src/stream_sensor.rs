//! AI stream sensor (Phase 3 / NEW-5).
//!
//! Independent, pluggable observation board. Classifies proxied HTTP traffic
//! as Unary / SSE / WebSocket from **headers only** (no body sniffing, no API
//! key inspection — ADR-0014). Counters are lock-free enough for the thin
//! shell: a parking_lot mutex around three u64s.
//!
//! On the multi-port SOCKS5 path most AI HTTPS is end-to-end encrypted after
//! CONNECT, so classification only fires for plain HTTP proxy requests where
//! request headers are visible. That is intentional — the sensor never
//! terminates TLS.

use parking_lot::Mutex;
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    Unary,
    Sse,
    WebSocket,
    Unknown,
}

/// Header-only classifier. `accept` / `upgrade` / `content_type` are raw
/// header values (may be empty).
pub fn classify_http_headers(accept: &str, upgrade: &str, content_type: &str) -> StreamKind {
    let upgrade_l = upgrade.to_ascii_lowercase();
    if upgrade_l.split(',').any(|p| p.trim() == "websocket") {
        return StreamKind::WebSocket;
    }
    let accept_l = accept.to_ascii_lowercase();
    if accept_l.contains("text/event-stream") {
        return StreamKind::Sse;
    }
    let ct = content_type.to_ascii_lowercase();
    if ct.contains("text/event-stream") {
        return StreamKind::Sse;
    }
    if accept_l.contains("application/json") || ct.contains("application/json") {
        return StreamKind::Unary;
    }
    if !accept.is_empty() || !content_type.is_empty() {
        return StreamKind::Unary;
    }
    StreamKind::Unknown
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct StreamSensorSnapshot {
    pub unary: u64,
    pub sse: u64,
    pub websocket: u64,
    pub unknown: u64,
}

#[derive(Debug, Default, Clone)]
struct Counters {
    unary: u64,
    sse: u64,
    websocket: u64,
    unknown: u64,
}

/// Process-wide observation board. Clone is cheap (Arc).
#[derive(Debug, Clone, Default)]
pub struct StreamSensor {
    inner: Arc<Mutex<Counters>>,
}

impl StreamSensor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, kind: StreamKind) {
        let mut g = self.inner.lock();
        match kind {
            StreamKind::Unary => g.unary = g.unary.saturating_add(1),
            StreamKind::Sse => g.sse = g.sse.saturating_add(1),
            StreamKind::WebSocket => g.websocket = g.websocket.saturating_add(1),
            StreamKind::Unknown => g.unknown = g.unknown.saturating_add(1),
        }
    }

    pub fn observe_headers(&self, accept: &str, upgrade: &str, content_type: &str) -> StreamKind {
        let kind = classify_http_headers(accept, upgrade, content_type);
        self.record(kind);
        kind
    }

    pub fn snapshot(&self) -> StreamSensorSnapshot {
        let g = self.inner.lock();
        StreamSensorSnapshot {
            unary: g.unary,
            sse: g.sse,
            websocket: g.websocket,
            unknown: g.unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_sse_accept() {
        assert_eq!(
            classify_http_headers("text/event-stream", "", "application/json"),
            StreamKind::Sse
        );
    }

    #[test]
    fn classifies_websocket_upgrade() {
        assert_eq!(
            classify_http_headers("*/*", "websocket", ""),
            StreamKind::WebSocket
        );
    }

    #[test]
    fn classifies_json_unary() {
        assert_eq!(
            classify_http_headers("application/json", "", "application/json"),
            StreamKind::Unary
        );
    }

    #[test]
    fn sensor_counts_independently() {
        let s = StreamSensor::new();
        s.observe_headers("text/event-stream", "", "");
        s.observe_headers("application/json", "", "");
        s.observe_headers("", "websocket", "");
        s.observe_headers("", "", "");
        let snap = s.snapshot();
        assert_eq!(snap.sse, 1);
        assert_eq!(snap.unary, 1);
        assert_eq!(snap.websocket, 1);
        assert_eq!(snap.unknown, 1);
    }
}
