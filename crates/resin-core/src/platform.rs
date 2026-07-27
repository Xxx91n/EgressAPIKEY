//! Platform / Account model (Resin architecture, Rust re-implementation).
//!
//! A Platform is one provider/target dimension (OpenAI, Anthropic, Azure, ...)
//! with its own routable account set. Each Account owns an anchored exit IP
//! (sticky for the account's lifetime unless evicted). Mirrors Resin's
//! Platform/Account two-layer isolation without the Go xsync lease table.

use dashmap::DashMap;
use std::sync::Arc;
use parking_lot::RwLock;

 /// One anchored exit-IP account inside a Platform.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Account {
    pub id: String,
    pub platform: String,
    /// Anchored exit IP (taken from mihomo lane mapping). Sticky by contract.
    pub exit_ip: Option<String>,
    /// Lane index this account is bound to.
    pub lane: usize,
    pub active: bool,
}

impl Account {
    pub fn new(id: impl Into<String>, platform: impl Into<String>, lane: usize) -> Self {
        Self { id: id.into(), platform: platform.into(), exit_ip: None, lane, active: true }
    }

    pub fn bind_ip(&mut self, ip: impl Into<String>) {
        self.exit_ip = Some(ip.into());
    }

    pub fn anchor_ip(&self) -> Option<&str> {
        self.exit_ip.as_deref()
    }
}

/// A Platform owns a routable account set independent of other Platforms.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Platform {
    pub name: String,
    pub accounts: Vec<Account>,
}

impl Platform {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), accounts: Vec::new() }
    }

    pub fn add_account(&mut self, account: Account) {
        self.accounts.push(account);
    }

    /// Pick a routable account: deterministic lane-preferring selection.
    ///
    /// Used when no latency signal is available (cold start / no TD-EWMA yet).
    /// Prefer accounts already bound to `prefer_lane`, then the lowest lane
    /// index — a stable, reproducible fallback that keeps SSE stickiness for a
    /// key's lane. Same behaviour as before; renamed docstring to reflect the
    /// new weighted sibling below.
    pub fn pick_account(&self, prefer_lane: usize) -> Option<&Account> {
        self.accounts
            .iter()
            .filter(|a| a.active)
            .min_by_key(|a| (a.lane != prefer_lane, a.lane))
    }

    /// Pick a routable account with TD-EWMA weighting (Resin's True P2C path).
    ///
    /// Resin's selection prefers lanes with the lowest latency EMA; a lane
    /// with no sample yet (fresh) is treated as more attractive than any lane
    /// that already has a (potentially high) EMA, so the scheduler can
    /// explore cold lanes before falling back to the proven-fast one. This is
    /// implemented as a deterministic min-by-key over the candidate set — no
    /// `rand` dependency — using a single tuple ordering:
    ///
    ///   (has_sample? 1 : 0, latency_ms_or_0, lane != prefer_lane? 1 : 0, lane)
    ///
    /// Lower tuple wins. `latency_for` is supplied by the caller (the
    /// Tauri command or gateway) so this module does NOT import TdEwma; the
    /// weighting source is pluggable and `platform` stays a leaf module.
    pub fn pick_account_weighted<'a, F>(
        &'a self,
        prefer_lane: usize,
        latency_for: F,
    ) -> Option<&'a Account>
    where
        F: Fn(&Account) -> Option<f64>,
    {
        self.accounts
            .iter()
            .filter(|a| a.active)
            .min_by_key(|a| {
                let sample = latency_for(a);
                let has_sample = if sample.is_some() { 1u8 } else { 0u8 };
                let latency = sample.map(|m| m.round() as u64).unwrap_or(0u64);
                let lane_match = if a.lane == prefer_lane { 0u8 } else { 1u8 };
                (has_sample, latency, lane_match, a.lane)
            })
    }
}

/// Registry of platforms. Concurrency-safe via dashmap + RwLock per platform.
#[derive(Default)]
pub struct PlatformRegistry {
    platforms: DashMap<String, Arc<RwLock<Platform>>>,
}

impl PlatformRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&self, p: Platform) {
        self.platforms.insert(p.name.clone(), Arc::new(RwLock::new(p)));
    }

    pub fn get(&self, name: &str) -> Option<Arc<RwLock<Platform>>> {
        self.platforms.get(name).map(|e| e.value().clone())
    }

    pub fn list(&self) -> Vec<String> {
        self.platforms.iter().map(|e| e.value().read().name.clone()).collect()
    }

    pub fn remove(&self, name: &str) -> bool {
        self.platforms.remove(name).is_some()
    }

    /// Bind an account's anchored exit IP. Mirrors Resin's Account/IP lease
    /// binding: the IP is sticky for the account unless explicitly evicted.
    pub fn bind_ip(&self, platform: &str, account: &str, ip: &str) -> bool {
        if let Some(p) = self.get(platform) {
            {
                let mut guard = p.write();
                if let Some(a) = guard.accounts.iter_mut().find(|a| a.id == account) {
                    a.bind_ip(ip);
                    return true;
                }
            }
        }
        false
    }

    pub fn account_snapshot(&self, platform: &str) -> Option<Vec<Account>> {
        self.get(platform).map(|p| p.read().accounts.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_anchor_roundtrip() {
        let mut a = Account::new("acc-1", "openai", 0);
        assert!(a.anchor_ip().is_none());
        a.bind_ip("203.0.113.5");
        assert_eq!(a.anchor_ip(), Some("203.0.113.5"));
    }

    /// Re3: TD-EWMA weighted pick chooses the cold lane over a saturated one.
    #[test]
    fn pick_account_weighted_prefers_unsampled_lane() {
        let mut p = Platform::new("openai");
        // lane 2 has a measured 800ms EMA, lane 3 is fresh (no sample yet).
        p.add_account(Account::new("acct-2", "openai", 2));
        p.add_account(Account::new("acct-3", "openai", 3));

        let pick = p
            .pick_account_weighted(0, |a| {
                if a.lane == 2 { Some(800.0) } else { None }
            })
            .unwrap();
        assert_eq!(pick.lane, 3, "fresh (unmeasured) lane must beat slow sampled lane");
    }

    /// Re3: When all candidates are sampled, the lowest EMA wins.
    #[test]
    fn pick_account_weighted_prefers_lower_latency() {
        let mut p = Platform::new("openai");
        p.add_account(Account::new("slow", "openai", 1));
        p.add_account(Account::new("fast", "openai", 4));

        let pick = p
            .pick_account_weighted(0, |a| match a.lane {
                1 => Some(900.0),
                4 => Some(120.0),
                _ => None,
            })
            .unwrap();
        assert_eq!(pick.lane, 4, "lower EMA lane should win");
    }

    /// Re3: Ties on (sample, latency) fall back to lane preference — preserving
    /// the SSE stickiness invariant: the same key always lands on the same lane.
    #[test]
    fn pick_account_weighted_lane_match_breaks_ties() {
        let mut p = Platform::new("openai");
        p.add_account(Account::new("a", "openai", 5));
        p.add_account(Account::new("b", "openai", 7));

        let pick = p
            .pick_account_weighted(5, |a| match a.lane {
                5 => Some(100.0),
                7 => Some(100.0),
                _ => None,
            })
            .unwrap();
        assert_eq!(pick.lane, 5, "tied latency must break on preferred lane");
    }

    #[test]
    fn platform_pick_prefers_preferred_lane() {
        let mut p = Platform::new("openai");
        p.add_account(Account::new("a", "openai", 3));
        p.add_account(Account::new("b", "openai", 1));
        let pick = p.pick_account(1).unwrap();
        assert_eq!(pick.lane, 1);
    }

    #[test]
    fn platform_pick_falls_back_to_other_active_when_no_lane_match() {
        let mut p = Platform::new("openai");
        p.add_account(Account::new("a", "openai", 3));
        p.add_account(Account::new("c", "openai", 7));
        let pick = p.pick_account(1).unwrap();
        // No account on lane 1; pick any active account deterministically (lowest lane).
        assert!(pick.active);
        assert_eq!(pick.lane, 3);
    }

    #[test]
    fn registry_roundtrip_and_bind() {
        let r = PlatformRegistry::new();
        let mut p = Platform::new("anthropic");
        p.add_account(Account::new("x", "anthropic", 0));
        r.upsert(p);
        assert_eq!(r.list(), vec!["anthropic".to_string()]);
        assert!(r.bind_ip("anthropic", "x", "198.51.100.1"));
        let snap = r.account_snapshot("anthropic").unwrap();
        assert_eq!(snap[0].anchor_ip(), Some("198.51.100.1"));
    }

    #[test]
    fn registry_remove() {
        let r = PlatformRegistry::new();
        r.upsert(Platform::new("test"));
        assert!(r.remove("test"));
        assert!(r.list().is_empty());
    }

    #[test]
    fn inactive_accounts_not_picked() {
        let mut p = Platform::new("p");
        let mut aa = Account::new("a", "p", 0);
        aa.active = false;
        p.add_account(aa);
        p.add_account(Account::new("b", "p", 1));
        let pick = p.pick_account(0).unwrap();
        assert_eq!(pick.id, "b");
    }
}
