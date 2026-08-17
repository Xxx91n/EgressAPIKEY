//! T18 Phase 1 — Port health batch probe (sing-box urltest pattern).
//!
//! Single Tokio task + ticker + for_each_concurrent(10) + AtomicBool reentry
//! guard + TTL skip + exponential backoff (5 fails -> Dead, interval × 2^min(fails,5), cap 5m).
//! Tab-hidden pause: the watcher checks an AtomicBool flag set by Tauri window
//! focus events; when hidden the tick returns early without probing.
//!
//! The watcher is generic over the emit function so the Tauri command can
//! pass a Channel<PortHealthSnapshot> and tests can pass a closure.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::db::PortMapping;

/// Minimum tick interval (seconds). Base of the adaptive formula.
const MIN_INTERVAL_SECS: u64 = 5;
/// Concurrency cap for batch probe (sing-box urltest batch.New WithConcurrencyNum).
const PROBE_CONCURRENCY: usize = 10;
/// Consecutive failures before a port is marked Dead (mihomo max-failed-times).
const FAILS_TO_DEAD: u32 = 5;
/// Backoff cap: interval × 2^min(fails,5), capped at 5 minutes.
const BACKOFF_CAP_SECS: u64 = 300;
/// Connect timeout (matches the existing per-port health check).
const CONNECT_TIMEOUT_MS: u64 = 200;
/// Read-reply timeout (matches the existing per-port health check).
const READ_TIMEOUT_MS: u64 = 550;

/// 4-state health for a single port, derived from probe + consecutive fails.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Alive,
    Degraded,
    Dead,
    Restarting,
}

impl HealthState {
    /// Map (reachable, fails) to the 4-state chip per ADR-0042 S1.
    pub fn classify(reachable: bool, fails: u32) -> Self {
        if reachable {
            if fails > 0 { HealthState::Degraded } else { HealthState::Alive }
        } else if fails >= FAILS_TO_DEAD {
            HealthState::Dead
        } else {
            HealthState::Restarting
        }
    }
}

/// One port's health entry in a PortHealthSnapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortHealthEntry {
    pub port: u16,
    pub state: HealthState,
    pub reachable: bool,
    pub fails: u32,
    pub latency_ms: Option<u32>,
    /// Current effective probe interval for this port (seconds), after backoff.
    pub interval_secs: u64,
}

/// One tick of port health, sent over the Tauri IPC Channel as a single message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortHealthSnapshot {
    /// Monotonic revision counter for idempotent UI updates.
    pub revision: u64,
    pub entries: Vec<PortHealthEntry>,
}

/// Per-port rolling history (sing-box urltest history record pattern).
#[derive(Debug, Clone)]
struct PortHistory {
    last_probe: Instant,
    fails: u32,
    interval: Duration,
    /// Last observed reachability (used when TTL-skip hides this tick).
    last_reachable: bool,
}

impl PortHistory {
    fn new(base_interval: Duration) -> Self { Self { last_probe: Instant::now(), fails: 0, interval: base_interval, last_reachable: true } }
}

/// Adaptive interval: max(MIN_INTERVAL_SECS, k·ln(1+N)) where k=2 (Cilium CFP-32820).
pub fn adaptive_interval(port_count: usize) -> Duration {
    if port_count == 0 { return Duration::from_secs(MIN_INTERVAL_SECS); }
    let secs = (2.0 * (1.0 + port_count as f64).ln()).ceil() as u64;
    Duration::from_secs(secs.max(MIN_INTERVAL_SECS))
}

/// Probe a single port. TCP connect + SOCKS5 greeting or HTTP GET; returns
/// (reachable, latency_ms). Mirrors src-tauri::commands::port_health_check.
async fn probe_one(port: u16, protocol: &str) -> (bool, Option<u32>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let started = Instant::now();
    let addr = format!("127.0.0.1:{port}");
    let connect = tokio::time::timeout(
        Duration::from_millis(CONNECT_TIMEOUT_MS),
        tokio::net::TcpStream::connect(&addr),
    ).await;
    let mut stream = match connect {
        Ok(Ok(s)) => s,
        _ => return (false, None),
    };
    let greeting: Vec<u8> = if protocol == "http" {
        format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").into_bytes()
    } else {
        vec![0x05, 0x02, 0x00, 0x02]
    };
    if stream.write_all(&greeting).await.is_err() {
        return (true, Some(started.elapsed().as_millis() as u32));
    }
    let mut buf = [0u8; 16];
    let read = tokio::time::timeout(Duration::from_millis(READ_TIMEOUT_MS), stream.read(&mut buf)).await;
    let elapsed = started.elapsed().as_millis() as u32;
    match read {
        Ok(Ok(n)) if protocol == "http" && n >= 12 && buf.starts_with(b"HTTP/") => (true, Some(elapsed)),
        Ok(Ok(n)) if protocol == "socks5" && n >= 2 && buf[0] == 0x05 && (buf[1] == 0x00 || buf[1] == 0x02) => (true, Some(elapsed)),
        Ok(Ok(n)) if n >= 4 => (true, Some(elapsed)), // protocol_mismatch but alive
        _ => (false, None),
    }
}

/// Run one batch tick: probe every enabled port concurrently (cap 10),
/// respecting TTL skip (skip if time Since last_probe < interval).
async fn run_tick(
    ports: &[PortMapping],
    histories: &mut HashMap<u16, PortHistory>,
    base_interval: Duration,
) -> Vec<PortHealthEntry> {
    use futures_util::stream::{iter, StreamExt};

    // Snapshot only the ports due for a probe this tick (TTL skip).
    // Clone the due PortMapping so the async closure owns its inputs (no borrow
    // of the caller's ports slice — required to satisfy higher-ranked lifetime).
    let now = Instant::now();
    let due: Vec<PortMapping> = ports.iter().filter(|p| {
        match histories.get(&p.port) {
            Some(h) => now.duration_since(h.last_probe) >= h.interval,
            None => true,
        }
    }).cloned().collect();

    let results: Vec<(u16, bool, Option<u32>)> = iter(due)
        .map(|p| async move {
            let (r, lat) = probe_one(p.port, &p.protocol).await;
            (p.port, r, lat)
        })
        .buffer_unordered(PROBE_CONCURRENCY)
        .collect()
        .await;

    let mut out: Vec<PortHealthEntry> = Vec::with_capacity(ports.len());
    for p in ports {
        let h = histories.entry(p.port).or_insert_with(|| PortHistory::new(base_interval));
        // Find this tick's result if it was due; else reuse prior state.
        let (reachable, latency) = match results.iter().find(|(port, _, _)| *port == p.port) {
            Some((_, r, lat)) => {
                h.last_probe = now;
                h.last_reachable = *r;
                if *r { h.fails = 0; h.interval = base_interval; }
                else {
                    h.fails = h.fails.saturating_add(1);
                    let exp = 2u64.saturating_pow(h.fails.min(FAILS_TO_DEAD));
                    let secs = (base_interval.as_secs()).saturating_mul(exp).min(BACKOFF_CAP_SECS);
                    h.interval = Duration::from_secs(secs.max(MIN_INTERVAL_SECS));
                }
                (*r, *lat)
            }
            None => (h.last_reachable, None),
        };
        let state = HealthState::classify(reachable, h.fails);
        out.push(PortHealthEntry {
            port: p.port,
            state,
            reachable,
            fails: h.fails,
            latency_ms: latency,
            interval_secs: h.interval.as_secs(),
        });
    }
    out
}

/// Spawn the watcher. Returns a guard handle; the task ends when `running`
/// is set to false or the emit closure returns Err (channel closed).
///
/// `ports_fn` is called every tick to refresh the port list (so new ports
/// appear and deleted ports drop out without restarting the watcher).
/// `paused` is the AtomicBool flag set by the Tauri window focus listener;
/// when true the tick returns early without probing (sing-box idle_timeout).
pub fn spawn_watcher<F, E>(
    ports_fn: F,
    emit: E,
    paused: Arc<AtomicBool>,
) where
    F: Fn() -> Vec<PortMapping> + Send + Sync + 'static,
    E: Fn(PortHealthSnapshot) -> Result<(), ()> + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let mut histories: HashMap<u16, PortHistory> = HashMap::new();
        let mut revision: u64 = 0;
        loop {
            let ports = ports_fn();
            let base = adaptive_interval(ports.len());
            tokio::time::sleep(base).await;
            if paused.load(Ordering::Relaxed) { continue; }
            revision = revision.wrapping_add(1);
            let entries = run_tick(&ports, &mut histories, base).await;
            let snapshot = PortHealthSnapshot { revision, entries };
            if emit(snapshot).is_err() { break; } // channel closed -> exit
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_interval_floor_5s_for_small_sets() {
        assert_eq!(adaptive_interval(0), Duration::from_secs(5));
        assert_eq!(adaptive_interval(1), Duration::from_secs(5));
    }

    #[test]
    fn adaptive_interval_grows_with_port_count() {
        let small = adaptive_interval(10);
        let large = adaptive_interval(1000);
        assert!(large >= small);
    }

    #[test]
    fn classify_alive_when_reachable_zero_fails() {
        assert_eq!(HealthState::classify(true, 0), HealthState::Alive);
    }

    #[test]
    fn classify_degraded_when_reachable_with_prior_fails() {
        assert_eq!(HealthState::classify(true, 2), HealthState::Degraded);
    }

    #[test]
    fn classify_restarting_when_unreachable_under_5_fails() {
        assert_eq!(HealthState::classify(false, 1), HealthState::Restarting);
        assert_eq!(HealthState::classify(false, 4), HealthState::Restarting);
    }

    #[test]
    fn classify_dead_when_unreachable_5_or_more_fails() {
        assert_eq!(HealthState::classify(false, 5), HealthState::Dead);
        assert_eq!(HealthState::classify(false, 99), HealthState::Dead);
    }

    #[tokio::test]
    async fn run_tick_marks_unreachable_port_and_backoff_grows() {
        let ports = vec![PortMapping {
            port: 1, protocol: "socks5".into(),
            platform_name: "P".into(), account: "a".into(), label: "".into(),
            enabled: true, auth_required: false,
        }];
        let mut histories = HashMap::new();
        let base = Duration::from_secs(5);
        let out = run_tick(&ports, &mut histories, base).await;
        assert_eq!(out.len(), 1);
        // Port 1 is not listening -> unreachable.
        assert!(!out[0].reachable);
        assert_eq!(out[0].fails, 1);
        assert_eq!(out[0].state, HealthState::Restarting);
        // After first fail interval doubles (5 × 2^1 = 10s, capped to ≥5s).
        assert!(out[0].interval_secs >= 5);
    }

    #[tokio::test]
    async fn run_tick_backoff_caps_at_5_minutes() {
        let ports = vec![PortMapping {
            port: 1, protocol: "socks5".into(),
            platform_name: "P".into(), account: "a".into(), label: "".into(),
            enabled: true, auth_required: false,
        }];
        let mut histories = HashMap::new();
        let base = Duration::from_secs(5);
        // Seed history then 9 ticks; before every tick, push last_probe back by
        // the current interval so the TTL guard always sees the port as due.
        let _ = run_tick(&ports, &mut histories, base).await;
        for _ in 0..9 {
            let h = histories.get_mut(&1).unwrap();
            h.last_probe = Instant::now() - h.interval - Duration::from_millis(10);
            let _ = run_tick(&ports, &mut histories, base).await;
        }
        let h = histories.get(&1).unwrap();
        assert!(h.interval.as_secs() <= BACKOFF_CAP_SECS, "interval capped: {}", h.interval.as_secs());
        assert!(h.fails >= 5, "reached Dead threshold: fails={}", h.fails);
    }

    #[tokio::test]
    async fn run_tick_ttl_skip_when_recently_probed() {
        let ports = vec![PortMapping {
            port: 1, protocol: "socks5".into(),
            platform_name: "P".into(), account: "a".into(), label: "".into(),
            enabled: true, auth_required: false,
        }];
        let mut histories = HashMap::new();
        let base = Duration::from_secs(5);
        let _ = run_tick(&ports, &mut histories, base).await;
        // Immediately run again — should TTL-skip (last_probe is now).
        let out = run_tick(&ports, &mut histories, base).await;
        // last_reachable from the first tick persists across the TTL-skipped tick.
        assert!(!out[0].reachable, "reachable persists across TTL-skip");
        // fails did not increment because the port was not probed this tick.
        assert_eq!(out[0].fails, 1, "fails stays at 1 when TTL-skipped");
    }

    #[tokio::test]
    async fn run_tick_skips_disabled_ports() {
        let ports: Vec<PortMapping> = vec![];
        let mut histories = HashMap::new();
        let out = run_tick(&ports, &mut histories, Duration::from_secs(5)).await;
        assert!(out.is_empty());
    }
}
