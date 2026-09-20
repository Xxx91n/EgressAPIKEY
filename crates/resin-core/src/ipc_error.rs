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
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", content = "data")]
pub enum IpcError {
    /// Port number already in use (EADDRINUSE / Resin 409 bind conflict).
    BindConflict { port: u16, i18n_key: String },
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
    /// Command input failed §7.5 validation (range/length cap) at the Rust
    /// boundary. introduced by the diag-poll-interval command
    /// pair; msg carries the rejected bound for log display. Reuses the
    /// existing "error.badRequest" locale key (no new i18n key, 不做清单).
    InvalidInput { msg: String, i18n_key: String },
    /// Name-based lookup miss: the typed face of the
    /// former stringly `format!("platform not found: {name}")` rejections the
    /// name→UUID call sites produced through `IpcError::from(String)`.
    /// Reuses the existing "error.notFound" locale key (already present in
    /// all 18 catalogs), so no new i18n key is added.
    NotFound { msg: String, i18n_key: String },
    /// Catch-all for internal errors (serde, IO, unexpected panic recovery).
    Internal { msg: String, i18n_key: String },
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

    /// §7.5 input-validation rejection. Reuses the existing
    /// "error.badRequest" locale key so no new i18n key is added.
    pub fn invalid_input(msg: &str) -> Self {
        Self::InvalidInput {
            msg: msg.chars().take(256).collect(),
            i18n_key: "error.badRequest".into(),
        }
    }

    /// Name-based lookup miss. `msg` carries the same human
    /// sentence the former stringly rejections used ("platform not found:
    /// {name}"); the i18n key reuses the existing "error.notFound" entry.
    pub fn not_found(msg: &str) -> Self {
        Self::NotFound {
            msg: msg.chars().take(256).collect(),
            i18n_key: "error.notFound".into(),
        }
    }

    /// Internal variant with a specific i18n key and the raw upstream error
    /// preserved in msg for log/debug display. Used by map_resin_error for
    /// recognized vocabulary that has no dedicated variant.
    pub fn internal_keyed(i18n_key: &str, msg: &str) -> Self {
        Self::Internal {
            msg: msg.chars().take(256).collect(),
            i18n_key: i18n_key.into(),
        }
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BindConflict { port, .. } => write!(f, "bind conflict on port {port}"),
            Self::InvalidStrategy { value, .. } => write!(f, "invalid strategy: {value}"),
            Self::ResinUpstream {
                status, excerpt, ..
            } => {
                write!(f, "Resin upstream {status}: {excerpt}")
            }
            Self::InvalidInput { msg, .. } => write!(f, "invalid input: {msg}"),
            Self::NotFound { msg, .. } => write!(f, "{msg}"),
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
            return IpcError::invalid_strategy(
                &raw,
                &[
                    "random",
                    "sequential",
                    "latency",
                    "quality",
                    "bandwidth",
                    "protocol_weight",
                ],
            );
        }
        IpcError::internal(&raw)
    }
}

impl From<serde_json::Error> for IpcError {
    fn from(e: serde_json::Error) -> Self {
        IpcError::internal(&e.to_string())
    }
}

/// Single Resin error -> IpcError mapping
/// the shell-side duplicate in src-tauri/src/commands/mod.rs was deleted; its
/// call sites now call this function directly via resin_core::map_resin_error).
///
/// Input is the Display string of the ResinClient anyhow error, which embeds
/// the HTTP method, path, status and a short body excerpt, e.g.
/// "resin_client: POST /endpoints -> 409 Conflict: {\"error\":...}".
///
/// Semantics kept from the former shell copy (probe-verified vocabulary; the
/// bind guard must run before the generic CONFLICT guard so 409 bind errors
/// keep their port number), with the i18n key now written into the variant
/// i18n_key field (the former shell copy put keys into Internal.msg, which
/// the frontend translateError never read) and bind conflicts producing the
/// typed BindConflict variant instead of a string round-trip through
/// From<String>. Unknown errors fall through IpcError::from -> Internal.
pub fn map_resin_error(raw: &str) -> IpcError {
    // Known Resin error patterns (from DESIGN.md error code table + probed).
    if raw.contains("cannot delete Default platform") {
        return IpcError::internal_keyed("error.cannotDeleteDefaultPlatform", raw);
    }
    if raw.contains("AUTH_REQUIRED") || raw.contains("auth required") {
        return IpcError::internal_keyed("error.authRequired", raw);
    }
    if raw.contains("AUTH_FAILED") || raw.contains("auth failed") {
        return IpcError::internal_keyed("error.authFailed", raw);
    }
    if raw.contains("URL_PARSE_ERROR") || raw.contains("url parse") {
        return IpcError::internal_keyed("error.urlParse", raw);
    }
    if raw.contains("INVALID_PROTOCOL") || raw.contains("invalid protocol") {
        return IpcError::internal_keyed("error.invalidProtocol", raw);
    }
    if raw.contains("UPSTREAM_CONNECT_FAILED") || raw.contains("upstream connect") {
        return IpcError::internal_keyed("error.upstreamConnectFailed", raw);
    }
    if raw.contains("UPSTREAM_REQUEST_FAILED") || raw.contains("upstream request") {
        return IpcError::internal_keyed("error.upstreamRequestFailed", raw);
    }
    // Port bind conflict (ADR-0026 Q6): Resin returns 409 with a body like
    // "listen on port 17111: bind: Only one usage of each socket address ...".
    // Extract the port so the frontend can show "port {{port}} already in use"
    // and offer the change-port recovery action.
    if raw.contains("bind")
        && (raw.contains("Only one usage")
            || raw.contains("EADDRINUSE")
            || raw.contains("address already in use"))
    {
        if let Some(port) = extract_port(raw) {
            return IpcError::bind_conflict(port);
        }
        return IpcError::internal_keyed("error.bindConflict", raw);
    }
    if raw.contains("CONFLICT") {
        return IpcError::internal_keyed("error.conflict", raw);
    }
    if raw.contains("not found") || raw.contains("NOT_FOUND") {
        return IpcError::internal_keyed("error.notFound", raw);
    }
    if raw.contains("BAD_REQUEST") || raw.contains("bad request") {
        return IpcError::internal_keyed("error.badRequest", raw);
    }
    if raw.contains("UNAUTHORIZED") || raw.contains("unauthorized") {
        return IpcError::internal_keyed("error.unauthorized", raw);
    }
    // An HTTP status embedded by ResinClient ("-> 418 : excerpt") surfaces as
    // typed ResinUpstream so the frontend can render status + excerpt.
    if let Some(status) = embedded_status(raw) {
        let excerpt = raw.rsplit("-> ").next().unwrap_or(raw);
        return IpcError::resin_upstream(status, excerpt);
    }
    // Unknown — pass through the literal for the frontend to display.
    IpcError::from(raw.to_string())
}

/// Extract a port number from an error message containing "port " + digits.
/// Zero-alloc, no regex (same logic as the deleted shell-side helper).
fn extract_port(s: &str) -> Option<u16> {
    let lower = s.to_ascii_lowercase();
    let idx = lower.find("port ")?;
    let rest = &s[idx + 5..];
    rest.chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

/// Pull the HTTP status code out of a ResinClient error string shaped
/// "... -> {method} {path} -> {status}: {excerpt}". Only digits immediately
/// after the last "-> " count, so number fragments elsewhere in the body
/// (lease ids, timestamps, ports) cannot be mistaken for the status.
fn embedded_status(raw: &str) -> Option<u16> {
    let tail = raw.rsplit("-> ").next()?.trim_start();
    let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() != 3 {
        return None;
    }
    match tail.chars().nth(3) {
        Some(':') | Some(' ') | None => digits.parse().ok(),
        _ => None,
    }
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
    fn ipc_error_invalid_input_serde_round_trip() {
        // §7.5 range-rejection variant. Reuses error.badRequest
        // so the frontend needs no new locale key.
        let e = IpcError::invalid_input("interval_ms must be 100..=86400000");
        let json = serde_json::to_string(&e).unwrap();
        let back: IpcError = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
        assert!(json.contains("\"kind\":\"InvalidInput\""));
        assert!(json.contains("\"i18n_key\":\"error.badRequest\""));
    }

    #[test]
    fn ipc_error_not_found_serde_round_trip() {
        // name→UUID lookup miss. Reuses the existing
        // error.notFound locale key (present in all 18 catalogs).
        let e = IpcError::not_found("platform not found: alpha");
        assert!(matches!(e, IpcError::NotFound { ref msg, ref i18n_key }
            if msg == "platform not found: alpha" && i18n_key == "error.notFound"));
        let json = serde_json::to_string(&e).unwrap();
        let back: IpcError = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
        assert!(json.contains("\"kind\":\"NotFound\""));
        assert!(json.contains("\"i18n_key\":\"error.notFound\""));
    }

    // ---- map_resin_error(raw) — single implementation ----

    #[test]
    fn map_resin_error_cannot_delete_default() {
        let raw = r#"409 Conflict: {"error":{"code":"CONFLICT","message":"cannot delete Default platform"}}"#;
        let e = map_resin_error(raw);
        assert!(
            matches!(e, IpcError::Internal { ref i18n_key, .. } if i18n_key == "error.cannotDeleteDefaultPlatform"),
            "got {e:?}"
        );
    }

    #[test]
    fn map_resin_error_auth_required() {
        let e =
            map_resin_error("resin_client: GET /platforms -> 407: AUTH_REQUIRED: missing token");
        assert!(
            matches!(e, IpcError::Internal { ref i18n_key, .. } if i18n_key == "error.authRequired")
        );
    }

    #[test]
    fn map_resin_error_auth_failed() {
        let e = map_resin_error("AUTH_FAILED: bad token");
        assert!(
            matches!(e, IpcError::Internal { ref i18n_key, .. } if i18n_key == "error.authFailed")
        );
    }

    #[test]
    fn map_resin_error_url_parse() {
        let e = map_resin_error("URL_PARSE_ERROR: invalid URL");
        assert!(
            matches!(e, IpcError::Internal { ref i18n_key, .. } if i18n_key == "error.urlParse")
        );
    }

    #[test]
    fn map_resin_error_invalid_protocol() {
        let e = map_resin_error("INVALID_PROTOCOL: ftp not supported");
        assert!(
            matches!(e, IpcError::Internal { ref i18n_key, .. } if i18n_key == "error.invalidProtocol")
        );
    }

    #[test]
    fn map_resin_error_upstream_connect_failed() {
        let e = map_resin_error("502 UPSTREAM_CONNECT_FAILED: connection refused");
        assert!(
            matches!(e, IpcError::Internal { ref i18n_key, .. } if i18n_key == "error.upstreamConnectFailed")
        );
    }

    #[test]
    fn map_resin_error_upstream_request_failed() {
        let e = map_resin_error("UPSTREAM_REQUEST_FAILED: 502 bad gateway");
        assert!(
            matches!(e, IpcError::Internal { ref i18n_key, .. } if i18n_key == "error.upstreamRequestFailed")
        );
    }

    #[test]
    fn map_resin_error_bind_conflict_with_port() {
        let raw = "create_endpoint: resin_client: POST /endpoints -> 409 Conflict: {\"error\":{\"message\":\"listen on port 17111: bind: Only one usage of each socket address (protocol/network address/port) is normally permitted.\"}}";
        let e = map_resin_error(raw);
        assert!(
            matches!(e, IpcError::BindConflict { port: 17111, ref i18n_key } if i18n_key == "error.bindConflict"),
            "got {e:?}"
        );
    }

    #[test]
    fn map_resin_error_bind_conflict_without_port() {
        // No "port <digits>" extractable -> Internal carrying the bindConflict
        // i18n key (generic template, no {{port}} interpolation) + raw message.
        let e = map_resin_error("EADDRINUSE: address already in use (bind)");
        assert!(
            matches!(e, IpcError::Internal { ref i18n_key, .. } if i18n_key == "error.bindConflict"),
            "got {e:?}"
        );
    }

    #[test]
    fn map_resin_error_bind_takes_precedence_over_conflict() {
        // regression: the bind guard must win before generic CONFLICT.
        let raw = "409 Conflict: listen on port 17999: bind: Only one usage of each socket address";
        let e = map_resin_error(raw);
        assert!(
            matches!(e, IpcError::BindConflict { port: 17999, .. }),
            "got {e:?}"
        );
    }

    #[test]
    fn map_resin_error_conflict() {
        let e = map_resin_error("409 CONFLICT: resource already exists");
        assert!(
            matches!(e, IpcError::Internal { ref i18n_key, .. } if i18n_key == "error.conflict")
        );
    }

    #[test]
    fn map_resin_error_not_found() {
        let e = map_resin_error("platform not found");
        assert!(
            matches!(e, IpcError::Internal { ref i18n_key, .. } if i18n_key == "error.notFound")
        );
    }

    #[test]
    fn map_resin_error_bad_request() {
        let e = map_resin_error("400 BAD_REQUEST: missing field");
        assert!(
            matches!(e, IpcError::Internal { ref i18n_key, .. } if i18n_key == "error.badRequest")
        );
    }

    #[test]
    fn map_resin_error_unauthorized() {
        let e = map_resin_error("401 UNAUTHORIZED: no admin token");
        assert!(
            matches!(e, IpcError::Internal { ref i18n_key, .. } if i18n_key == "error.unauthorized")
        );
    }

    #[test]
    fn map_resin_error_embedded_http_status_surfaces_resin_upstream() {
        // ADR-0045 contract: non-2xx without recognized vocabulary must keep
        // status + excerpt, not degrade to "internal error".
        let raw = "resin_client: GET /nodes -> 418 : I'm a teapot";
        let e = map_resin_error(raw);
        assert!(
            matches!(e, IpcError::ResinUpstream { status: 418, .. }),
            "got {e:?}"
        );
        if let IpcError::ResinUpstream {
            status,
            excerpt,
            i18n_key,
        } = e
        {
            assert_eq!(status, 418);
            assert!(excerpt.contains("teapot"), "excerpt: {excerpt}");
            assert_eq!(i18n_key, "error.resinUpstream");
        }
    }

    #[test]
    fn map_resin_error_403_forbidden_becomes_resin_upstream() {
        let e = map_resin_error("resin_client: GET /platforms -> 403 : forbidden");
        assert!(
            matches!(e, IpcError::ResinUpstream { status: 403, .. }),
            "got {e:?}"
        );
    }

    #[test]
    fn map_resin_error_500_becomes_resin_upstream_regression() {
        let e = map_resin_error("resin_client: GET /leases -> 500 : internal server error");
        assert!(
            matches!(e, IpcError::ResinUpstream { status: 500, .. }),
            "got {e:?}"
        );
    }

    #[test]
    fn map_resin_error_unknown_passes_through_as_internal() {
        // Unknown text carries no embedded status -> From<String> -> Internal
        // with the raw text preserved in msg (frontend shows it via fallback).
        let raw = "some completely unknown error string";
        let e = map_resin_error(raw);
        match e {
            IpcError::Internal { msg, i18n_key } => {
                assert_eq!(msg, raw);
                assert_eq!(i18n_key, "error.internal");
            }
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    #[test]
    fn extract_port_finds_number_after_port_keyword() {
        assert_eq!(extract_port("listen on port 17111: bind"), Some(17111));
        assert_eq!(extract_port("port 443"), Some(443));
        assert_eq!(extract_port("no port here"), None);
    }

    #[test]
    fn embedded_status_parses_resin_client_shape_only() {
        assert_eq!(
            embedded_status("resin_client: GET /x -> 404 : nope"),
            Some(404)
        );
        assert_eq!(embedded_status("POST /y -> 503: down"), Some(503));
        // Numbers not directly after the last arrow are not statuses.
        assert_eq!(embedded_status("lease 17111 expired"), None);
        assert_eq!(embedded_status("timeout after 8s"), None);
        // Four digits is not an HTTP status.
        assert_eq!(embedded_status("GET /x -> 1711 : weird"), None);
    }

    #[test]
    fn internal_keyed_preserves_raw_msg_and_key() {
        let e = IpcError::internal_keyed("error.authRequired", "AUTH_REQUIRED: missing");
        match e {
            IpcError::Internal { msg, i18n_key } => {
                assert_eq!(msg, "AUTH_REQUIRED: missing");
                assert_eq!(i18n_key, "error.authRequired");
            }
            other => panic!("expected Internal, got {other:?}"),
        }
    }
}
