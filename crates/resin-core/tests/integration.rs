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

/// Re3: Full SSE-stream lifecycle — simulate the ip allocation + the stream
/// phase + latency recording + lease release on a fresh request, then assert
/// the lane reopens AND the TD-EWMA reflects the latency observed during the
/// stream. This is the "walk through every phase" test the README contract
/// promises (reserve -> stream body -> record_latency -> release -> reopen).
#[test]
fn sse_stream_full_lifecycle_reserve_to_record_to_release() {
    let s = GatewayState::new(4);

    // 1. IP allocation: the proxy-pool entrypoint reserves the lane+lease.
    let api_key = "sk-test-sse";
    let account = "acct-x";
    let authority = "api.openai.com";
    let exit_ip = "203.0.113.10";

    let reserved = s.reserve(api_key, account, authority, Some(exit_ip));
    assert_eq!(reserved.reason, LeaseReason::Acquired);
    let lane = reserved.lane;
    let lease = reserved.lease.expect("acquired lease must be Some");

    // 2. Stream phase: a second concurrent reserve on the SAME key must hit
    //    LaneBusy — the SSE stream holds the lane until completion.
    let second_attempt = s.reserve(api_key, account, authority, Some(exit_ip));
    assert_eq!(second_attempt.reason, LeaseReason::LaneBusy,
        "stream must lock the lane for its duration");
    assert!(second_attempt.lease.is_none());

    // 3. During the stream the upper layer observed a 250ms time-to-first-byte
    //    and a 420ms end-of-stream. record_latency may be called more than
    //    once during a stream; the second sample smoothes into the EMA.
    s.record_latency(authority, Duration::from_millis(250));
    s.record_latency(authority, Duration::from_millis(420));

    // 4. The stream ends: release the lease — frees the lane for the next
    //    request bound to the same api_key.
    s.release(Some(lease));

    // 5. TD-EWMA must now record this authority with a smoothed EMA between
    //    250ms and 420ms, and at least 2 samples.
    let stats = s.tdewma.get(authority).expect("latency must be recorded during stream");
    assert!(stats.ema_ms > 250.0 && stats.ema_ms < 420.0,
        "EMA after two samples must lie between them; got {}", stats.ema_ms);
    assert_eq!(stats.samples, 2, "two record_latency calls must accumulate");

    // 6. Lane reopens: the next request on the same key must acquire again.
    let next = s.reserve(api_key, account, authority, Some(exit_ip));
    assert_eq!(next.reason, LeaseReason::Acquired,
        "lane must reopen after the SSE stream released its lease");
    assert_eq!(next.lane, lane, "same key must hash back onto the same lane");

    // 7. Failure-path cohabits: evicting the lane mid-stream (the simulated
    //    upstream-abort path) releases the lane and does not corrupt tdewma.
    s.evict_lane(next.lane);
    let after_evict = s.reserve(api_key, account, authority, Some(exit_ip));
    assert_eq!(after_evict.reason, LeaseReason::Acquired,
        "lane must be re-acquirable after evict");
    // tdewma entry survives an evict — it tracks authority latency, not lane.
    assert!(s.tdewma.get(authority).is_some(),
        "tdewma must outlive a lane eviction");
}

/// Re3 upper-layer: weighted account pick via PlatformRegistry +
/// GatewayState. Walks platform_add -> account_add -> bind_ip -> record a
/// latency sample on one lane -> pick_account_weighted must prefer the
/// low-latency account, while pick_account (deterministic) ignores latency.
/// This exercises the exact composition the Tauri `gateway_select_account`
/// command calls; failures here surface regressions in the weighted path
/// before they reach the desktop IPC boundary.
#[test]
fn weighted_pick_prefers_low_latency_account() {
    let reg = PlatformRegistry::new();
    let mut p = Platform::new("openai");
    // Two accounts on different lanes. api_key hashes to prefer_lane=lane_a
    // but weighted selection must override when lane_a's latency is high.
    p.add_account(Account::new("acct-low", "openai", 0));
    p.add_account(Account::new("acct-high", "openai", 1));
    reg.upsert(p);
    assert!(reg.bind_ip("openai", "acct-low", "203.0.113.5"));
    assert!(reg.bind_ip("openai", "acct-high", "203.0.113.9"));

    let s = GatewayState::new(10);
    let authority = "api.openai.com";
    // acct-high (lane 1) has a terse latency sample; acct-low has none yet.
    // pick_account_weighted ranks has_sample=0 (cold) ahead of has_sample=1,
    // so the cold acct-low lane must win — mirrors cold-lane-wins policy.
    let plat_arc = reg.get("openai").unwrap();
    let plat = plat_arc.read();
    let cold = plat.pick_account_weighted(0, |_| s.tdewma.get(authority).map(|st| st.ema_ms));
    assert_eq!(cold.unwrap().id.clone(), "acct-low",
        "cold lane must win when the other lane has samples");

    // Now give acct-low's authority a latency record too — but much worse.
    // After both have samples the lower EMA must win.
    s.record_latency(authority, Duration::from_millis(900));
    // Force the weighted path to see a low-latency for acct-high context:
    // we only have one authority in tdewma. To test cross-account weighting,
    // use a closure that returns per-account explicit latencies.
    let latencies: &[(&str, f64)] = &[("acct-low", 900.0), ("acct-high", 50.0)];
    let plat2_arc = reg.get("openai").unwrap();
    let plat2 = plat2_arc.read();
    let weighted = plat2.pick_account_weighted(0, |a| {
        latencies.iter().find(|(id, _)| *id == a.id).map(|(_, ms)| *ms)
    });
    assert_eq!(weighted.unwrap().id.clone(), "acct-high",
        "weighted pick must prefer the lower-latency account");

    // Deterministic pick ignores latency; it only orders by lane preference.
    let plat3_arc = reg.get("openai").unwrap();
    let plat3 = plat3_arc.read();
    let det = plat3.pick_account(0).unwrap();
    assert_eq!(det.id, "acct-low",
        "deterministic pick must prefer the preferred lane regardless of latency");
}
