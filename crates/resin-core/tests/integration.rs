//! Integration coverage for the Platform/Account registry composition the
//! shell uses (SharedRegistry in src-tauri/src/lib.rs). The former
//! lane/lease/tdewma/gateway composition tests were deleted together with
//! those modules (ADR-0050); platform behaviour also stays covered by the
//! module unit tests in src/platform.rs.

use resin_core::platform::{Account, Platform, PlatformRegistry};

#[test]
fn platform_account_lifecycle_upsert_bind_snapshot() {
    let reg = PlatformRegistry::new();
    let mut p = Platform::new("openai");
    p.add_account(Account::new("acct", "openai", 0));
    reg.upsert(p);
    assert!(reg.bind_ip("openai", "acct", "203.0.113.7"));

    let snap = reg.account_snapshot("openai").unwrap();
    assert_eq!(snap[0].anchor_ip(), Some("203.0.113.7"));
    assert!(reg.list().contains(&"openai".to_string()));
}

#[test]
fn weighted_pick_prefers_low_latency_account() {
    // Mirrors the shell SharedRegistry usage: the registry hands out
    // Arc<RwLock<Platform>> and the latency source is injected as a closure.
    let reg = PlatformRegistry::new();
    let mut p = Platform::new("openai");
    p.add_account(Account::new("acct-low", "openai", 0));
    p.add_account(Account::new("acct-high", "openai", 1));
    reg.upsert(p);

    let plat = reg.get("openai").unwrap();
    let plat = plat.read();
    // Cold lane wins while the other lane already has a (worse) sample.
    let cold = plat.pick_account_weighted(0, |a| {
        if a.id == "acct-high" { Some(900.0) } else { None }
    });
    assert_eq!(cold.unwrap().id, "acct-low");

    // With both lanes sampled, the lower EMA must win.
    let latencies: &[(&str, f64)] = &[("acct-low", 900.0), ("acct-high", 50.0)];
    let weighted = plat.pick_account_weighted(0, |a| {
        latencies
            .iter()
            .find(|(id, _)| *id == a.id)
            .map(|(_, ms)| *ms)
    });
    assert_eq!(weighted.unwrap().id, "acct-high");
}

#[test]
fn deterministic_pick_ignores_latency_and_prefers_lane() {
    let reg = PlatformRegistry::new();
    let mut p = Platform::new("openai");
    p.add_account(Account::new("acct-low", "openai", 0));
    p.add_account(Account::new("acct-high", "openai", 1));
    reg.upsert(p);
    let plat = reg.get("openai").unwrap();
    let plat = plat.read();
    let det = plat.pick_account(0).unwrap();
    assert_eq!(det.id, "acct-low");
}
