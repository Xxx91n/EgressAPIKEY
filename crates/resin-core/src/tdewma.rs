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
        Self { inner: DashMap::new(), alpha: alpha.clamp(0.01, 1.0) }
    }

 /// Record a latency sample for `authority` (e.g. "api.openai.com").
 /// Returns the updated stats.
    pub fn record(&self, authority: &str, latency: Duration) -> LatencyStats {
        let ms = latency.as_secs_f64() * 1000.0;
        let mut entry = self.inner.entry(authority.to_string()).or_insert(LatencyStats {
            ema_ms: ms,
            samples: 0,
            trend: 0,
        });
        let s = entry.value_mut();
        if s.samples == 0 {
            // First sample seeds the EMA directly.
            s.ema_ms = ms;
            s.trend = 0;
        } else {
            let prev = s.ema_ms;
            s.ema_ms = (1.0 - self.alpha) * prev + self.alpha * ms;
            s.trend = if (ms - s.ema_ms).abs() < 1e-9 {
                0
            } else if ms > s.ema_ms {
                1
            } else {
                -1
            };
        }
        s.samples += 1;
        *s
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
        assert!(s.ema_ms > 100.0 && s.ema_ms < 200.0, "ema={} should be between", s.ema_ms);
    }

    #[test]
    fn trend_marks_degradation_and_improvement() {
        let t = TdEwma::new();
        let _ = t.record("y", Duration::from_millis(100));
        let _ = t.record("y", Duration::from_millis(100));
        let s_up = t.record("y", Duration::from_millis(500));
        assert_eq!(s_up.trend, 1, "huge sample above ema should mark degradation");
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
    fn alpha_is_clamped() {
        let _ = TdEwma::with_alpha(99.0);
        let _ = TdEwma::with_alpha(-1.0);
        // Construction must not panic.
    }
}
