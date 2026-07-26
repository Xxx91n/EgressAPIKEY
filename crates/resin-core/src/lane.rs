//! Lane hashing: an AI API key is hashed into one of N lanes.
//!
//! Design contract (from README): N lanes (default 10, max 50). Keys hash into
//! lanes, not a 1:1 port mapping. Same key always lands on the same lane within
//! a session so SSE stickiness and lease accounting are deterministic per key.

use crate::sanitize_lanes;
use fxhash::FxHasher;
use std::hash::{Hash, Hasher};

/// Lane configuration. `lanes` is validated to 1..=50.
#[derive(Debug, Clone)]
pub struct LaneConfig {
    pub lanes: usize,
}

impl LaneConfig {
    pub fn new(lanes: usize) -> Self {
        Self { lanes: sanitize_lanes(lanes) }
    }
}

impl Default for LaneConfig {
    fn default() -> Self {
        Self::new(crate::DEFAULT_LANES)
    }
}

/// Hash an API key into a lane index in [0, lanes).
///
/// Uses FxHash (fast, non-cryptographic). The same key always maps to the
/// same lane for a fixed lane count, giving deterministic lease accounting.
/// Input is treated as bytes (UTF-8). The "Bearer " prefix MUST be stripped
/// by the caller before hashing so the lease key is stable across auth forms.
pub fn lane_index(key: &str, cfg: &LaneConfig) -> usize {
    let mut h = FxHasher::default();
    key.hash(&mut h);
    let v = h.finish();
    (v as usize) % cfg.lanes.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_key_same_lane() {
        let cfg = LaneConfig::default();
        let a = lane_index("sk-test-abc", &cfg);
        let b = lane_index("sk-test-abc", &cfg);
        assert_eq!(a, b);
    }

    #[test]
    fn lane_in_range_default() {
        let cfg = LaneConfig::default();
        for i in 0..10_000 {
            let key = format!("sk-{i}");
            let l = lane_index(&key, &cfg);
            assert!(l < cfg.lanes, "lane {l} >= {}", cfg.lanes);
        }
    }

    #[test]
    fn clamp_to_max_50() {
        let cfg = LaneConfig::new(10_000);
        assert_eq!(cfg.lanes, 50);
        let cfg = LaneConfig::new(0);
        assert_eq!(cfg.lanes, 1);
    }

    #[test]
    fn distribution_is_balanced_enough() {
        // Keys should spread across lanes; no lane gets >50% for 10k keys over 10 lanes.
        let cfg = LaneConfig::default();
        let mut counts = vec![0usize; cfg.lanes];
        for i in 0..10_000 {
            let l = lane_index(&format!("sk-{i}"), &cfg);
            counts[l] += 1;
        }
        let max = *counts.iter().max().unwrap();
        assert!(max < 5_000, "lane {max:?} too hot, distribution skewed");
    }

    #[test]
    fn lane_changes_when_count_changes() {
        let a = lane_index("sk-fix", &LaneConfig::new(10));
        let b = lane_index("sk-fix", &LaneConfig::new(50));
        // Not asserting inequality (could collide) but check both in-range.
        assert!(a < 10 && b < 50);
    }
}
