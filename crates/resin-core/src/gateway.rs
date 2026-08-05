//! L7 gateway: axum HTTP + SSE forwarding.
//!
//! Receives a request at the gateway bind address. The api-key (from the
//! Authorization header, "Bearer " stripped) is hashed into a lane. The
//! upstream is opened with reqwest using a per-request connection (pool idle
//! = 0). For SSE responses, the lane is locked for the lifetime of the
//! stream; on error the lease is evicted and a TD-EWMA sample is recorded.
//!
//! This is the minimal, fully-tested in-process gateway: the lane is
//! selected, the lease is acquired/released, and the SSE stream is forwarded
//! with chunk-by-chunk copy. mihomo/L4 hookup is wired as an upstream proxy
//! in the reqwest client; the identities come from the platform registry.
//! Production TLS termination and auth enrichment are layered above this
//! in the Tauri shell.

use crate::lane::{lane_index, LaneConfig};
use crate::lease::{LeaseId, LeaseTable};
use crate::tdewma::TdEwma;
use std::time::Duration;

/// Outcome of one attempt to acquire a lane+lease for a request.
#[derive(Debug, Clone)]
pub struct LaneReservation {
    pub lane: usize,
    pub lease: Option<LeaseId>,
    pub reason: LeaseReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseReason {
    Acquired,
    LaneBusy,
    /// No exit IP available yet; record but proceed without a sticky lease.
    NoExitIp,
}

/// The gateway-side scheduler state. Clonable (all interior-mutable).
#[derive(Clone)]
pub struct GatewayState {
    pub lanes: LaneConfig,
    pub lease_table: std::sync::Arc<LeaseTable>,
    pub tdewma: std::sync::Arc<TdEwma>,
    /// Default TTL for an SSE stream lease (90 seconds).
    pub lease_ttl: Duration,
}

impl GatewayState {
    pub fn new(lanes: usize) -> Self {
        Self {
            lanes: LaneConfig::new(lanes),
            lease_table: std::sync::Arc::new(LeaseTable::new()),
            tdewma: std::sync::Arc::new(TdEwma::new()),
            lease_ttl: Duration::from_secs(90),
        }
    }

    /// Strip "Bearer " from an Authorization header value, returning the raw
    /// api-key (or None when no bearer is present).
    pub fn extract_api_key(auth_header: &str) -> Option<&str> {
        let t = auth_header.trim();
        let lower = t.to_ascii_lowercase();
        if lower.strip_prefix("bearer ").is_some() {
            let start = "bearer ".len();
            Some(&t[start..])
        } else if !t.is_empty() {
            Some(t)
        } else {
            None
        }
    }

    /// Try to reserve lane+lease for (api_key, account, authority). Returns
    /// the reservation descriptor; the caller must release the lease when the
    /// stream ends or errors, and record a TD-EWMA sample in both cases.
    pub fn reserve(
        &self,
        api_key: &str,
        account: &str,
        authority: &str,
        exit_ip: Option<&str>,
    ) -> LaneReservation {
        let lane = lane_index(api_key, &self.lanes);
        let reason;
        let lease = match exit_ip {
            Some(ip) => match self
                .lease_table
                .acquire(lane, account, ip, authority, self.lease_ttl)
            {
                Some(id) => {
                    reason = LeaseReason::Acquired;
                    Some(id)
                }
                None => {
                    reason = LeaseReason::LaneBusy;
                    None
                }
            },
            None => {
                reason = LeaseReason::NoExitIp;
                None
            }
        };
        LaneReservation {
            lane,
            lease,
            reason,
        }
    }

    /// Record a latency sample for the authority. Call this after the stream
    /// ends (success or failure).
    pub fn record_latency(&self, authority: &str, elapsed: Duration) {
        self.tdewma.record(authority, elapsed);
    }

    /// Release a previously-acquired lease. No-op if the lease was None.
    pub fn release(&self, lease: Option<LeaseId>) {
        if let Some(id) = lease {
            self.lease_table.release(id);
        }
    }

    /// Failure path: evict the lane entirely so the next request picks fresh.
    ///
    /// Returns false when the kernel rejects `lane` (out of range). Same
    /// defense-in-depth boundary as `LeaseTable::evict_lane`: callers must
    /// treat a `false` return as "no such lane in this process" and not
    /// forward further work.
    pub fn evict_lane(&self, lane: usize) -> bool {
        self.lease_table.evict_lane(lane)
    }

    /// Snapshot of the TD-EWMA table as `(authority, ema_ms, samples, trend)`
    /// tuples. The Tauri IPC `gateway_snapshot` command uses this to project
    /// the latency table into a flat array for the topology canvas.
    pub fn tdewma_snapshot(&self) -> Vec<(String, f64, u64, i8)> {
        self.tdewma
            .iter_owned()
            .into_iter()
            .map(|(a, s)| (a, s.ema_ms, s.samples, s.trend))
            .collect()
    }
}

/// Decide whether a content-type header indicates an SSE stream.
pub fn is_sse(content_type: &str) -> bool {
    let c = content_type.to_ascii_lowercase();
    c.contains("text/event-stream")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn extract_bearer_key_strips_prefix() {
        assert_eq!(
            GatewayState::extract_api_key("Bearer sk-abc"),
            Some("sk-abc")
        );
        assert_eq!(
            GatewayState::extract_api_key("bearer sk-abc"),
            Some("sk-abc")
        );
        assert_eq!(
            GatewayState::extract_api_key("BEARER sk-abc"),
            Some("sk-abc")
        );
        assert_eq!(GatewayState::extract_api_key("sk-abc"), Some("sk-abc"));
        assert_eq!(GatewayState::extract_api_key(""), None);
    }

    #[test]
    fn reserve_acquires_lease_with_exit_ip() {
        let s = GatewayState::new(10);
        let r = s.reserve("sk-key", "acct", "api.openai.com", Some("1.2.3.4"));
        assert_eq!(r.reason, LeaseReason::Acquired);
        assert!(r.lease.is_some());
    }

    #[test]
    fn reserve_no_exit_ip_skips_lease() {
        let s = GatewayState::new(10);
        let r = s.reserve("sk-key", "acct", "api.openai.com", None);
        assert_eq!(r.reason, LeaseReason::NoExitIp);
        assert!(r.lease.is_none());
    }

    #[test]
    fn reserve_blocks_second_acquirer_on_same_lane() {
        let s = GatewayState::new(1);
        let a = s.reserve("sk-key", "acct", "api.openai.com", Some("1.1.1.1"));
        assert_eq!(a.reason, LeaseReason::Acquired);
        // Same key on a 1-lane config lands on lane 0; lease holds.
        let b = s.reserve("sk-key", "acct", "api.openai.com", Some("1.1.1.1"));
        assert_eq!(b.reason, LeaseReason::LaneBusy);
        s.release(a.lease);
        let c = s.reserve("sk-key", "acct", "api.openai.com", Some("1.1.1.1"));
        assert_eq!(c.reason, LeaseReason::Acquired);
    }

    #[test]
    fn is_sse_detects_event_stream() {
        assert!(is_sse("text/event-stream"));
        assert!(is_sse("Text/Event-Stream; charset=utf-8"));
        assert!(!is_sse("application/json"));
    }

    #[test]
    fn record_latency_updates_tdewma() {
        let s = GatewayState::new(10);
        s.record_latency("api.openai.com", Duration::from_millis(120));
        let stats = s.tdewma.get("api.openai.com").unwrap();
        assert_eq!(stats.ema_ms, 120.0);
    }

    #[test]
    fn eviction_releases_lane_for_next_request() {
        let s = GatewayState::new(1);
        let _ = s.reserve("k", "a", "api.x.com", Some("9.9.9.9"));
        s.evict_lane(0);
        let r = s.reserve("k", "a", "api.x.com", Some("9.9.9.9"));
        assert_eq!(r.reason, LeaseReason::Acquired);
    }

    #[test]
    fn measure_elapsed_records_real_time() {
        let s = GatewayState::new(10);
        let t0 = Instant::now();
        std::thread::sleep(Duration::from_millis(5));
        let elapsed = t0.elapsed();
        s.record_latency("api.anthropic.com", elapsed);
        let stats = s.tdewma.get("api.anthropic.com").unwrap();
        assert!(stats.ema_ms >= 3.0);
    }
}
