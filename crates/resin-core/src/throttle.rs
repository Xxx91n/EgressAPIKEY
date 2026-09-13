//! single-owner throttle model.
//!
//! The shell-side "how long until the next fire?" arithmetic previously
//! lived inline in two modules: `port_health.rs` (adaptive poll interval +
//! exponential failure backoff) and the shell crate `lightweight.rs`
//! (one-shot close delay). Both now route through this module, which owns
//! the arithmetic exactly once; each call site keeps only its own parameter
//! values (a `ThrottleParams` const). No new architecture layer - a leaf
//! module next to `port_health`, same as `stream_sensor`.

use std::time::Duration;

/// Rhythm parameters owned by each call site; the arithmetic below is
/// owned by this module. `floor_secs` is the hard minimum for any computed
/// delay; `cap_secs` (when present) is the hard maximum.
///
/// Invariant: `floor_secs <= cap_secs` when a cap is present (both live
/// instances satisfy it; the clamp is a no-op ordering-wise either way).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThrottleParams {
    pub floor_secs: u64,
    pub cap_secs: Option<u64>,
}

impl ThrottleParams {
    /// Floor-only model (one-shot delays: no upper bound - the caller's own
    /// validation caps the input, e.g. lightweight_set's 1..=1440 minutes).
    pub const fn new(floor_secs: u64) -> Self {
        Self { floor_secs, cap_secs: None }
    }

    /// Floor + cap model (polling rhythms: failure backoff must not run away).
    pub const fn bounded(floor_secs: u64, cap_secs: u64) -> Self {
        Self { floor_secs, cap_secs: Some(cap_secs) }
    }

    /// The single clamp point: cap first, then floor - byte-for-byte the
    /// order the legacy port-health backoff used, so rhythms are identical.
    pub fn clamp_delay(&self, delay: Duration) -> Duration {
        let secs = delay.as_secs();
        let capped = match self.cap_secs {
            Some(cap) => secs.min(cap),
            None => secs,
        };
        Duration::from_secs(capped.max(self.floor_secs))
    }
}

/// Adaptive poll interval: max(floor, ceil(k*ln(1+N))) seconds for N ports
/// (Cilium CFP-32820 pattern; the poll call site passes k=2). N=0
/// degenerates to the floor, same as the legacy formula.
pub fn adaptive_interval(params: ThrottleParams, k: f64, port_count: usize) -> Duration {
    let secs = if port_count == 0 {
        0
    } else {
        (k * (1.0 + port_count as f64).ln()).ceil() as u64
    };
    params.clamp_delay(Duration::from_secs(secs))
}

/// Exponential failure backoff: base * 2^min(fails, exponent_cap), clamped
/// by `params` (the poll call site passes exponent_cap=5, cap=300s).
/// Saturating arithmetic throughout - hostile inputs cannot overflow-panic.
pub fn backoff_interval(params: ThrottleParams, base: Duration, fails: u32, exponent_cap: u32) -> Duration {
    let exp = 2u64.saturating_pow(fails.min(exponent_cap));
    params.clamp_delay(Duration::from_secs(base.as_secs().saturating_mul(exp)))
}

/// One-shot delay for `minutes` minutes, clamped by `params` (the
/// lightweight-mode call site: delay = minutes*60s, floor = 1 minute).
pub fn delay_minutes(params: ThrottleParams, minutes: u32) -> Duration {
    params.clamp_delay(Duration::from_secs(minutes as u64 * 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLL: ThrottleParams = ThrottleParams::bounded(5, 300);
    const DELAY: ThrottleParams = ThrottleParams::new(60);

    // --- clamp_delay: the single ownership point ---

    #[test]
    fn clamp_applies_cap_then_floor_in_legacy_order() {
        // Legacy inline order was min(cap) then max(floor); the shared clamp
        // reproduces it exactly (floor <= cap for the poll params).
        assert_eq!(POLL.clamp_delay(Duration::from_secs(1)), Duration::from_secs(5));
        assert_eq!(POLL.clamp_delay(Duration::from_secs(10)), Duration::from_secs(10));
        assert_eq!(POLL.clamp_delay(Duration::from_secs(301)), Duration::from_secs(300));
    }

    #[test]
    fn clamp_without_cap_has_no_upper_bound() {
        assert_eq!(DELAY.clamp_delay(Duration::from_secs(86_400)), Duration::from_secs(86_400));
        assert_eq!(DELAY.clamp_delay(Duration::from_secs(1)), Duration::from_secs(60));
    }

    // --- adaptive_interval: port-health rhythm, exact legacy values ---

    #[test]
    fn adaptive_matches_legacy_formula() {
        // Recomputed inline from the pre- formula for a spread of N.
        for n in [0usize, 1, 2, 5, 10, 50, 100, 1000, 100_000] {
            let legacy_secs = if n == 0 {
                0
            } else {
                (2.0 * (1.0 + n as f64).ln()).ceil() as u64
            };
            let expected = Duration::from_secs(legacy_secs.max(5));
            assert_eq!(adaptive_interval(POLL, 2.0, n), expected, "n={n}");
        }
    }

    #[test]
    fn adaptive_floor_and_growth_boundaries() {
        assert_eq!(adaptive_interval(POLL, 2.0, 0), Duration::from_secs(5));
        assert_eq!(adaptive_interval(POLL, 2.0, 1), Duration::from_secs(5)); // 2*ln2 ~ 1.39 -> 2 < 5
        assert_eq!(adaptive_interval(POLL, 2.0, 10), Duration::from_secs(5)); // 2*ln11 ~ 4.8 -> 5 == floor
        assert_eq!(adaptive_interval(POLL, 2.0, 100), Duration::from_secs(10)); // 2*ln101 ~ 9.23 -> 10
        assert_eq!(adaptive_interval(POLL, 2.0, 1000), Duration::from_secs(14)); // 2*ln1001 ~ 13.8 -> 14
    }

    // --- backoff_interval: port-health rhythm, exact legacy values ---

    #[test]
    fn backoff_doubles_with_consecutive_fails_until_exponent_cap() {
        let base = Duration::from_secs(5);
        assert_eq!(backoff_interval(POLL, base, 1, 5), Duration::from_secs(10));
        assert_eq!(backoff_interval(POLL, base, 2, 5), Duration::from_secs(20));
        assert_eq!(backoff_interval(POLL, base, 3, 5), Duration::from_secs(40));
        assert_eq!(backoff_interval(POLL, base, 4, 5), Duration::from_secs(80));
        assert_eq!(backoff_interval(POLL, base, 5, 5), Duration::from_secs(160));
        // Exponent cap pins further growth: 2^min(6,5) == 2^5.
        assert_eq!(backoff_interval(POLL, base, 6, 5), Duration::from_secs(160));
        assert_eq!(backoff_interval(POLL, base, 99, 5), Duration::from_secs(160));
    }

    #[test]
    fn backoff_caps_at_model_cap_and_floors_below_model_floor() {
        // Cap binds when the base is large: 200s * 2^2 = 800s -> capped 300s.
        assert_eq!(backoff_interval(POLL, Duration::from_secs(200), 2, 5), Duration::from_secs(300));
        // Floor binds when the product is small: 2s * 2^1 = 4s -> floored 5s.
        assert_eq!(backoff_interval(POLL, Duration::from_secs(2), 1, 5), Duration::from_secs(5));
    }

    #[test]
    fn backoff_saturates_without_overflow_panic() {
        let out = backoff_interval(POLL, Duration::from_secs(u64::MAX), 99, 5);
        assert_eq!(out, Duration::from_secs(300));
    }

    // --- delay_minutes: lightweight rhythm, exact legacy values ---

    #[test]
    fn delay_minutes_matches_legacy_for_live_inputs() {
        // The legacy inline computation was Duration::from_secs(m*60) with
        // m >= 1 guaranteed upstream; identical for every reachable value.
        assert_eq!(delay_minutes(DELAY, 1), Duration::from_secs(60));
        assert_eq!(delay_minutes(DELAY, 10), Duration::from_secs(600));
        assert_eq!(delay_minutes(DELAY, 1440), Duration::from_secs(86_400));
        assert_eq!(delay_minutes(DELAY, u32::MAX), Duration::from_secs(u32::MAX as u64 * 60));
    }

    #[test]
    fn delay_minutes_floors_zero_at_one_minute() {
        // The explicit floor: 0 is unreachable through live paths, and the
        // shared model keeps the documented one-minute minimum instead of a
        // 0s fire.
        assert_eq!(delay_minutes(DELAY, 0), Duration::from_secs(60));
    }
}
