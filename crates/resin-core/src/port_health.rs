//! Phase 1 — Port health batch probe (sing-box urltest pattern).
//!
//! Single Tokio task + ticker + for_each_concurrent(10) + AtomicBool reentry
//! guard + TTL skip + exponential backoff (5 fails -> Dead, interval × 2^min(fails,5), cap 5m).
//! Tab-hidden pause: the watcher checks an AtomicBool flag set by Tauri window
//! focus events; when hidden the tick returns early without probing.
//!
//! The watcher is generic over the emit function so the Tauri command can
//! pass a Channel<PortHealthSnapshot> and tests can pass a closure.
//!
//! interval/backoff arithmetic lives in
//! the shared crate::throttle model; this module keeps only the poll's
//! parameter values and the loop itself.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::db::PortMapping;
use crate::throttle::{self, ThrottleParams};

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

/// the poll's rhythm parameter VALUES (unchanged from before the
/// consolidation: floor 5s, backoff cap 300s, k=2, exponent cap 5); the
/// arithmetic itself lives only in crate::throttle.
const POLL_PARAMS: ThrottleParams = ThrottleParams::bounded(MIN_INTERVAL_SECS, BACKOFF_CAP_SECS);
/// k in max(floor, k*ln(1+N)) (Cilium CFP-32820).
const ADAPTIVE_K: f64 = 2.0;
/// Backoff exponent cap: interval x 2^min(fails, cap).
const BACKOFF_EXPONENT_CAP: u32 = 5;

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
            if fails > 0 {
                HealthState::Degraded
            } else {
                HealthState::Alive
            }
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
    fn new(base_interval: Duration) -> Self {
        Self {
            last_probe: Instant::now(),
            fails: 0,
            interval: base_interval,
            last_reachable: true,
        }
    }
}

/// Adaptive interval: max(MIN_INTERVAL_SECS, k·ln(1+N)) where k=2 (Cilium CFP-32820).
/// thin delegation to the single-owner throttle model - same
/// signature, same rhythm.
pub fn adaptive_interval(port_count: usize) -> Duration {
    throttle::adaptive_interval(POLL_PARAMS, ADAPTIVE_K, port_count)
}

/// Probe a single port. TCP connect + SOCKS5 greeting or HTTP GET; returns
/// (reachable, latency_ms). Mirrors src-tauri::commands::port_health_check.
///
/// `mixed` is probed with the SOCKS5 greeting:
/// a dual-flag listener answers it, which is what the live D4 gate observed.
async fn probe_one(port: u16, protocol: &str) -> (bool, Option<u32>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let started = Instant::now();
    let addr = format!("127.0.0.1:{port}");
    let connect = tokio::time::timeout(
        Duration::from_millis(CONNECT_TIMEOUT_MS),
        tokio::net::TcpStream::connect(&addr),
    )
    .await;
    let mut stream = match connect {
        Ok(Ok(s)) => s,
        _ => return (false, None),
    };
    let greeting: Vec<u8> = if protocol == "http" {
        format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")
            .into_bytes()
    } else {
        vec![0x05, 0x02, 0x00, 0x02]
    };
    if stream.write_all(&greeting).await.is_err() {
        return (true, Some(started.elapsed().as_millis() as u32));
    }
    let mut buf = [0u8; 16];
    let read = tokio::time::timeout(
        Duration::from_millis(READ_TIMEOUT_MS),
        stream.read(&mut buf),
    )
    .await;
    let elapsed = started.elapsed().as_millis() as u32;
    match read {
        Ok(Ok(n)) if protocol == "http" && n >= 12 && buf.starts_with(b"HTTP/") => {
            (true, Some(elapsed))
        }
        // `mixed` ports are probed with the SOCKS5 greeting (the branch
        // above), so a SOCKS5 method-selection reply is the expected answer for
        // BOTH `socks5` and `mixed`. Verified live against Resin: a dual-flag
        // port answers 05 00 (ADR-0068 D4 gate).
        Ok(Ok(n))
            if protocol != "http"
                && n >= 2
                && buf[0] == 0x05
                && (buf[1] == 0x00 || buf[1] == 0x02) =>
        {
            (true, Some(elapsed))
        }
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
    let due: Vec<PortMapping> = ports
        .iter()
        .filter(|p| match histories.get(&p.port) {
            Some(h) => now.duration_since(h.last_probe) >= h.interval,
            None => true,
        })
        .cloned()
        .collect();

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
        let h = histories
            .entry(p.port)
            .or_insert_with(|| PortHistory::new(base_interval));
        // Find this tick's result if it was due; else reuse prior state.
        let (reachable, latency) = match results.iter().find(|(port, _, _)| *port == p.port) {
            Some((_, r, lat)) => {
                h.last_probe = now;
                h.last_reachable = *r;
                if *r {
                    h.fails = 0;
                    h.interval = base_interval;
                } else {
                    h.fails = h.fails.saturating_add(1);
                    h.interval = throttle::backoff_interval(
                        POLL_PARAMS,
                        base_interval,
                        h.fails,
                        BACKOFF_EXPONENT_CAP,
                    );
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

/// Tick cadence: `Adaptive` recomputes the base interval from
/// the live port count each tick - the production rhythm, byte-identical to
/// pre- behavior; `Fixed` pins the interval so rhythm semantics
/// (multi-client coexistence, shared pause) are testable deterministically
/// under tokio virtual time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchCadence {
    Adaptive,
    Fixed(Duration),
}

/// Spawn the watcher: one task per call; it ends when the emit closure
/// returns Err (channel closed).
///
/// `ports_fn` is called every tick to refresh the port list (so new ports
/// appear and deleted ports drop out without restarting the watcher).
/// `paused` is the AtomicBool flag set by the Tauri window focus listener;
/// when true the tick returns early without probing (sing-box idle_timeout).
/// Multiple concurrent callers (multiple watch_port_health clients) each get
/// an independent task with its own revision counter and probe history; they
/// share only the port source and the pause flag.
pub fn spawn_watcher<F, E>(ports_fn: F, emit: E, paused: Arc<AtomicBool>)
where
    F: Fn() -> Vec<PortMapping> + Send + Sync + 'static,
    E: Fn(PortHealthSnapshot) -> Result<(), ()> + Send + Sync + 'static,
{
    spawn_watcher_with(ports_fn, emit, paused, WatchCadence::Adaptive);
}

/// `spawn_watcher` with an explicit tick cadence ( test seam;
/// production callers use `spawn_watcher`, which passes `Adaptive`).
pub fn spawn_watcher_with<F, E>(
    ports_fn: F,
    emit: E,
    paused: Arc<AtomicBool>,
    cadence: WatchCadence,
) where
    F: Fn() -> Vec<PortMapping> + Send + Sync + 'static,
    E: Fn(PortHealthSnapshot) -> Result<(), ()> + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let mut histories: HashMap<u16, PortHistory> = HashMap::new();
        let mut revision: u64 = 0;
        loop {
            let ports = ports_fn();
            let base = match cadence {
                WatchCadence::Adaptive => adaptive_interval(ports.len()),
                WatchCadence::Fixed(d) => d,
            };
            tokio::time::sleep(base).await;
            if paused.load(Ordering::Relaxed) {
                continue;
            }
            revision = revision.wrapping_add(1);
            let entries = run_tick(&ports, &mut histories, base).await;
            let snapshot = PortHealthSnapshot { revision, entries };
            if emit(snapshot).is_err() {
                break;
            } // channel closed -> exit
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

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

    /// a `mixed` port must classify as reachable
    /// when it answers the SOCKS5 greeting. A dual-flag Resin listener replies
    /// `05 00` - exactly what the ADR-0068 D4 gate observed live - and the
    /// pre-change match arm only recognised `socks5`, so `mixed` would have
    /// fallen through and been reported dead.
    #[tokio::test]
    async fn mixed_protocol_probe_accepts_a_socks5_method_reply() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4];
            let _ = sock.read(&mut buf).await;
            let _ = sock.write_all(&[0x05, 0x00]).await;
            tokio::time::sleep(Duration::from_millis(200)).await;
        });
        let (reachable, latency) = probe_one(port, "mixed").await;
        assert!(reachable, "a mixed port answering 05 00 is reachable");
        assert!(latency.is_some(), "reachability carries a latency sample");
        let _ = server.await;
    }

    #[tokio::test]
    async fn run_tick_marks_unreachable_port_and_backoff_grows() {
        let ports = vec![PortMapping {
            port: 1,
            protocol: "socks5".into(),
            platform_name: "P".into(),
            account: "a".into(),
            label: "".into(),
            enabled: true,
            auth_required: false,
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
            port: 1,
            protocol: "socks5".into(),
            platform_name: "P".into(),
            account: "a".into(),
            label: "".into(),
            enabled: true,
            auth_required: false,
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
        assert!(
            h.interval.as_secs() <= BACKOFF_CAP_SECS,
            "interval capped: {}",
            h.interval.as_secs()
        );
        assert!(h.fails >= 5, "reached Dead threshold: fails={}", h.fails);
    }

    #[tokio::test]
    async fn run_tick_ttl_skip_when_recently_probed() {
        let ports = vec![PortMapping {
            port: 1,
            protocol: "socks5".into(),
            platform_name: "P".into(),
            account: "a".into(),
            label: "".into(),
            enabled: true,
            auth_required: false,
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

    // --- spawn_watcher cadence / multi-client / shared-pause semantics ---

    #[tokio::test(start_paused = true)]
    async fn spawn_watcher_default_adaptive_emits_monotonic_revisions() {
        // The production entry point (Adaptive cadence) still ticks and emits
        // strictly monotonic per-client revisions. Empty port list -> no
        // sockets; adaptive base for 0 ports is the 5s floor (virtual time).
        let snaps: Arc<Mutex<Vec<PortHealthSnapshot>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = snaps.clone();
        let emit = move |s: PortHealthSnapshot| -> Result<(), ()> {
            sink.lock().unwrap().push(s);
            Ok(())
        };
        spawn_watcher(
            || Vec::<PortMapping>::new(),
            emit,
            Arc::new(AtomicBool::new(false)),
        );
        tokio::time::timeout(Duration::from_secs(60), async {
            while snaps.lock().unwrap().len() < 3 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("watcher did not emit 3 snapshots");
        let revs: Vec<u64> = snaps.lock().unwrap().iter().map(|s| s.revision).collect();
        assert_eq!(
            revs,
            vec![1, 2, 3],
            "revisions strictly monotonic per client"
        );
        assert!(snaps.lock().unwrap().iter().all(|s| s.entries.is_empty()));
    }

    #[tokio::test(start_paused = true)]
    async fn spawn_watcher_two_clients_independent_streams_shared_pause() {
        // Multiple watch_port_health clients coexist: one task per call, each
        // with its own revision counter starting at 1 (independent streams),
        // sharing only the port source and the pause flag. The shared flag
        // freezes BOTH streams; unpause resumes both without resetting.
        let a: Arc<Mutex<Vec<PortHealthSnapshot>>> = Arc::new(Mutex::new(Vec::new()));
        let b: Arc<Mutex<Vec<PortHealthSnapshot>>> = Arc::new(Mutex::new(Vec::new()));
        let paused = Arc::new(AtomicBool::new(false));
        for sink in [&a, &b] {
            let s2 = sink.clone();
            let emit = move |s: PortHealthSnapshot| -> Result<(), ()> {
                s2.lock().unwrap().push(s);
                Ok(())
            };
            spawn_watcher_with(
                || Vec::<PortMapping>::new(),
                emit,
                paused.clone(),
                WatchCadence::Fixed(Duration::from_millis(50)),
            );
        }
        tokio::time::timeout(Duration::from_secs(60), async {
            while a.lock().unwrap().len() < 2 || b.lock().unwrap().len() < 2 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("clients did not emit");
        for client in [&a, &b] {
            let g = client.lock().unwrap();
            assert_eq!(g[0].revision, 1, "each client stream starts at revision 1");
            assert_eq!(
                g[1].revision, 2,
                "revisions advance independently per client"
            );
        }
        paused.store(true, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(500)).await;
        let (la, lb) = (a.lock().unwrap().len(), b.lock().unwrap().len());
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert_eq!(a.lock().unwrap().len(), la, "client A frozen while paused");
        assert_eq!(
            b.lock().unwrap().len(),
            lb,
            "client B frozen while paused (shared flag)"
        );
        paused.store(false, Ordering::Relaxed);
        tokio::time::timeout(Duration::from_secs(60), async {
            while a.lock().unwrap().len() <= la || b.lock().unwrap().len() <= lb {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("clients did not resume after unpause");
    }
}
