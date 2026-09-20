//! Phase 5-1 trace helpers (ADR-0026 Q1-Q5).
//!
//! Frontend injects `__trace_id` (UUID v4 via crypto.randomUUID()) into every
//! Tauri IPC call's args. The Rust command entry extracts it and opens a
//! tracing::info_span! so every log line in the daily-rotated file carries
//! the same trace_id — enabling end-to-end bug reproduction from a single grep.
//!
//! Ponytail: no uuid crate. UUID v4 validation is a 50-char pattern check.

/// Extract `__trace_id` from the serde_json args object the Tauri command
/// received. Returns Some(id) if the field exists and matches UUID v4 format
/// (8-4-4-4-12 hex), None otherwise. Never panics — a missing/garbage trace_id
/// is logged as "none" and does not block the command.
pub fn extract_trace_id(args: &serde_json::Value) -> Option<String> {
    let raw = args.get("__trace_id")?.as_str()?;
    if is_uuid_v4(raw) {
        Some(raw.to_string())
    } else {
        None
    }
}

/// Create a tracing span for an IPC command. The span carries the trace_id
/// field so all child tracing::info!/warn!/error! events inherit it.
pub fn trace_ipc_span(cmd: &str, trace_id: &str) -> tracing::Span {
    tracing::info_span!("ipc", cmd = %cmd, trace_id = %trace_id)
}

/// Validate a UUID v4 string: 8-4-4-4-12 hex, version nibble = 4.
/// ponytail: hand-rolled to avoid adding uuid crate for a 1-field validation.
fn is_uuid_v4(s: &str) -> bool {
    if s.len() != 36 {
        return false;
    }
    let bytes = s.as_bytes();
    // Format: xxxxxxxx-xxxx-Mxxx-Nxxx-xxxxxxxxxxxx
    // Positions 8,13,18,23 must be '-'. Position 14 (version) must be '4'.
    bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && bytes[14] == b'4'
        && bytes[..8].iter().all(|b| b.is_ascii_hexdigit())
        && bytes[9..13].iter().all(|b| b.is_ascii_hexdigit())
        && bytes[14..18].iter().all(|b| b.is_ascii_hexdigit())
        && bytes[19..23].iter().all(|b| b.is_ascii_hexdigit())
        && bytes[24..36].iter().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_trace_id_valid_uuid() {
        let args = json!({"__trace_id": "550e8400-e29b-41d4-a716-446655440000", "name": "test"});
        assert_eq!(
            extract_trace_id(&args),
            Some("550e8400-e29b-41d4-a716-446655440000".to_string())
        );
    }

    #[test]
    fn extract_trace_id_rejects_garbage() {
        let args = json!({"__trace_id": "not-a-uuid", "name": "test"});
        assert_eq!(extract_trace_id(&args), None);
    }

    #[test]
    fn extract_trace_id_none_when_missing() {
        let args = json!({"name": "test"});
        assert_eq!(extract_trace_id(&args), None);
    }

    #[test]
    fn extract_trace_id_none_when_not_string() {
        let args = json!({"__trace_id": 42});
        assert_eq!(extract_trace_id(&args), None);
    }

    #[test]
    fn trace_ipc_span_carries_id() {
        let span = trace_ipc_span("platform_add", "abc-123");
        let _enter = span.enter();
        tracing::info!("test event");
        // If we got here without panicking, the span works.
    }

    #[test]
    fn is_uuid_v4_rejects_v1() {
        assert!(!is_uuid_v4("550e8400-e29b-11d4-a716-446655440000"));
    }

    #[test]
    fn is_uuid_v4_accepts_valid_v4() {
        assert!(is_uuid_v4("550e8400-e29b-41d4-a716-446655440000"));
    }
}
