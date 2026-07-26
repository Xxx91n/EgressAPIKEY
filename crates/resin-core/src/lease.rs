//! SSE session sticky lease table.
//!
//! While an SSE stream is open for (lane, account), the lane is locked to the
//! anchored exit IP until the stream ends or errors. On failure the lease is
//! evicted and the next request picks a fresh lane/IP. Mirrors Resin's
//! Account/IP lease table but in-process + DashMap-backed.

//! Concurrency: lane occupancy is guarded by a per-lane parking_lot::Mutex
//! slot so that the check-then-insert critical section is atomic. Two
//! concurrent acquirers of the same lane cannot both win: the first takes the
//! lock, observes an empty slot, inserts and stores its LeaseId; the second
//! takes the same lock next, observes a live lease, and returns None. This
//! preserves the SSE stickiness contract under contention.

use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use crate::MAX_LANES;

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
    /// slot_idx -> guard of occupancy for logical lane = slot_idx.
    /// Triple is (id, created, ttl) of the current occupant. None => free.
    /// Fixed size MAX_LANES; logical lane < lanes <= MAX_LANES so distinct
    /// logical lanes never share a slot for valid lane counts.
    lane_slots: Vec<std::sync::Arc<Mutex<Option<(LeaseId, Instant, Duration)>>>>,
    next: AtomicU64,
}

impl LeaseTable {
    pub fn new() -> Self {
        let lane_slots = (0..MAX_LANES)
            .map(|_| std::sync::Arc::new(Mutex::new(None)))
            .collect();
        Self {
            inner: DashMap::new(),
            lane_slots,
            next: AtomicU64::new(1),
        }
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
        let slot = &self.lane_slots[lane];
        let mut guard = slot.lock();
        // Check-then-insert is atomic under the per-lane lock.
        // A TTL-expired occupant is evicted lazily: we overwrite the slot
        // below with the new id, making the next release/renew on that id a
        // no-op (they observe slot_id != their id and skip).
        if let Some((_id, created, prev_ttl)) = *guard {
            if now.duration_since(created) < prev_ttl {
                return None;
            }
        }
        let id = LeaseId(self.next.fetch_add(1, Ordering::SeqCst));
        let lease = Lease {
            id, lane, account: account.to_string(), exit_ip: exit_ip.to_string(),
            authority: authority.to_string(), created: now, ttl, alive: true,
        };
        self.inner.insert(id, lease);
        *guard = Some((id, now, ttl));
        Some(id)
    }

    /// Renew TTL after a heartbeat / chunk. Returns false if the lease was
    /// already evicted (stream aborted upstream).
    pub fn renew(&self, id: LeaseId, ttl: Duration) -> bool {
        let now = Instant::now();
        let lane = match self.inner.get_mut(&id) {
            Some(mut l) => {
                l.ttl = ttl;
                l.created = now;
                l.alive = true;
                l.lane
            }
            None => return false,
        };
        // Refresh the slot so the lane stays sticky under the new TTL.
        let slot = &self.lane_slots[lane];
        let mut guard = slot.lock();
        if let Some((slot_id, _created, _prev_ttl)) = *guard {
            if slot_id == id {
                *guard = Some((id, now, ttl));
            }
        }
        true
    }

    /// Mark the lease dead (stream ended / errored). Drops the lane lock.
    pub fn release(&self, id: LeaseId) {
        let lane_opt = self.inner.get(&id).map(|e| e.value().lane);
        self.inner.remove(&id);
        if let Some(lane) = lane_opt {
            let slot = &self.lane_slots[lane];
            let mut guard = slot.lock();
            if let Some((slot_id, _created, _ttl)) = *guard {
                if slot_id == id {
                    *guard = None;
                }
            }
        }
    }

    /// Force-evict a lane (failure path). Frees it for the next request.
    pub fn evict_lane(&self, lane: usize) {
        // Clear the slot first so a concurrent acquire cannot re-admit a
        // lease for the same lane while we are evicting it.
        {
            let slot = &self.lane_slots[lane];
            let mut guard = slot.lock();
            *guard = None;
        }
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

    /// Concurrency: two threads racing to acquire the same lane must yield
    /// exactly one winner. Guards against the original check-then-insert race
    /// that allowed two live leases on one lane.
    #[test]
    fn concurrent_acquire_serialises_on_lane() {
        let t = std::sync::Arc::new(LeaseTable::new());
        let lane = 6;
        const N: usize = 32;
        let mut handles = Vec::new();
        let winners = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for _ in 0..N {
            let t = t.clone();
            let winners = winners.clone();
            handles.push(std::thread::spawn(move || {
                let r = t.acquire(lane, "acct", "1.2.3.4", "api.x.com", Duration::from_secs(60));
                if r.is_some() {
                    winners.fetch_add(1, Ordering::SeqCst);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(
            winners.load(Ordering::SeqCst),
            1,
            "exactly one concurrent acquire should win the lane"
        );
    }

    /// Concurrency: concurrent release + acquire must not deadlock or corrupt
    /// the slot. After release, an acquire on the same lane always wins.
    #[test]
    fn concurrent_release_then_acquire() {
        let t = std::sync::Arc::new(LeaseTable::new());
        let lane = 9;
        let id = t.acquire(lane, "a", "1.1.1.1", "api.x.com", Duration::from_secs(60)).unwrap();
        let t2 = t.clone();
        let h = std::thread::spawn(move || {
            // Spin until the lane opens, then take it.
            for _ in 0..200 {
                if let Some(id2) = t2.acquire(lane, "a", "1.1.1.1", "api.x.com", Duration::from_secs(60)) {
                    t2.release(id2);
                    return true;
                }
                sleep(Duration::from_millis(2));
            }
            false
        });
        // Give the thread time to start spinning, then release.
        sleep(Duration::from_millis(10));
        t.release(id);
        let ok = h.join().unwrap();
        assert!(ok, "post-release acquire must succeed");
    }
}
