//! TD-EWMA-lite: per-authority latency exponential moving average.
//!
//! Resin's full TD-EWMA separates authoritative DNS names from ordinary sites
//! with an LRU and a "trend decay" weight on top of the EWMA. This is a
//! lightweight in-process variant: a single EMA per authority plus a trend
//! sign, enough for the scheduler to prefer fresh lanes and detect degradation.

use dashmap::DashMap;
use std::time::Duration;

/// Default EMA alpha (weight on the latest sample). Lower = smoother.
pub const DEFAULT_ALPHA: f64 = 0.3;

/// One authority's latency stats.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct LatencyStats {
    pub ema_ms: f64,
    pub samples: u64,
    /// +1 if last sample > ema (degrading), -1 if < ema (improving), 0 neutral.
    pub trend: i8,
}

/// Maximum number of tracked authorities. Caps unbounded growth from a
/// hostile or buggy caller that floods the IPC `gateway_record_latency`
/// with a new authority string every request. Existing authorities get
/// refreshed under their existing entry; brand-new authorities past this cap
/// are reported as a transient stats snapshot without persisting an entry, so
/// `len()` and `iter_owned()` remain bounded.
pub const MAX_AUTHORITIES: usize = 256;

/// Per-authority latency tracker.
pub struct TdEwma {
    inner: DashMap<String, LatencyStats>,
    alpha: f64,
}

impl TdEwma {
    pub fn new() -> Self {
        Self::with_alpha(DEFAULT_ALPHA)
    }

    pub fn with_alpha(alpha: f64) -> Self {
        Self {
            inner: DashMap::new(),
            alpha: alpha.clamp(0.01, 1.0),
        }
    }

    /// Cap state: when the table already holds `MAX_AUTHORITIES` entries,
    /// a brand-new authority is served a throw-away snapshot (synthesised from
    /// this single sample) instead of being inserted. Known authorities are
    /// always updated. This is the in-process analog of an LRU but without
    /// the bookkeeping cost, good enough for the capped threat model.
    fn is_full(&self) -> bool {
        self.inner.len() >= MAX_AUTHORITIES
    }

    /// Record a latency sample for `authority` (e.g. "api.openai.com").
    /// Returns the updated stats. When the authority table is already at
    /// `MAX_AUTHORITIES` and `authority` is new, the insert is rejected and a
    /// one-shot snapshot is returned; the table stays bounded and the caller
    /// still gets a non-empty stats record.
    pub fn record(&self, authority: &str, latency: Duration) -> LatencyStats {
        let ms = latency.as_secs_f64() * 1000.0;
        // Fast path: existing entry short-circuits the cap check.
        if let Some(mut existing) = self.inner.get_mut(authority) {
            let st = existing.value_mut();
            Self::update(st, self.alpha, ms);
            return *st;
        }
        if self.is_full() {
            return LatencyStats {
                ema_ms: ms,
                samples: 1,
                trend: 0,
            };
        }
        let mut entry = self
            .inner
            .entry(authority.to_string())
            .or_insert(LatencyStats {
                ema_ms: ms,
                samples: 0,
                trend: 0,
            });
        let s = entry.value_mut();
        Self::update(s, self.alpha, ms);
        *s
    }

    /// Shared EMA/trend update for both branch arms.
    fn update(st: &mut LatencyStats, alpha: f64, ms: f64) {
        if st.samples == 0 {
            st.ema_ms = ms;
            st.trend = 0;
        } else {
            let prev = st.ema_ms;
            st.ema_ms = (1.0 - alpha) * prev + alpha * ms;
            st.trend = if (ms - st.ema_ms).abs() < 1e-9 {
                0
            } else if ms > st.ema_ms {
                1
            } else {
                -1
            };
        }
        st.samples += 1;
    }

    /// Snapshot the stats for an authority (None if unseen).
    pub fn get(&self, authority: &str) -> Option<LatencyStats> {
        self.inner.get(authority).map(|e| *e.value())
    }

    /// Forget an authority (used when a lane/IP is evicted).
    pub fn forget(&self, authority: &str) {
        self.inner.remove(authority);
    }

    /// Number of tracked authorities.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Iterate over (authority, stats) as owned tuples, snapshotting the
    /// current TD-EWMA table. Used by the Tauri IPC `gateway_snapshot`
    /// command to project the table into a flat array for the frontend.
    pub fn iter_owned(&self) -> Vec<(String, LatencyStats)> {
        self.inner
            .iter()
            .map(|e| (e.key().clone(), *e.value()))
            .collect()
    }
}

impl Default for TdEwma {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_seeds_ema() {
        let t = TdEwma::new();
        let s = t.record("api.openai.com", Duration::from_millis(100));
        assert_eq!(s.ema_ms, 100.0);
        assert_eq!(s.samples, 1);
        assert_eq!(s.trend, 0);
    }

    #[test]
    fn ema_smooths_on_repeated_samples() {
        let t = TdEwma::new();
        let _ = t.record("x", Duration::from_millis(100));
        let _ = t.record("x", Duration::from_millis(200));
        let s = t.record("x", Duration::from_millis(200));
        assert!(
            s.ema_ms > 100.0 && s.ema_ms < 200.0,
            "ema={} should be between",
            s.ema_ms
        );
    }

    #[test]
    fn trend_marks_degradation_and_improvement() {
        let t = TdEwma::new();
        let _ = t.record("y", Duration::from_millis(100));
        let _ = t.record("y", Duration::from_millis(100));
        let s_up = t.record("y", Duration::from_millis(500));
        assert_eq!(
            s_up.trend, 1,
            "huge sample above ema should mark degradation"
        );
        let s_down = t.record("y", Duration::from_millis(10));
        assert_eq!(s_down.trend, -1, "sample below ema should mark improvement");
    }

    #[test]
    fn forget_clears_authority() {
        let t = TdEwma::new();
        let _ = t.record("z", Duration::from_millis(50));
        assert_eq!(t.len(), 1);
        t.forget("z");
        assert!(t.get("z").is_none());
        assert!(t.is_empty());
    }

    #[test]
    fn cap_rejects_new_authority_past_max() {
        let t = TdEwma::new();
        for i in 0..MAX_AUTHORITIES {
            let _ = t.record(&format!("host{i}"), Duration::from_millis(10));
        }
        assert_eq!(t.len(), MAX_AUTHORITIES);
        let s = t.record("newhost", Duration::from_millis(42));
        assert_eq!(s.ema_ms, 42.0);
        assert_eq!(s.samples, 1);
        assert_eq!(
            t.len(),
            MAX_AUTHORITIES,
            "capacity must not grow on new-authority overflow"
        );
        assert!(t.get("newhost").is_none());
    }

    #[test]
    fn cap_does_not_lock_existing_authority() {
        let t = TdEwma::new();
        let _ = t.record("known", Duration::from_millis(100));
        for i in 0..MAX_AUTHORITIES {
            let _ = t.record(&format!("h{i}"), Duration::from_millis(1));
        }
        let s = t.record("known", Duration::from_millis(500));
        assert!(
            s.samples >= 2,
            "known authority must keep accumulating samples at cap"
        );
        assert_eq!(s.trend, 1, "500ms above prior ema marks degradation");
    }

    #[test]
    fn alpha_is_clamped() {
        let _ = TdEwma::with_alpha(99.0);
        let _ = TdEwma::with_alpha(-1.0);
        // Construction must not panic.
    }
}
