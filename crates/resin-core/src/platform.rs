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

    /// Pick a routable account (power-of-two-choices-lite): sample two and
    /// return the one with the lower lane index when both active. Resin uses
    /// True TD-EWMA + P2C; here we expose a deterministic selection so the
    /// scheduler has a fallback path before full TD-EWMA wiring.
    pub fn pick_account(&self, prefer_lane: usize) -> Option<&Account> {
        self.accounts
            .iter()
            .filter(|a| a.active)
            .min_by_key(|a| (a.lane != prefer_lane, a.lane))
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
