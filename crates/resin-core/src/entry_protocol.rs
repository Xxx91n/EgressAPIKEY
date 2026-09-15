//! Entry-port protocol vocabulary - /, ADR-0068 D3.
//!
//! An Entry Port declares exactly ONE of three protocol values, and `mixed`
//! is the default. `mixed` is a single listener that accepts both HTTP-proxy
//! and SOCKS5 clients: Resin opens `allow_http_forward` and `allow_socks5`
//! together and disambiguates on the connection's first byte (0x05 = SOCKS5,
//! otherwise HTTP). The live same-port behaviour - including the per-protocol
//! refusal the tightened mapping relies on - was verified against a running
//! Resin before this module landed (ADR-0068 D4 gate; raw evidence in
//! `.scratch//repro/d4-mixed/`).
//!
//! This module is the single source of truth for two things that used to be
//! spread across the codebase and therefore drifted apart:
//!
//!   1. the CLOSED value set the whitebox validator and the IPC / headless
//!      HTTP boundaries enforce (`canonical_protocol` / `is_valid_protocol`),
//!      and
//!   2. the Resin endpoint capability flags derived from the token
//!      (`engine_flags`) - previously hand-rolled at four separate call sites.
//!
//! Behavioural change: `socks5` NO LONGER implies HTTP forwarding.
//! Before the change a `socks5` port produced (allow_socks5 = true,
//! allow_http_forward = true), i.e. it behaved as `mixed`; a `socks5` port is
//! now SOCKS5-only and `mixed` is the only way to serve both dialects. Legacy
//! documents are therefore migrated `socks5` -> `mixed` so their effective
//! engine flags are unchanged (see
//! `whitebox_config::migrate_entry_port_protocols`).

/// The closed value set. `mixed` is listed first because it is the default.
pub const ENTRY_PORT_PROTOCOLS: [&str; 3] = ["mixed", "http", "socks5"];

/// Protocol assigned to a new Entry Port when the caller expresses no
/// preference (`mixed` is the default).
pub const DEFAULT_ENTRY_PORT_PROTOCOL: &str = "mixed";

/// The one message every boundary returns for an out-of-set token. Kept here so
/// the whitebox validator, the shell command layer, the headless HTTP layer and
/// the frontend guard cannot drift apart in wording or in the values they name.
pub const ENTRY_PORT_PROTOCOL_ERROR: &str = "protocol must be socks5, http or mixed";

/// Canonical spelling of `raw`, or `None` when the token is outside the closed
/// set. Case-insensitive and whitespace-tolerant; an unknown token is REJECTED
/// rather than coerced, because a closed value set is what keeps drift
/// detectable.
pub fn canonical_protocol(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "mixed" => Some("mixed"),
        "http" => Some("http"),
        "socks5" => Some("socks5"),
        _ => None,
    }
}

/// Whether `raw` names a supported entry-port protocol.
pub fn is_valid_protocol(raw: &str) -> bool {
    canonical_protocol(raw).is_some()
}

/// Resin endpoint capability flags for an entry-port protocol token, returned
/// as `(allow_socks5, allow_http_forward)`.
///
/// | protocol | allow_socks5 | allow_http_forward | listener      |
/// |----------|--------------|--------------------|---------------|
/// | `mixed`  | true         | true               | HTTP + SOCKS5 |
/// | `http`   | false        | true               | HTTP only     |
/// | `socks5` | true         | false              | SOCKS5 only   |
///
/// TOTAL by design: every boundary that can introduce a token rejects
/// out-of-set values first (`canonical_protocol` in the whitebox validator, the
/// shell and headless port validators, and the frontend guards), so this
/// function is the ONE derivation every endpoint body uses. The four
/// hand-rolled copies it replaced are exactly how the mapping drifted. A token
/// that somehow bypasses the boundaries (a hand-edited SQLite row) takes the
/// declared default `mixed` - the permissive dual listener - instead of a
/// silently half-dead port.
pub fn engine_flags(raw: &str) -> (bool, bool) {
    match canonical_protocol(raw) {
        Some("http") => (false, true),
        Some("socks5") => (true, false),
        _ => (true, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The declared default must be a member of the closed set, otherwise
    /// "default" would name a value the validator rejects.
    #[test]
    fn default_is_a_member_of_the_closed_set() {
        assert!(is_valid_protocol(DEFAULT_ENTRY_PORT_PROTOCOL));
        assert_eq!(ENTRY_PORT_PROTOCOLS.len(), 3);
    }

    #[test]
    fn canonical_protocol_accepts_the_three_values_and_normalises() {
        assert_eq!(canonical_protocol("mixed"), Some("mixed"));
        assert_eq!(canonical_protocol("HTTP"), Some("http"));
        assert_eq!(canonical_protocol("  Socks5 "), Some("socks5"));
    }

    /// An unknown token is rejected, never coerced - the same discipline the
    /// strategy vocabulary adopted.
    #[test]
    fn canonical_protocol_rejects_out_of_set_tokens() {
        for bad in ["", "socks", "socks4", "https", "auto", "mixed5", "0x05"] {
            assert_eq!(canonical_protocol(bad), None, "token must be rejected");
            assert!(!is_valid_protocol(bad));
        }
    }

    /// The flag table, locked against what the ADR-0068 D4 gate observed live
    /// against a running Resin: the dual-flag port answered BOTH dialects, the
    /// http-only port refused SOCKS5 (05 ff) and the socks5-only port refused
    /// HTTP (403 ENDPOINT_CAPABILITY_DISABLED).
    #[test]
    fn engine_flags_table_matches_the_d4_gate_observation() {
        assert_eq!(engine_flags("mixed"), (true, true));
        assert_eq!(engine_flags("http"), (false, true));
        assert_eq!(engine_flags("socks5"), (true, false));
        // Case/whitespace tolerance and the total fallback.
        assert_eq!(engine_flags("MIXED"), (true, true));
        assert_eq!(engine_flags(" socks5 "), (true, false));
        assert_eq!(engine_flags("nonsense"), (true, true));
    }
}
