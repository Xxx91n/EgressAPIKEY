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
