//! Integration coverage that ties the lane / lease / platform / gateway units
//! together the way the Tauri shell will use them. Stays local (no sockets).

use resin_core::gateway::{GatewayState, LeaseReason};
use resin_core::platform::{Account, Platform, PlatformRegistry};
use resin_core::{lane_index, LaneConfig};
use std::time::Duration;

#[test]
fn gateway_lane_matches_lane_hash() {
    let s = GatewayState::new(10);
    let expected = lane_index("sk-x", &LaneConfig::new(10));
    let r = s.reserve("sk-x", "acct", "api.openai.com", Some("1.2.3.4"));
    assert_eq!(r.lane, expected);
}

#[test]
fn platform_account_lifecycle_with_gateway_reservation() {
    let reg = PlatformRegistry::new();
    let mut p = Platform::new("openai");
    p.add_account(Account::new("acct", "openai", 0));
    reg.upsert(p);
    assert!(reg.bind_ip("openai", "acct", "203.0.113.7"));

    let s = GatewayState::new(10);
    let r = s.reserve("sk-key", "acct", "api.openai.com", Some("203.0.113.7"));
    assert_eq!(r.reason, LeaseReason::Acquired);

    let snap = reg.account_snapshot("openai").unwrap();
    assert_eq!(snap[0].anchor_ip(), Some("203.0.113.7"));
    s.release(r.lease);
}

#[test]
fn sse_stream_lock_then_release_allows_next() {
    let s = GatewayState::new(1);
    let first = s.reserve("k", "a", "api.x.com", Some("1.1.1.1"));
    assert_eq!(first.reason, LeaseReason::Acquired);
    let second = s.reserve("k", "a", "api.x.com", Some("1.1.1.1"));
    assert_eq!(second.reason, LeaseReason::LaneBusy);
    s.release(first.lease);
    let third = s.reserve("k", "a", "api.x.com", Some("1.1.1.1"));
    assert_eq!(third.reason, LeaseReason::Acquired);
}

#[test]
fn failure_path_records_latency_and_evicts_lane() {
    let s = GatewayState::new(1);
    let r = s.reserve("k", "a", "api.x.com", Some("9.9.9.9"));
    s.record_latency("api.x.com", Duration::from_millis(300));
    s.evict_lane(r.lane);
    let after = s.tdewma.get("api.x.com").unwrap();
    assert_eq!(after.ema_ms, 300.0);
    let next = s.reserve("k", "a", "api.x.com", Some("9.9.9.9"));
    assert_eq!(next.reason, LeaseReason::Acquired);
}
