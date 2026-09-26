//! Headless control-surface security primitives.
//!
//! The headless binary (`egressapikey-headless`) exposes the FULL Resin admin
//! control plane over HTTP. Two controls keep that surface unreachable from a
//! hostile origin:
//!
//! 1. Shared-secret token (`--auth-token`). Required on every `/api/v1/*` and
//!    `/metrics/*` request. Binding a non-loopback address without a token
//!    refuses startup.
//! 2. Host / Origin allowlist. Rejects any request whose `Host` (or `Origin`)
//!    is neither a loopback name nor an explicitly declared host. This is the
//!    same mitigation Caddy / Envoy / nginx apply against DNS rebinding: an
//!    attacker page rebinds its own hostname to 127.0.0.1, so the browser keeps
//!    sending the attacker's `Host` while the TCP connection lands on the local
//!    control plane.
//!
//! The token comes from the OS CSPRNG (`getrandom`). Time + PID entropy is
//! explicitly rejected: it is guessable from the process start window and is not
//! a cryptographic source.
//!

/// Number of CSPRNG bytes per token, before hex encoding (32 bytes = 256 bits).
const TOKEN_BYTES: usize = 32;

/// Cookie that carries the token after a `?auth_token=` bootstrap handshake.
pub const TOKEN_COOKIE: &str = "egressapikey_token";

/// Query parameter used for the one-shot bootstrap handshake.
pub const TOKEN_QUERY: &str = "auth_token";

/// Where the process token came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    /// Supplied by the operator via `--auth-token`.
    Provided,
    /// Generated from the OS CSPRNG because the bind address is loopback.
    Generated,
}

/// The token this process enforces, plus how it was obtained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedToken {
    /// The secret itself. This module never logs it.
    pub token: String,
    /// Provenance, for the startup banner.
    pub source: TokenSource,
}

/// True when `bind` denotes a loopback interface (so an auto-generated token is
/// acceptable). `localhost` is accepted as a convenience alias.
pub fn is_loopback_bind(bind: &str) -> bool {
    let b = bind.trim();
    if b.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let inner = b
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(b);
    inner
        .parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// Draw a fresh token from the OS CSPRNG. This is the ONLY sanctioned source:
/// time + PID entropy is explicitly rejected (guessable from the start window).
pub fn generate_token() -> Result<String, String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| format!("headless: OS CSPRNG unavailable: {e}"))?;
    Ok(hex::encode(bytes))
}

/// Startup gate. Decides the process token, or refuses to start:
///
/// * `--auth-token` supplied (non-blank) -> use it, on any bind address.
/// * omitted + loopback bind -> generate with the OS CSPRNG.
/// * omitted + non-loopback bind -> REFUSE. An unauthenticated admin control
///   plane must never be exposed off-host.
pub fn resolve_token(cli_token: Option<&str>, bind: &str) -> Result<ResolvedToken, String> {
    let provided = cli_token.map(str::trim).filter(|t| !t.is_empty());
    match provided {
        Some(t) => Ok(ResolvedToken {
            token: t.to_string(),
            source: TokenSource::Provided,
        }),
        None => {
            if cli_token.is_some() {
                return Err("headless: --auth-token must not be blank".to_string());
            }
            if is_loopback_bind(bind) {
                Ok(ResolvedToken {
                    token: generate_token()?,
                    source: TokenSource::Generated,
                })
            } else {
                Err(format!(
                    "headless: refusing to start: --bind {bind} is not a loopback address and no --auth-token was supplied; the headless control surface exposes the full admin plane, so pass --auth-token <secret> or bind 127.0.0.1"
                ))
            }
        }
    }
}

/// Build the `Host` / `Origin` allowlist: loopback names, plus the bind address
/// when it is a concrete (non-unspecified) IP, plus operator-declared hosts
/// (`--allowed-host`). Everything is lowercased.
pub fn allowed_hosts(bind: &str, declared: &[String]) -> Vec<String> {
    let mut hosts: Vec<String> = vec![
        "127.0.0.1".to_string(),
        "localhost".to_string(),
        "::1".to_string(),
    ];
    let b = bind.trim();
    let inner = b
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(b);
    if let Ok(ip) = inner.parse::<std::net::IpAddr>() {
        // An unspecified bind (0.0.0.0 / ::) tells us nothing about the name
        // clients will use, so it must NOT widen the allowlist.
        if !ip.is_unspecified() {
            push_unique(&mut hosts, ip.to_string());
        }
    }
    for host in declared {
        push_unique(&mut hosts, host.trim().to_ascii_lowercase());
    }
    hosts
}

fn push_unique(hosts: &mut Vec<String>, host: String) {
    if !host.is_empty() && !hosts.iter().any(|h| h == &host) {
        hosts.push(host);
    }
}

/// Normalize a `Host` header value or an `Origin` URL to a bare lowercase
/// hostname: strips scheme, userinfo, path/query/fragment, port and IPv6
/// brackets, and a trailing root dot.
pub fn hostname_of(value: &str) -> String {
    let v = value.trim();
    let v = match v.find("://") {
        Some(i) => &v[i + 3..],
        None => v,
    };
    let v = v
        .split(|c| c == '/' || c == '?' || c == '#')
        .next()
        .unwrap_or(v);
    let v = match v.rfind('@') {
        Some(i) => &v[i + 1..],
        None => v,
    };
    let v = v.trim();
    if let Some(rest) = v.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return rest[..end].to_ascii_lowercase();
        }
    }
    let host = match v.rfind(':') {
        Some(i) if !v[..i].contains(':') => &v[..i],
        _ => v,
    };
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// Accept the request when both `Host` and (when present) `Origin` resolve to an
/// allowlisted hostname. Returns the rejection reason otherwise.
pub fn host_allowed(
    host_header: Option<&str>,
    origin_header: Option<&str>,
    allowed: &[String],
) -> Result<(), String> {
    let host = host_header.map(hostname_of).unwrap_or_default();
    if host.is_empty() {
        return Err("missing Host header".to_string());
    }
    if !allowed.iter().any(|a| a == &host) {
        return Err(format!("unexpected Host {host:?} (DNS-rebinding guard)"));
    }
    if let Some(origin) = origin_header {
        if !origin.trim().is_empty() {
            let origin_host = hostname_of(origin);
            if !allowed.iter().any(|a| a == &origin_host) {
                return Err(format!(
                    "unexpected Origin host {origin_host:?} (DNS-rebinding guard)"
                ));
            }
        }
    }
    Ok(())
}

/// Length-checked, content-constant-time byte comparison (no early exit).
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Extract the token from `Authorization: Bearer <token>`.
pub fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, rest) = value.trim().split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") {
        let token = rest.trim();
        if token.is_empty() {
            None
        } else {
            Some(token)
        }
    } else {
        None
    }
}

/// Extract `name=value` from a `Cookie` header (first match).
pub fn cookie_value<'a>(cookie_header: &'a str, name: &str) -> Option<&'a str> {
    for part in cookie_header.split(';') {
        if let Some((key, value)) = part.trim().split_once('=') {
            if key.trim() == name {
                return Some(value.trim());
            }
        }
    }
    None
}

/// Extract a parameter from a raw query string (`a=1&b=2`). No percent-decoding:
/// the generated token is hex, and the bootstrap URL is built by this process.
/// Distinct from headless_main::query_param_decoded, which percent-decodes
/// its matches for the BFF translation routes - the guard's token compare
/// must see the raw bytes.
pub fn query_param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            if key == name {
                return Some(value);
            }
        }
    }
    None
}

/// `Set-Cookie` value storing the token for subsequent same-origin requests.
/// Deliberately no `Secure` flag: headless deployments commonly terminate TLS at
/// a reverse proxy, and `Secure` would silently break plain-HTTP LAN access.
pub fn token_cookie(token: &str) -> String {
    format!("{TOKEN_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict")
}

/// True only when the socket peer is a declared `--trusted-proxy` AND the
/// LAST (rightmost) segment of the merged `X-Forwarded-Proto` value is
/// `https`; a last segment outside the literal {`http`,`https`} pair
/// fails closed (not secure).
///
/// XFP is a client-writable header: trusting it unconditionally let any
/// caller forge the `Secure` suffix onto the session cookie on a plain-HTTP
/// deployment (r12 wave-d R12-D1, ADR-0071 errata). The socket peer - not
/// the XFF chain - is the trust anchor: the guard only needs "did a trusted
/// proxy relay this request", never the real client IP. Every header
/// instance is merged in wire order and only the rightmost segment is
/// consulted: that segment is the scheme the trusted hop NEAREST this
/// process observed - earlier segments are client-suppliable and never
/// rescue a non-https tail (r12 wave-i D-002; same direction as ASP.NET
/// ForwardLimit=1 / Envoy xff_num_trusted_hops, which consume XFP/XFF from
/// the right). IPv4-mapped-IPv6 peers (`::ffff:a.b.c.d`) normalize via
/// `to_ipv4_mapped()` - NOT `to_ipv4()`, which also maps `::1` ->
/// `0.0.0.1` and would break the v6 loopback peer.
///
/// `x_forwarded_proto` is every `X-Forwarded-Proto` header instance in wire
/// order (a merged comma-joined value is equivalent); an empty iterator
/// means no header arrived.
pub fn trusted_forwarded_https<'a, I>(
    peer: std::net::IpAddr,
    trusted: &[ipnet::IpNet],
    x_forwarded_proto: I,
) -> bool
where
    I: IntoIterator<Item = &'a str>,
{
    let peer = match peer {
        std::net::IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(std::net::IpAddr::V4)
            .unwrap_or(std::net::IpAddr::V6(v6)),
        v4 => v4,
    };
    if !trusted.iter().any(|net| net.contains(&peer)) {
        return false;
    }
    x_forwarded_proto
        .into_iter()
        .flat_map(|v| v.split(','))
        .next_back()
        .map(|v| v.trim().eq_ignore_ascii_case("https"))
        .unwrap_or(false)
}

/// Token + Host allowlist, evaluated per request by the axum guard in
/// `headless_main.rs`.
#[derive(Debug, Clone)]
pub struct HeadlessGuard {
    allowed_hosts: Vec<String>,
    token: String,
    /// Peers trusted to speak for the client's real scheme (the
    /// `--trusted-proxy` set). `X-Forwarded-Proto` is honoured only when the
    /// socket peer is inside this set - it is never trusted from an
    /// arbitrary client (the header is client-writable).
    trusted_proxies: Vec<ipnet::IpNet>,
}

impl HeadlessGuard {
    pub fn new(
        allowed_hosts: Vec<String>,
        token: String,
        trusted_proxies: Vec<ipnet::IpNet>,
    ) -> Self {
        Self {
            allowed_hosts,
            token,
            trusted_proxies,
        }
    }

    pub fn allowed_hosts(&self) -> &[String] {
        &self.allowed_hosts
    }

    /// The `--trusted-proxy` peer set, consulted by `trusted_forwarded_https`.
    pub fn trusted_proxies(&self) -> &[ipnet::IpNet] {
        &self.trusted_proxies
    }

    /// `Set-Cookie` value that plants the session cookie during the bootstrap
    /// handshake (the token arriving via `?auth_token=`).
    pub fn session_cookie(&self) -> String {
        token_cookie(&self.token)
    }

    /// Constant-time comparison against the process token.
    pub fn token_matches(&self, candidate: &str) -> bool {
        constant_time_eq(self.token.as_bytes(), candidate.as_bytes())
    }

    /// Pull a candidate token from the request carriers, most trusted first.
    /// The `bool` is true when the token arrived via the query string, i.e. the
    /// one-shot bootstrap handshake that also plants the cookie.
    pub fn extract_token<'a>(
        &self,
        authorization: Option<&'a str>,
        cookie: Option<&'a str>,
        query: Option<&'a str>,
    ) -> Option<(&'a str, bool)> {
        if let Some(value) = authorization {
            if let Some(token) = bearer_token(value) {
                return Some((token, false));
            }
        }
        if let Some(header) = cookie {
            if let Some(token) = cookie_value(header, TOKEN_COOKIE) {
                if !token.is_empty() {
                    return Some((token, false));
                }
            }
        }
        if let Some(raw) = query {
            if let Some(token) = query_param(raw, TOKEN_QUERY) {
                if !token.is_empty() {
                    return Some((token, true));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_bind_detection() {
        assert!(is_loopback_bind("127.0.0.1"));
        assert!(is_loopback_bind("127.0.0.2"));
        assert!(is_loopback_bind("::1"));
        assert!(is_loopback_bind("[::1]"));
        assert!(is_loopback_bind("localhost"));
        assert!(is_loopback_bind("LOCALHOST"));
        assert!(is_loopback_bind(" 127.0.0.1 "));
        assert!(!is_loopback_bind("0.0.0.0"));
        assert!(!is_loopback_bind("10.1.2.3"));
        assert!(!is_loopback_bind("::"));
        assert!(!is_loopback_bind("egress.example.com"));
        assert!(!is_loopback_bind(""));
    }

    #[test]
    fn token_is_hex_of_expected_length_and_unique() {
        let a = generate_token().expect("csprng available");
        let b = generate_token().expect("csprng available");
        assert_eq!(a.len(), TOKEN_BYTES * 2);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b, "two CSPRNG draws must differ");
    }

    #[test]
    fn resolve_token_accepts_operator_value_on_any_bind() {
        let r = resolve_token(Some("s3cret"), "0.0.0.0").expect("provided token accepted");
        assert_eq!(r.token, "s3cret");
        assert_eq!(r.source, TokenSource::Provided);
    }

    #[test]
    fn resolve_token_generates_for_loopback() {
        let r = resolve_token(None, "127.0.0.1").expect("loopback auto-generates");
        assert_eq!(r.source, TokenSource::Generated);
        assert_eq!(r.token.len(), TOKEN_BYTES * 2);
    }

    #[test]
    fn resolve_token_refuses_non_loopback_without_token() {
        let err = resolve_token(None, "0.0.0.0").expect_err("must refuse");
        assert!(err.contains("refusing to start"), "got: {err}");
        assert!(resolve_token(None, "203.0.113.9").is_err());
    }

    #[test]
    fn resolve_token_rejects_blank_operator_token() {
        let err = resolve_token(Some("   "), "127.0.0.1").expect_err("blank rejected");
        assert!(err.contains("must not be blank"), "got: {err}");
    }

    #[test]
    fn allowed_hosts_always_contains_loopback_names() {
        let h = allowed_hosts("0.0.0.0", &[]);
        for want in ["127.0.0.1", "localhost", "::1"] {
            assert!(h.iter().any(|x| x.as_str() == want), "missing {want}");
        }
        assert!(
            !h.iter().any(|x| x.as_str() == "0.0.0.0"),
            "an unspecified bind must not widen the allowlist"
        );
    }

    #[test]
    fn allowed_hosts_adds_concrete_bind_and_declared_hosts() {
        let h = allowed_hosts(
            "10.1.2.3",
            &["egress.example.com".to_string(), "Example.ORG".to_string()],
        );
        assert!(h.iter().any(|x| x.as_str() == "10.1.2.3"));
        assert!(h.iter().any(|x| x.as_str() == "egress.example.com"));
        assert!(
            h.iter().any(|x| x.as_str() == "example.org"),
            "declared hosts lowercase"
        );
    }

    #[test]
    fn hostname_of_strips_scheme_port_brackets_and_root_dot() {
        assert_eq!(hostname_of("127.0.0.1:14200"), "127.0.0.1");
        assert_eq!(hostname_of("[::1]:14200"), "::1");
        assert_eq!(hostname_of("::1"), "::1");
        assert_eq!(
            hostname_of("http://egress.example.com:8443"),
            "egress.example.com"
        );
        assert_eq!(hostname_of("https://LOCALHOST"), "localhost");
        assert_eq!(hostname_of("localhost."), "localhost");
        assert_eq!(hostname_of(""), "");
    }

    #[test]
    fn host_allowed_accepts_loopback_and_rejects_foreign() {
        let allowed = allowed_hosts("127.0.0.1", &[]);
        assert!(host_allowed(Some("127.0.0.1:14200"), None, &allowed).is_ok());
        assert!(host_allowed(
            Some("localhost:14200"),
            Some("http://localhost:14200"),
            &allowed
        )
        .is_ok());
        assert!(host_allowed(Some("evil.example:14200"), None, &allowed).is_err());
        assert!(
            host_allowed(None, None, &allowed).is_err(),
            "missing Host rejected"
        );
        assert!(host_allowed(Some("127.0.0.1"), Some("http://evil.example"), &allowed).is_err());
        assert!(
            host_allowed(Some("127.0.0.1"), Some("null"), &allowed).is_err(),
            "opaque Origin rejected"
        );
    }

    #[test]
    fn constant_time_eq_matches_eq_semantics() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn bearer_cookie_and_query_extraction() {
        assert_eq!(bearer_token("Bearer abc123"), Some("abc123"));
        assert_eq!(bearer_token("bearer abc123"), Some("abc123"));
        assert_eq!(bearer_token("Basic abc123"), None);
        assert_eq!(bearer_token("Bearer"), None);
        assert_eq!(
            cookie_value("a=1; egressapikey_token=deadbeef; b=2", TOKEN_COOKIE),
            Some("deadbeef")
        );
        assert_eq!(cookie_value("other=1", TOKEN_COOKIE), None);
        assert_eq!(
            query_param("x=1&auth_token=cafe&y=2", TOKEN_QUERY),
            Some("cafe")
        );
        assert_eq!(query_param("x=1", TOKEN_QUERY), None);
    }

    #[test]
    fn guard_token_precedence_and_match() {
        let g = HeadlessGuard::new(
            allowed_hosts("127.0.0.1", &[]),
            "tok".to_string(),
            Vec::new(),
        );
        assert!(g.token_matches("tok"));
        assert!(!g.token_matches("nope"));
        assert_eq!(
            g.extract_token(Some("Bearer tok"), None, None),
            Some(("tok", false))
        );
        assert_eq!(
            g.extract_token(None, Some("egressapikey_token=tok"), None),
            Some(("tok", false))
        );
        assert_eq!(
            g.extract_token(None, None, Some("auth_token=tok")),
            Some(("tok", true))
        );
        assert_eq!(g.extract_token(None, None, None), None);
    }

    #[test]
    fn cookie_is_httponly_samesite_without_secure() {
        let c = token_cookie("tok");
        assert!(c.starts_with("egressapikey_token=tok;"));
        assert!(c.contains("HttpOnly"));
        assert!(c.contains("SameSite=Strict"));
        assert!(
            !c.contains("Secure"),
            "plain-HTTP LAN deployments must still receive the cookie"
        );
    }

    // -- R12-D1: X-Forwarded-Proto trust boundary (ADR-0071 errata) --

    fn v4(s: &str) -> std::net::IpAddr {
        s.parse().expect("v4 addr")
    }
    fn net(s: &str) -> ipnet::IpNet {
        s.parse().expect("cidr")
    }

    #[test]
    fn xfp_ignored_when_no_trusted_proxy_declared() {
        // No --trusted-proxy flag -> XFP fully ignored even from loopback.
        assert!(!trusted_forwarded_https(
            v4("127.0.0.1"),
            &[],
            Some("https")
        ));
    }

    #[test]
    fn xfp_ignored_for_untrusted_peer() {
        let trusted = vec![net("127.0.0.1/32")];
        assert!(!trusted_forwarded_https(
            v4("10.0.0.9"),
            &trusted,
            Some("https")
        ));
    }

    #[test]
    fn trusted_peer_asserting_https_marks_secure() {
        let trusted = vec![net("127.0.0.1/32")];
        assert!(trusted_forwarded_https(
            v4("127.0.0.1"),
            &trusted,
            Some("https")
        ));
    }

    #[test]
    fn trusted_peer_without_xfp_gets_no_secure() {
        let trusted = vec![net("127.0.0.1/32")];
        assert!(!trusted_forwarded_https(
            v4("127.0.0.1"),
            &trusted,
            None::<&str>
        ));
    }

    #[test]
    fn trusted_peer_xfp_http_gets_no_secure() {
        let trusted = vec![net("127.0.0.1/32")];
        assert!(!trusted_forwarded_https(
            v4("127.0.0.1"),
            &trusted,
            Some("http")
        ));
    }

    #[test]
    fn xfp_list_uses_rightmost_scheme_fail_closed() {
        let trusted = vec![net("127.0.0.1/32")];
        // Rightmost list value = the scheme the nearest trusted hop
        // observed; earlier (client-writable) segments never rescue a
        // non-https tail.
        assert!(!trusted_forwarded_https(
            v4("127.0.0.1"),
            &trusted,
            Some("https,http")
        ));
        assert!(trusted_forwarded_https(
            v4("127.0.0.1"),
            &trusted,
            Some("http,https")
        ));
    }

    #[test]
    fn xfp_multiple_header_instances_merge_in_wire_order() {
        let trusted = vec![net("127.0.0.1/32")];
        // Header instances merge in wire order (get_all); the last segment
        // of the LAST instance is the nearest-hop assertion.
        assert!(trusted_forwarded_https(
            v4("127.0.0.1"),
            &trusted,
            ["http", "https"]
        ));
        assert!(!trusted_forwarded_https(
            v4("127.0.0.1"),
            &trusted,
            ["https", "http"]
        ));
        // A per-instance comma list still resolves on the rightmost segment.
        assert!(trusted_forwarded_https(
            v4("127.0.0.1"),
            &trusted,
            ["http,https", "https"]
        ));
    }

    #[test]
    fn xfp_last_segment_outside_http_https_fails_closed() {
        let trusted = vec![net("127.0.0.1/32")];
        for bad in ["https,gibberish", "https,", " https , http "] {
            assert!(!trusted_forwarded_https(
                v4("127.0.0.1"),
                &trusted,
                Some(bad)
            ));
        }
    }

    #[test]
    fn v6_mapped_peer_hits_v4_trusted_entry() {
        let trusted = vec![net("127.0.0.1/32")];
        let peer: std::net::IpAddr = "::ffff:127.0.0.1".parse().expect("v6-mapped");
        assert!(trusted_forwarded_https(peer, &trusted, Some("https")));
    }

    #[test]
    fn cidr_trusted_entry_hit_and_miss() {
        let trusted = vec![net("10.8.0.0/16")];
        assert!(trusted_forwarded_https(
            v4("10.8.3.4"),
            &trusted,
            Some("https")
        ));
        assert!(!trusted_forwarded_https(
            v4("10.9.0.1"),
            &trusted,
            Some("https")
        ));
    }
}
