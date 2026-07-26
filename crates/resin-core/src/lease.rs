//! SSE session sticky lease table.
//!
//! While an SSE stream is open for (lane, account), the lane is locked to the
//! anchored exit IP until the stream ends or errors. On failure the lease is
//! evicted and the next request picks a fresh lane/IP. Mirrors Resin's
//! Account/IP lease table but in-process + DashMap-backed.

use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Unique lease id (process-wide monotonic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LeaseId(pub u64);

impl LeaseId {
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// Lease metadata for one in-flight SSE stream on a lane.
#[derive(Debug, Clone)]
pub struct Lease {
    pub id: LeaseId,
    pub lane: usize,
    pub account: String,
    pub exit_ip: String,
    pub authority: String,
    pub created: Instant,
    pub ttl: Duration,
    pub alive: bool,
}

/// TTL-protected, lock-of-the-lane lease table.
pub struct LeaseTable {
    inner: DashMap<LeaseId, Lease>,
    next: AtomicU64,
}

impl LeaseTable {
    pub fn new() -> Self {
        Self { inner: DashMap::new(), next: AtomicU64::new(1) }
    }

 /// Acquire the lease for (lane, account). Returns [`None`] if the lane
 /// already has a live lease for this account (caller should pick another
 /// lane or wait). Records TTL = 90s default.
    pub fn acquire(
        &self,
        lane: usize,
        account: &str,
        exit_ip: &str,
        authority: &str,
        ttl: Duration,
    ) -> Option<LeaseId> {
        let now = Instant::now();
        // Evict expired leases on this lane first.
        self.evict_expired_lane(lane);
        // Block if a live lease already holds this lane (SSE sticky contract).
        for entry in self.inner.iter() {
            let l = entry.value();
            if l.lane == lane && l.alive && now.duration_since(l.created) < l.ttl {
                return None;
            }
        }
        let id = LeaseId(self.next.fetch_add(1, Ordering::SeqCst));
        let lease = Lease {
            id, lane, account: account.to_string(), exit_ip: exit_ip.to_string(),
            authority: authority.to_string(), created: now, ttl, alive: true,
        };
        self.inner.insert(id, lease);
        Some(id)
    }

    /// Renew TTL after a heartbeat / chunk. Returns false if the lease was
    /// already evicted (stream aborted upstream).
    pub fn renew(&self, id: LeaseId, ttl: Duration) -> bool {
        if let Some(mut l) = self.inner.get_mut(&id) {
            l.ttl = ttl;
            l.created = Instant::now();
            l.alive = true;
            true
        } else {
            false
        }
    }

    /// Mark the lease dead (stream ended / errored). Drops the lane lock.
    pub fn release(&self, id: LeaseId) {
        if let Some(mut l) = self.inner.get_mut(&id) {
            l.alive = false;
        }
        self.inner.remove(&id);
    }

    /// Force-evict a lane (failure path). Frees it for the next request.
    pub fn evict_lane(&self, lane: usize) {
        let to_remove: Vec<LeaseId> = self
            .inner
            .iter()
            .filter(|e| e.value().lane == lane)
            .map(|e| e.value().id)
            .collect();
        for id in to_remove {
            self.inner.remove(&id);
        }
    }

    /// Number of currently-held leases.
    pub fn live_count(&self) -> usize {
        self.inner.iter().filter(|e| e.value().alive).count()
    }

    /// Inspect a lease by id (snapshot).
    pub fn get(&self, id: LeaseId) -> Option<Lease> {
        self.inner.get(&id).map(|e| e.value().clone())
    }

    fn evict_expired_lane(&self, lane: usize) {
        let now = Instant::now();
        let to_remove: Vec<LeaseId> = self
            .inner
            .iter()
            .filter(|e| {
                let l = e.value();
                l.lane == lane && now.duration_since(l.created) >= l.ttl
            })
            .map(|e| e.value().id)
            .collect();
        for id in to_remove {
            self.inner.remove(&id);
        }
    }
}

impl Default for LeaseTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    fn ttl() -> Duration {
        Duration::from_millis(100)
    }

    #[test]
    fn acquire_then_block_second() {
        let t = LeaseTable::new();
        let a = t.acquire(3, "acct-1", "1.2.3.4", "api.openai.com", ttl());
        assert!(a.is_some());
        let b = t.acquire(3, "acct-1", "1.2.3.4", "api.openai.com", ttl());
        // Same lane is locked while the first lease is alive.
        assert!(b.is_none(), "second acquire on live lane should be blocked");
    }

    #[test]
    fn different_lane_acquires() {
        let t = LeaseTable::new();
        let a = t.acquire(1, "acct-1", "1.1.1.1", "api.openai.com", ttl());
        let b = t.acquire(2, "acct-2", "2.2.2.2", "api.openai.com", ttl());
        assert!(a.is_some() && b.is_some());
    }

    #[test]
    fn release_reopens_lane() {
        let t = LeaseTable::new();
        let a = t.acquire(7, "acct", "9.9.9.9", "api.x.com", ttl()).unwrap();
        t.release(a);
        let b = t.acquire(7, "acct", "9.9.9.9", "api.x.com", ttl());
        assert!(b.is_some(), "release should reopen the lane");
    }

    #[test]
    fn ttl_expiry_reopens_lane() {
        let t = LeaseTable::new();
        let _a = t.acquire(0, "a", "1.1.1.1", "api.x.com", Duration::from_millis(20));
        sleep(Duration::from_millis(40));
        let b = t.acquire(0, "a", "1.1.1.1", "api.x.com", ttl());
        assert!(b.is_some(), "expired lease should free the lane");
    }

    #[test]
    fn evict_lane_releases_all_live_on_lane() {
        let t = LeaseTable::new();
        let _a = t.acquire(4, "a", "1.1.1.1", "api.x.com", ttl());
        t.evict_lane(4);
        assert_eq!(t.live_count(), 0);
    }

    #[test]
    fn renew_keeps_lease_alive() {
        let t = LeaseTable::new();
        let id = t.acquire(5, "a", "8.8.8.8", "api.x.com", Duration::from_millis(10)).unwrap();
        sleep(Duration::from_millis(15));
        // Without renew the lane would have expired. Renew and try a second acquire: should still be blocked.
        assert!(t.renew(id, ttl()));
        let b = t.acquire(5, "a", "8.8.8.8", "api.x.com", ttl());
        assert!(b.is_none(), "renewed lease should still hold the lane");
    }

    #[test]
    fn get_returns_snapshot() {
        let t = LeaseTable::new();
        let id = t.acquire(2, "a", "1.1.1.1", "api.x.com", ttl()).unwrap();
        let snap = t.get(id).unwrap();
        assert_eq!(snap.lane, 2);
        assert_eq!(snap.account, "a");
        assert_eq!(snap.exit_ip, "1.1.1.1");
        assert!(snap.alive);
    }
}
