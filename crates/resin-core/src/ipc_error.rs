//! Phase 5-2: typed IPC error contract (ADR-0026 Q6-Q8).
//!
//! Every `#[tauri::command]` returns `Result<T, IpcError>` instead of
//! `Result<T, String>`. The enum carries an `i18n_key` per variant so the
//! frontend can render a locale-specific message without parsing English
//! error text. Externally-tagged serde for TS discriminated-union narrowing.
//!
//! Ponytail: no extra crate. serde + std only.

use serde::{Deserialize, Serialize};

/// Typed IPC error returned by every Tauri command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "data")]
pub enum IpcError {
    /// Port number already in use (EADDRINUSE / Resin 409 bind conflict).
    BindConflict {
        port: u16,
        i18n_key: String,
    },
    /// Strategy value rejected by the Rust-side catalog.
    InvalidStrategy {
        value: String,
        accepted: Vec<String>,
        i18n_key: String,
    },
    /// Resin sidecar returned a non-2xx status.
    ResinUpstream {
        status: u16,
        excerpt: String,
        i18n_key: String,
    },
    /// Catch-all for internal errors (serde, IO, unexpected panic recovery).
    Internal {
        msg: String,
        i18n_key: String,
    },
}

impl IpcError {
    pub fn bind_conflict(port: u16) -> Self {
        Self::BindConflict {
            port,
            i18n_key: "error.bindConflict".into(),
        }
    }

    pub fn invalid_strategy(value: &str, accepted: &[&str]) -> Self {
        Self::InvalidStrategy {
            value: value.into(),
            accepted: accepted.iter().map(|s| (*s).into()).collect(),
            i18n_key: "error.invalidStrategy".into(),
        }
    }

    pub fn resin_upstream(status: u16, excerpt: &str) -> Self {
        Self::ResinUpstream {
            status,
            excerpt: excerpt.chars().take(256).collect(),
            i18n_key: "error.resinUpstream".into(),
        }
    }

    pub fn internal(msg: &str) -> Self {
        Self::Internal {
            msg: msg.chars().take(256).collect(),
            i18n_key: "error.internal".into(),
        }
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BindConflict { port, .. } => write!(f, "bind conflict on port {port}"),
            Self::InvalidStrategy { value, .. } => write!(f, "invalid strategy: {value}"),
            Self::ResinUpstream { status, excerpt, .. } => {
                write!(f, "Resin upstream {status}: {excerpt}")
            }
            Self::Internal { msg, .. } => write!(f, "internal: {msg}"),
        }
    }
}

impl std::error::Error for IpcError {}
impl From<String> for IpcError {
    /// Auto-convert String errors so commands using `?` operator with String-returning
    /// helpers (validate_*, resin_client, map_resin_error) can return Result<T, IpcError>
    /// without per-call .map_err boilerplate. Ponytail: From impl, not 44 hand edits.
    fn from(raw: String) -> Self {
        if raw.contains("bind") && (raw.contains("Only one usage") || raw.contains("CONFLICT")) {
            if let Some(port) = extract_port(&raw) {
                return IpcError::bind_conflict(port);
            }
        }
        if raw.contains("must be BALANCED") || raw.contains("InvalidStrategy") {
            return IpcError::invalid_strategy(&raw, &["random","sequential","latency","quality","bandwidth","protocol_weight"]);
        }
        IpcError::internal(&raw)
    }
}

impl From<serde_json::Error> for IpcError {
    fn from(e: serde_json::Error) -> Self {
        IpcError::internal(&e.to_string())
    }
}


/// Map a Resin HTTP response (status + body excerpt) to an IpcError variant.
/// Called by `commands/mod.rs` when `ResinClient::send()` returns a non-2xx.
pub fn map_resin_error(status: u16, body: &str) -> IpcError {
    let lower = body.to_ascii_lowercase();
    match status {
        0 => IpcError::internal("non-HTTP failure (sidecar unreachable / connect timeout / DNS)"),

        409 if lower.contains("bind") || lower.contains("already exists") || lower.contains("port") => {
            // Extract port number from the error message if possible.
            let port = extract_port(body).unwrap_or(0);
            IpcError::bind_conflict(port)
        }
        400 if lower.contains("must be") || lower.contains("allocation_policy") || lower.contains("strategy") => {
            IpcError::invalid_strategy(body, &[
                "random", "sequential", "latency", "quality", "bandwidth", "protocol_weight",
            ])
        }
        400..=499 => IpcError::resin_upstream(status, body),
        500..=599 => IpcError::resin_upstream(status, body),
        _ => IpcError::resin_upstream(status, body),
    }
}

/// Extract a port number from an error message containing "port" + digits.
fn extract_port(s: &str) -> Option<u16> {
    // Look for "port 12345" or ":12345" patterns
    let lower = s.to_ascii_lowercase();
    let port_idx = lower.find("port ")?;
    let rest = &s[port_idx + 5..];
    rest.chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_error_bind_conflict_serde_round_trip() {
        let e = IpcError::bind_conflict(17111);
        let json = serde_json::to_string(&e).unwrap();
        let back: IpcError = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
        assert!(json.contains("\"kind\":\"BindConflict\""));
        assert!(json.contains("\"i18n_key\":\"error.bindConflict\""));
    }

    #[test]
    fn ipc_error_invalid_strategy_serde_round_trip() {
        let e = IpcError::invalid_strategy("balanced", &["random", "sequential"]);
        let json = serde_json::to_string(&e).unwrap();
        let back: IpcError = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
        assert!(json.contains("\"kind\":\"InvalidStrategy\""));
    }

    #[test]
    fn ipc_error_resin_upstream_serde_round_trip() {
        let e = IpcError::resin_upstream(503, "service unavailable");
        let json = serde_json::to_string(&e).unwrap();
        let back: IpcError = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
        assert!(json.contains("\"kind\":\"ResinUpstream\""));
    }

    #[test]
    fn ipc_error_internal_serde_round_trip() {
        let e = IpcError::internal("unexpected IO error");
        let json = serde_json::to_string(&e).unwrap();
        let back: IpcError = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
        assert!(json.contains("\"kind\":\"Internal\""));
    }

    #[test]
    fn map_resin_error_409_bind_to_bind_conflict() {
        let e = map_resin_error(
            409,
            "listen on port 17111: bind: Only one usage of each socket address",
        );
        assert!(matches!(e, IpcError::BindConflict { port: 17111, .. }));
    }

    #[test]
    fn map_resin_error_400_strategy_to_invalid_strategy() {
        let e = map_resin_error(
            400,
            "must be BALANCED, PREFER_LOW_LATENCY, or PREFER_IDLE_IP",
        );
        assert!(matches!(e, IpcError::InvalidStrategy { .. }));
    }

    #[test]
    fn map_resin_error_403_to_resin_upstream() {
        let e = map_resin_error(403, "forbidden");
        assert!(matches!(e, IpcError::ResinUpstream { status: 403, .. }));
    }

    #[test]
    fn map_resin_error_503_to_resin_upstream() {
        let e = map_resin_error(503, "service unavailable");
        assert!(matches!(e, IpcError::ResinUpstream { status: 503, .. }));
    }

    #[test]
    fn map_resin_error_unknown_status_falls_back_to_upstream() {
        let e = map_resin_error(418, "I'm a teapot");
        assert!(matches!(e, IpcError::ResinUpstream { status: 418, .. }));
    }

    #[test]
    fn extract_port_finds_number_after_port_keyword() {
        assert_eq!(extract_port("listen on port 17111: bind"), Some(17111));
        assert_eq!(extract_port("no port here"), None);
    }

    #[test]
    fn map_resin_error_status_zero_returns_internal() {
        let e = map_resin_error(0, "sidecar unreachable");
        assert!(matches!(e, IpcError::Internal { .. }));
        let result = match e {
            IpcError::Internal { msg, i18n_key } => (msg, i18n_key),
            _ => unreachable!(),
        };
        assert_eq!(result.1, "error.internal");
    }

    #[test]
    fn map_resin_error_status_500_returns_resin_upstream_regression() {
        let e = map_resin_error(500, "internal server error");
        assert!(matches!(e, IpcError::ResinUpstream { status: 500, .. }));
    }
}