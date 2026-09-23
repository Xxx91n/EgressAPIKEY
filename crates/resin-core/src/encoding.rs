//! URL percent-encoding helpers shared by every component that builds Resin
//! or upstream-provider URLs (R12-D2 consolidation).
//!
//! Two encode sets, two names, zero ambiguity:
//!
//! - [`encode_path_segment`] - strict set: RFC 3986 alphanumerics plus `-`
//!   and `_` literal; every other UTF-8 byte becomes `%XX`. Used for Resin
//!   path segments AND query values (over-encoding a query value is always
//!   safe). A space encodes to `%20`, never the form-urlencoded `+`.
//! - [`encode_uri_component`] - byte-for-byte JS `encodeURIComponent` parity
//!   (unreserved set `- _ . ! ~ * ' ( )`), kept for the account-header-rule
//!   `url_prefix` contract that upstream tests exercise verbatim.
//!
//! The name `urlencoding` is deliberately absent: the form-vs-path
//! ambiguity it carried is what shipped the IPQS key-in-path bug (a `+` in
//! place of `%20` inside a path segment).

/// Strict path-segment encode set: alphanumerics + `-`/`_` only.
const PATH_SEGMENT_ENCODE_SET: &percent_encoding::AsciiSet =
    &percent_encoding::NON_ALPHANUMERIC.remove(b'-').remove(b'_');

/// Percent-encode one URL path segment (strict set): ` ` -> `%20`,
/// `+` -> `%2B`, `/` -> `%2F`. Never emits the form-urlencoded `+`.
pub fn encode_path_segment(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, PATH_SEGMENT_ENCODE_SET).to_string()
}

/// JS `encodeURIComponent` unreserved set: A-Z a-z 0-9 - _ . ! ~ * ' ( ).
const URI_COMPONENT_ENCODE_SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// `encodeURIComponent`-compatible encoder for the account-header-rule
/// `url_prefix` path segment. The upstream contract test exercises
/// `api.example.com%2Fv1` - the `/` MUST stay percent-encoded so the Go 1.22
/// ServeMux `{prefix...}` wildcard receives one segment and unescapes it back
/// to `api.example.com/v1`. Byte-for-byte identical to JS
/// `encodeURIComponent` (same unreserved set, UTF-8 bytes) - the proven-good
/// encoding against this exact server.
pub fn encode_uri_component(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, URI_COMPONENT_ENCODE_SET).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_path_segment_strict_set_vectors() {
        // space -> %20 (NOT `+`: the form-urlencoded bug R12-D2 fixes)
        assert_eq!(encode_path_segment("a b"), "a%20b");
        // a literal `+` is data -> %2B
        assert_eq!(encode_path_segment("a+b"), "a%2Bb");
        // `/` inside a segment -> %2F
        assert_eq!(encode_path_segment("a/b"), "a%2Fb");
        // unreserved stays literal
        assert_eq!(encode_path_segment("abc-123_xyz"), "abc-123_xyz");
    }

    #[test]
    fn encode_path_segment_non_ascii_utf8() {
        // multi-byte UTF-8 is percent-encoded byte by byte
        assert_eq!(encode_path_segment("名前"), "%E5%90%8D%E5%89%8D");
    }

    #[test]
    fn encode_uri_component_matches_js_unreserved_set() {
        // moved verbatim from resin_client.rs (byte-equivalence contract)
        assert_eq!(
            encode_uri_component("api.example.com/v1"),
            "api.example.com%2Fv1"
        );
        // encodeURIComponent keeps - _ . ! ~ * ' ( ) literal.
        assert_eq!(
            encode_uri_component("a-b_c.d!e~f*g'h(i)"),
            "a-b_c.d!e~f*g'h(i)"
        );
        assert_eq!(encode_uri_component("a b"), "a%20b");
    }
}
