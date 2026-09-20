//! T6-4 proxy_e2e: integration tests for the exit-IP probe pipeline.
//!
//! Layer 1 (unit): mockito echo server returns a fake cdn-cgi/trace body;
//!   we verify parse_trace_body_ip extracts the exit IP correctly.
//! Layer 2 (integration, #[ignore]): real probe through the live sidecar
//!   to http://1.1.1.1/cdn-cgi/trace. Requires the desktop app running.

use resin_core::parse_trace_body_ip;

/// Unit: parse_trace_body_ip extracts the IP from a mockito echo body
/// that mimics the Cloudflare trace endpoint response.
#[tokio::test]
async fn mockito_echo_trace_body_parses_exit_ip() {
    let body = "fl=42f\nh=mock.local\nip=203.0.113.99\nuag=mockito/1.0\n";
    let ip = parse_trace_body_ip(body);
    assert_eq!(ip, "203.0.113.99");
}

/// Unit: parse_trace_body_ip handles a body with no ip= line gracefully.
#[tokio::test]
async fn mockito_echo_trace_body_no_ip_returns_empty() {
    let body = "fl=42f\nh=mock.local\nvisited=2026-01-01\n";
    let ip = parse_trace_body_ip(body);
    assert!(ip.is_empty());
}

/// Unit: parse_trace_body_ip trims whitespace around the IP value.
#[tokio::test]
async fn mockito_echo_trace_body_trims_whitespace() {
    let body = "ip=  198.51.100.42  \n";
    let ip = parse_trace_body_ip(body);
    assert_eq!(ip, "198.51.100.42");
}

/// Integration: probe the real Cloudflare trace endpoint through the live
/// sidecar. This requires the desktop app running with a healthy Resin
/// sidecar on 127.0.0.1. Run with: cargo test -- --ignored test_probe_real_1_1_1_1
#[tokio::test]
#[ignore = "requires live sidecar + active entry port"]
async fn test_probe_real_1_1_1_1() {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    let resp = client
        .get("http://1.1.1.1/cdn-cgi/trace")
        .send()
        .await
        .expect("failed to reach 1.1.1.1");
    let body = resp.text().await.expect("failed to read body");
    let ip = parse_trace_body_ip(&body);
    assert!(
        !ip.is_empty(),
        "expected non-empty exit IP from 1.1.1.1/cdn-cgi/trace, got empty. Body: {body}"
    );
}

// reqwest is a workspace dep, available in test scope.
