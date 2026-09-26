//! Shell-side port->platform mapping store (ADR-0012).
//!
//! A tiny SQLite store mapping each Entry Port number to a (platform_name,
//! account_string) pair. The multi-port listener reads this on every inbound
//! connection to inject the correct X-Resin-Account header before forwarding
//! to Resin. Reuses the DbPool infrastructure from ADR-0011 (parking_lot::Mutex
//! + WAL + hand-rolled PRAGMA user_version migration).

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[cfg(feature = "db-lock-metrics")]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::LazyLock;
#[cfg(feature = "db-lock-metrics")]
use std::sync::Mutex as StdMutex;
use std::time::Duration;

/// Cross-connection write contention budget. A second process or
/// thread holding a write txn (e.g. the backup snapshot path) makes an
/// un-tuned connection fail instantly with SQLITE_BUSY; 5s absorbs the
/// worst-case snapshot window without masking a real deadlock.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// One row of the port_mappings table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PortMapping {
    pub port: u16,
    pub protocol: String,
    pub platform_name: String,
    pub account: String,
    pub label: String,
    pub enabled: bool,
    pub auth_required: bool,
}

/// R12-H2 (D-004 step1, remedy order B): a dedicated read-only connection
/// for the read paths, lazily opened on first use and kept for the pool's
/// life. hynek's WAL caveat applies - a short-lived reader pays the -shm
/// handshake on every open/close and lands SQLITE_BUSY, so the handle MUST
/// be long-lived (the LazyLock keeps it for the pool's life). `None` for an
/// in-memory pool (a second in-memory handle would be a different database)
/// or when the read-only open failed - both cases fall back to the writer
/// mutex so reads keep working.
type ReadConnLazy = LazyLock<
    Option<Mutex<Connection>>,
    Box<dyn FnOnce() -> Option<Mutex<Connection>> + Send + Sync>,
>;

#[derive(Clone)]
pub struct DbPool(
    Arc<Mutex<Connection>>,
    Arc<ReadConnLazy>,
    #[cfg(feature = "db-lock-metrics")] Arc<LockWaitStats>,
);

/// Shared `None` initializer for pools that have no file-backed read
/// connection (open_in_memory; tests that hand-build a pool).
fn no_read_conn() -> Arc<ReadConnLazy> {
    Arc::new(LazyLock::new(Box::new(|| None)))
}

/// R12-E3, adjudicated r12-wave-f D-002 (register row 'DbPool
/// single-lock (bb8)'): a lock-wait instrument on every DbPool
/// acquisition. The 1 ms existence threshold is anchored to the
/// parking_lot eventual-fairness forcing line (~1 ms; 0.5 ms average);
/// the 5 ms magnitude tier (r12-wave-i D-003.1) sits above it because
/// the fairness forcing line makes ~1 ms waits a SCHEDULER guarantee
/// under contention - existence alone cannot discriminate a real
/// contention event from fairness noise. Verdict history: arm (ii)
/// fired 2026-09-24 and discharged to keep-Mutex - the synthetic probe
/// proved lock saturation exists under contention but is not a
/// production-impact criterion. Arm (i) - THIS counter on real load
/// - is the primary instrument: a real-load wait >= 1 ms re-opens the
/// pool-impl adjudication (successor: deadpool-sqlite; r2d2 is
/// 404-dead on crates.io, recorded; remedy order B read-conn before A
/// migration). The instrument sits behind the opt-in Cargo feature
/// `db-lock-metrics` (r12-wave-i D-003.1): default-off builds compile
/// it out entirely (zero-cost - acquire() collapses to the bare lock),
/// and feature-on builds exist for observation cycles ONLY, never a
/// shipping configuration.
#[cfg(feature = "db-lock-metrics")]
const LOCK_WAIT_REOPEN: Duration = Duration::from_millis(1);

/// Magnitude tier (r12-wave-i D-003.1): waits >= 5 ms. Above this a wait
/// is a real contention event, not parking_lot fairness forcing.
#[cfg(feature = "db-lock-metrics")]
const LOCK_WAIT_MAGNITUDE: Duration = Duration::from_millis(5);

/// Dev-session reservoir cap: counts keep accumulating after the sample
/// vec is full so a long session cannot grow memory unboundedly.
#[cfg(feature = "db-lock-metrics")]
const LOCK_WAIT_SAMPLE_CAP: usize = 16384;

#[cfg(feature = "db-lock-metrics")]
#[derive(Default)]
struct LockWaitStats {
    acquisitions: AtomicU64,
    over_threshold: AtomicU64,
    /// r12-wave-i D-003.1: magnitude tier - writer waits >=
    /// LOCK_WAIT_MAGNITUDE (5 ms), counted separately so existence and
    /// magnitude stay distinguishable.
    over_magnitude: AtomicU64,
    max_wait_micros: AtomicU64,
    wait_micros: StdMutex<Vec<u64>>,
    /// R12-H2 step0 (D-004): last WRITER acquisition callsite - the holder
    /// identity a >=1ms waiter was blocked on (the in-flight/preceding
    /// acquirer). Lets the next real-load WARN name its starver
    /// (candidates per the contention profile: replace_ports/backup
    /// self-check/migrate).
    holder: StdMutex<Option<&'static str>>,
    /// r12-wave-i D-003.1/D-005: dedicated read-conn observation leg.
    /// Reader-vs-reader waits on the read mutex get their own counters +
    /// holder slot so the writer-lock counts stay the pure B-side pivot.
    read_acquisitions: AtomicU64,
    read_over_threshold: AtomicU64,
    read_over_magnitude: AtomicU64,
    read_max_wait_micros: AtomicU64,
    /// Last dedicated-read-conn acquirer callsite - the holder=read-guard
    /// attribution the register's reader-vs-reader reopen-when depends on.
    read_holder: StdMutex<Option<&'static str>>,
}

#[cfg(feature = "db-lock-metrics")]
impl LockWaitStats {
    fn new_shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Record a WRITER-lock acquisition wait attributed to `who`. The
    /// wait lands in the p99 sample reservoir (the arm-(ii) A/B readout
    /// caliber).
    fn observe(&self, wait: Duration, who: &'static str) {
        let micros = wait.as_micros() as u64;
        self.acquisitions.fetch_add(1, Ordering::Relaxed);
        self.max_wait_micros.fetch_max(micros, Ordering::Relaxed);
        Self::observe_domain(
            wait,
            micros,
            who,
            "writer",
            &self.over_threshold,
            &self.over_magnitude,
            &self.holder,
        );
        if let Ok(mut s) = self.wait_micros.lock() {
            if s.len() < LOCK_WAIT_SAMPLE_CAP {
                s.push(micros);
            }
        }
    }

    /// Record a dedicated READ-CONN acquisition wait attributed to `who`
    /// (r12-wave-i D-003.1: the reader-vs-reader leg the D-005 reopen-when
    /// depends on). Read waits stay OUT of the writer p99 sample so the
    /// A/B readout keeps its single-lock caliber.
    fn observe_read(&self, wait: Duration, who: &'static str) {
        let micros = wait.as_micros() as u64;
        self.read_acquisitions.fetch_add(1, Ordering::Relaxed);
        self.read_max_wait_micros
            .fetch_max(micros, Ordering::Relaxed);
        Self::observe_domain(
            wait,
            micros,
            who,
            "read-conn",
            &self.read_over_threshold,
            &self.read_over_magnitude,
            &self.read_holder,
        );
    }

    /// Shared threshold/WARN/holder bookkeeping for one lock domain.
    /// `holder` is the domain's last-acquirer slot - written AFTER the
    /// over-threshold read so a waiter names the acquirer it was actually
    /// blocked behind.
    fn observe_domain(
        wait: Duration,
        micros: u64,
        who: &'static str,
        domain: &'static str,
        over_threshold: &AtomicU64,
        over_magnitude: &AtomicU64,
        holder: &StdMutex<Option<&'static str>>,
    ) {
        if wait >= LOCK_WAIT_MAGNITUDE {
            over_magnitude.fetch_add(1, Ordering::Relaxed);
        }
        if wait >= LOCK_WAIT_REOPEN {
            over_threshold.fetch_add(1, Ordering::Relaxed);
            let holder_name = holder.lock().ok().and_then(|h| *h).unwrap_or("unknown");
            tracing::warn!(
                target: "db",
                wait_micros = micros,
                domain = domain,
                waiter = who,
                holder = holder_name,
                magnitude = wait >= LOCK_WAIT_MAGNITUDE,
                "DbPool lock wait >= 1ms (register arm i threshold)"
            );
        }
        if let Ok(mut h) = holder.lock() {
            *h = Some(who);
        }
    }
}

/// Snapshot of the lock-wait instrument (gated on the opt-in
/// `db-lock-metrics` feature, r12-wave-i D-003.1 - observation builds
/// only). The synthetic concurrency probe reads `wait_micros` for its p99
/// regression/A-B readout (arm (ii) discharged 2026-09-24, keep-Mutex
/// verdict - r12-wave-f D-002); `over_threshold` is arm (i)'s
/// observation surface on real load - the primary instrument. The
/// `read_*` leg observes the dedicated read conn (the reader-vs-reader
/// surface for D-005's reopen-when) without polluting the writer p99.
#[cfg(feature = "db-lock-metrics")]
#[derive(Debug, Default, Clone)]
pub struct LockWaitReport {
    /// Total WRITER-lock acquisitions observed since open/reset.
    pub acquisitions: u64,
    /// Writer waits that hit >= LOCK_WAIT_REOPEN (1 ms existence tier).
    pub over_threshold: u64,
    /// Writer waits >= LOCK_WAIT_MAGNITUDE (5 ms magnitude tier).
    pub over_magnitude: u64,
    /// Largest single writer acquisition wait, microseconds.
    pub max_wait_micros: u64,
    /// Bounded reservoir of per-acquisition WRITER waits (microseconds).
    pub wait_micros: Vec<u64>,
    /// Dedicated read-conn acquisitions observed.
    pub read_acquisitions: u64,
    /// Read-conn waits >= LOCK_WAIT_REOPEN (reader-vs-reader surface).
    pub read_over_threshold: u64,
    /// Read-conn waits >= LOCK_WAIT_MAGNITUDE.
    pub read_over_magnitude: u64,
    /// Largest single read-conn acquisition wait, microseconds.
    pub read_max_wait_micros: u64,
}

impl DbPool {
    /// Single acquisition seam for every WRITER-path DbPool method (read
    /// paths go through read_guard and only land here on fallback). With
    /// `db-lock-metrics` on it times the parking_lot lock acquisition into
    /// the instrument and attributes the waiter callsite; default builds
    /// are the bare lock - zero-cost.
    #[inline]
    fn acquire(&self, _who: &'static str) -> parking_lot::MutexGuard<'_, Connection> {
        #[cfg(feature = "db-lock-metrics")]
        let guard = {
            let t = std::time::Instant::now();
            let g = self.0.lock();
            self.2.observe(t.elapsed(), _who);
            g
        };
        #[cfg(not(feature = "db-lock-metrics"))]
        let guard = self.0.lock();
        guard
    }

    /// Read seam for `list_ports`/`get_port` (D-004 step1): prefer the
    /// dedicated read-only connection so reads never queue behind a writer's
    /// critical section; fall back to the instrumented writer lock when no
    /// dedicated connection exists (in-memory pool, failed read-only open).
    /// With `db-lock-metrics` on, the dedicated read-conn acquisition is
    /// timed too (r12-wave-i D-003.1/D-005: the reader-vs-reader leg) -
    /// landing in the read_* counters, never the writer p99 sample.
    #[inline]
    fn read_guard(&self, who: &'static str) -> parking_lot::MutexGuard<'_, Connection> {
        if let Some(m) = &**self.1 {
            #[cfg(feature = "db-lock-metrics")]
            let guard = {
                let t = std::time::Instant::now();
                let g = m.lock();
                self.2.observe_read(t.elapsed(), who);
                g
            };
            #[cfg(not(feature = "db-lock-metrics"))]
            let guard = m.lock();
            return guard;
        }
        self.acquire(who)
    }

    /// Feature-gated: reset the lock-wait instrument (probe baseline).
    #[cfg(feature = "db-lock-metrics")]
    pub fn lock_wait_reset(&self) {
        let s = &self.2;
        s.acquisitions.store(0, Ordering::Relaxed);
        s.over_threshold.store(0, Ordering::Relaxed);
        s.over_magnitude.store(0, Ordering::Relaxed);
        s.max_wait_micros.store(0, Ordering::Relaxed);
        s.read_acquisitions.store(0, Ordering::Relaxed);
        s.read_over_threshold.store(0, Ordering::Relaxed);
        s.read_over_magnitude.store(0, Ordering::Relaxed);
        s.read_max_wait_micros.store(0, Ordering::Relaxed);
        if let Ok(mut v) = s.wait_micros.lock() {
            v.clear();
        }
        if let Ok(mut h) = s.holder.lock() {
            *h = None;
        }
        if let Ok(mut h) = s.read_holder.lock() {
            *h = None;
        }
    }

    /// Feature-gated: snapshot the lock-wait counters + sample reservoir.
    #[cfg(feature = "db-lock-metrics")]
    pub fn lock_wait_report(&self) -> LockWaitReport {
        let s = &self.2;
        LockWaitReport {
            acquisitions: s.acquisitions.load(Ordering::Relaxed),
            over_threshold: s.over_threshold.load(Ordering::Relaxed),
            over_magnitude: s.over_magnitude.load(Ordering::Relaxed),
            max_wait_micros: s.max_wait_micros.load(Ordering::Relaxed),
            wait_micros: s.wait_micros.lock().map(|v| v.clone()).unwrap_or_default(),
            read_acquisitions: s.read_acquisitions.load(Ordering::Relaxed),
            read_over_threshold: s.read_over_threshold.load(Ordering::Relaxed),
            read_over_magnitude: s.read_over_magnitude.load(Ordering::Relaxed),
            read_max_wait_micros: s.read_max_wait_micros.load(Ordering::Relaxed),
        }
    }

    /// Dev-only: true once the dedicated read-only connection initialized
    /// (file-backed pool). Read paths fall back to the writer lock on false.
    #[cfg(debug_assertions)]
    pub fn read_conn_dedicated(&self) -> bool {
        (&**self.1).is_some()
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("open db: {e}"))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| format!("pragma journal_mode=WAL: {e}"))?;
        // WAL pairs with NORMAL sync: FULL still fsyncs every commit, which
        // WAL makes unnecessary for durability here (checkpoint durability
        // is unaffected; commit-group fsync is what NORMAL relaxes).
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|e| format!("pragma synchronous=NORMAL: {e}"))?;
        Self::configure(&conn)?;
        Self::migrate(&conn)?;
        // Dedicated read conn (D-004 step1): opened lazily on first read so
        // a pool that never reads pays nothing. Read-only + long-lived per
        // the hynek WAL caveat; any failure degrades to the writer lock.
        let read_path = path.to_path_buf();
        let init_read: Box<dyn FnOnce() -> Option<Mutex<Connection>> + Send + Sync> = Box::new(
            move || {
                let c = match Connection::open_with_flags(
                    &read_path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                ) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(target: "db", "dedicated read conn open failed ({e}); reads fall back to the writer lock");
                        return None;
                    }
                };
                if let Err(e) = c.busy_timeout(BUSY_TIMEOUT) {
                    tracing::warn!(target: "db", "dedicated read conn busy_timeout failed ({e}); reads fall back to the writer lock");
                    return None;
                }
                Some(Mutex::new(c))
            },
        );
        Ok(Self(
            Arc::new(Mutex::new(conn)),
            Arc::new(LazyLock::new(init_read)),
            #[cfg(feature = "db-lock-metrics")]
            LockWaitStats::new_shared(),
        ))
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("open in-memory: {e}"))?;
        Self::configure(&conn)?;
        Self::migrate(&conn)?;
        Ok(Self(
            Arc::new(Mutex::new(conn)),
            no_read_conn(),
            #[cfg(feature = "db-lock-metrics")]
            LockWaitStats::new_shared(),
        ))
    }

    fn configure(conn: &Connection) -> Result<(), String> {
        conn.busy_timeout(BUSY_TIMEOUT)
            .map_err(|e| format!("pragma busy_timeout: {e}"))
    }

    fn migrate(conn: &Connection) -> Result<(), String> {
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| format!("read user_version: {e}"))?;
        if v < 2 {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS port_mappings (port INTEGER PRIMARY KEY, protocol TEXT NOT NULL DEFAULT 'socks5', platform_name TEXT NOT NULL, account TEXT NOT NULL, label TEXT NOT NULL DEFAULT '', enabled INTEGER NOT NULL DEFAULT 1); PRAGMA user_version = 2;",
            )
            .map_err(|e| format!("apply migration v2: {e}"))?;
            tracing::info!(target: "db", "migrated to user_version 2 (port_mappings)");
        }
        if v < 3 {
            conn.execute_batch(
                "ALTER TABLE port_mappings ADD COLUMN auth_required INTEGER NOT NULL DEFAULT 1; PRAGMA user_version = 3;",
            )
            .map_err(|e| format!("apply migration v3: {e}"))?;
            tracing::info!(target: "db", "migrated to user_version 3 (auth_required column)");
        }
        if v < 4 {
            // (ADR-0068 D3): the entry-port protocol
            // enum gained `mixed` and `socks5` was tightened to SOCKS5-only. A
            // pre-change `socks5` row produced BOTH engine flags, so it is
            // rewritten to the flag-preserving `mixed` - the "existing ports
            // migrate once" half of the decision for the rows that seed the
            // whitebox when the file is absent. The table is rebuilt rather than
            // only UPDATEd so the column DEFAULT stops advertising the
            // pre-change vocabulary: a future insert that omits `protocol` must
            // land on the new default, not on a SOCKS5-only port.
            conn.execute_batch(
                "
                BEGIN;
                CREATE TABLE port_mappings_v4 (
                    port INTEGER PRIMARY KEY,
                    protocol TEXT NOT NULL DEFAULT 'mixed',
                    platform_name TEXT NOT NULL,
                    account TEXT NOT NULL,
                    label TEXT NOT NULL DEFAULT '',
                    enabled INTEGER NOT NULL DEFAULT 1,
                    auth_required INTEGER NOT NULL DEFAULT 1
                );
                INSERT INTO port_mappings_v4
                    (port, protocol, platform_name, account, label, enabled, auth_required)
                    SELECT port,
                           CASE WHEN lower(trim(protocol)) = 'socks5' THEN 'mixed' ELSE protocol END,
                           platform_name, account, label, enabled, auth_required
                    FROM port_mappings;
                DROP TABLE port_mappings;
                ALTER TABLE port_mappings_v4 RENAME TO port_mappings;
                PRAGMA user_version = 4;
                COMMIT;
                ",
            )
            .map_err(|e| format!("apply migration v4: {e}"))?;
            tracing::info!(target: "db", "migrated to user_version 4 (protocol enum: mixed default, legacy socks5 -> mixed)");
        }
        Ok(())
    }

    pub fn list_ports(&self) -> Result<Vec<PortMapping>, String> {
        let conn = self.read_guard("list_ports");
        let mut stmt = conn
            .prepare("SELECT port, protocol, platform_name, account, label, enabled, auth_required FROM port_mappings ORDER BY port")
            .map_err(|e| format!("prepare list_ports: {e}"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PortMapping {
                    port: r.get(0)?,
                    protocol: r.get(1)?,
                    platform_name: r.get(2)?,
                    account: r.get(3)?,
                    label: r.get(4)?,
                    enabled: r.get::<_, i64>(5)? != 0,
                    auth_required: r.get::<_, i64>(6)? != 0,
                })
            })
            .map_err(|e| format!("query_map list_ports: {e}"))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| format!("row: {e}"))?);
        }
        Ok(out)
    }

    pub fn upsert_port(&self, m: &PortMapping) -> Result<(), String> {
        let conn = self.acquire("upsert_port");
        conn.execute(
            "INSERT INTO port_mappings (port, protocol, platform_name, account, label, enabled, auth_required) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(port) DO UPDATE SET protocol = excluded.protocol, platform_name = excluded.platform_name, account = excluded.account, label = excluded.label, enabled = excluded.enabled, auth_required = excluded.auth_required",
            params![m.port, m.protocol, m.platform_name, m.account, m.label, m.enabled as i64, m.auth_required as i64],
        )
        .map_err(|e| format!("upsert port_mappings: {e}"))?;
        Ok(())
    }

    pub fn delete_port(&self, port: u16) -> Result<(), String> {
        let conn = self.acquire("delete_port");
        conn.execute("DELETE FROM port_mappings WHERE port = ?1", params![port])
            .map_err(|e| format!("delete port_mappings: {e}"))?;
        Ok(())
    }

    /// Replace the complete port map in one SQLite transaction. The caller has
    /// already validated the desired configuration; a DB error leaves the old
    /// map untouched.
    pub fn replace_ports(&self, mappings: &[PortMapping]) -> Result<(), String> {
        let mut conn = self.acquire("replace_ports");
        // BEGIN IMMEDIATE: take the RESERVED lock at BEGIN, not at
        // the first write — a deferred txn that upgrades mid-flight can hit
        // SQLITE_BUSY_SNAPSHOT under WAL when a reader advanced past it, and
        // the whole DELETE+INSERT batch would have to abort. Grabbing the
        // write lock up front makes the batch either start cleanly (after
        // busy_timeout) or fail fast with nothing half-applied.
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| format!("begin replace_ports: {e}"))?;
        tx.execute("DELETE FROM port_mappings", [])
            .map_err(|e| format!("clear port_mappings: {e}"))?;
        for m in mappings {
            tx.execute(
                "INSERT INTO port_mappings (port, protocol, platform_name, account, label, enabled, auth_required) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![m.port, m.protocol, m.platform_name, m.account, m.label, m.enabled, m.auth_required],
            ).map_err(|e| format!("insert port_mappings: {e}"))?;
        }
        tx.commit()
            .map_err(|e| format!("commit replace_ports: {e}"))
    }
    pub fn get_port(&self, port: u16) -> Result<Option<PortMapping>, String> {
        let conn = self.read_guard("get_port");
        let row = conn
            .query_row(
                "SELECT port, protocol, platform_name, account, label, enabled, auth_required FROM port_mappings WHERE port = ?1",
                params![port],
                |r| {
                    Ok(PortMapping {
                        port: r.get(0)?,
                        protocol: r.get(1)?,
                        platform_name: r.get(2)?,
                        account: r.get(3)?,
                        label: r.get(4)?,
                        enabled: r.get::<_, i64>(5)? != 0,
                        auth_required: r.get::<_, i64>(6)? != 0,
                    })
                },
            )
            .optional();
        match row {
            Ok(o) => Ok(o),
            Err(e) => Err(format!("get port_mapping: {e}")),
        }
    }
}

/// Produce a consistent, self-checked copy of a live SQLite database.
///
/// `VACUUM INTO` is the sqlite.org-sanctioned way to snapshot a running
/// database: it opens the source read-only, writes a brand-new database and
/// leaves the source untouched. A plain file copy of a live database is the
/// failure mode the SQLite documentation names explicitly - it can capture a
/// torn page or a half-applied WAL.
///
/// The copy is then reopened and `PRAGMA quick_check`-ed, so a snapshot that
/// somehow landed malformed is reported as a failure instead of being handed
/// back as if it were trustworthy.
///
/// This never writes to `src`; the destination is a fresh file. Callers keep
/// the ADR-0050-bis constraint on Resin private state.
pub fn snapshot_db_readonly(src: &Path, dst: &Path) -> Result<(), String> {
    if !src.is_file() {
        return Err(format!("snapshot: source is not a file: {}", src.display()));
    }
    if dst.exists() {
        std::fs::remove_file(dst).map_err(|e| format!("snapshot: clear {}: {e}", dst.display()))?;
    }
    let dst_str = dst.to_string_lossy().to_string();
    let conn = Connection::open_with_flags(src, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("snapshot: open {} read-only: {e}", src.display()))?;
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(|e| format!("snapshot: busy_timeout {}: {e}", src.display()))?;
    conn.execute("VACUUM INTO ?1", [dst_str.as_str()])
        .map_err(|e| format!("snapshot: VACUUM INTO {}: {e}", dst.display()))?;
    drop(conn);
    let check = Connection::open_with_flags(dst, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("snapshot: reopen {}: {e}", dst.display()))?;
    let verdict: String = check
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(|e| format!("snapshot: quick_check {}: {e}", dst.display()))?;
    if verdict != "ok" {
        return Err(format!("snapshot: quick_check reported '{verdict}'"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_v2_creates_table_idempotently() {
        let pool = DbPool::open_in_memory().unwrap();
        let pool2 = DbPool::open_in_memory().unwrap();
        let conn = pool2.0.lock();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 4);
        drop(conn);
        drop(pool);
    }

    /// a v3-era table carried the pre-change
    /// vocabulary, where `socks5` also opened HTTP forwarding. Migration v4
    /// rewrites those rows to the flag-preserving `mixed` and moves the column
    /// default off the retired token so a future insert that omits `protocol`
    /// cannot land on a SOCKS5-only port by accident.
    #[test]
    fn migrate_v4_rewrites_legacy_socks5_and_moves_the_column_default() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE port_mappings (port INTEGER PRIMARY KEY, protocol TEXT NOT NULL DEFAULT 'socks5', platform_name TEXT NOT NULL, account TEXT NOT NULL, label TEXT NOT NULL DEFAULT '', enabled INTEGER NOT NULL DEFAULT 1, auth_required INTEGER NOT NULL DEFAULT 1); PRAGMA user_version = 3;",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO port_mappings (port, protocol, platform_name, account, label, enabled, auth_required) VALUES (17990, 'socks5', 'A', 'a', '', 1, 1), (17991, 'http', 'B', 'b', '', 1, 1), (17992, 'mixed', 'C', 'c', '', 1, 1)",
            [],
        )
        .unwrap();

        DbPool::migrate(&conn).unwrap();
        let pool = DbPool(
            Arc::new(Mutex::new(conn)),
            no_read_conn(),
            #[cfg(feature = "db-lock-metrics")]
            LockWaitStats::new_shared(),
        );
        let rows = pool.list_ports().unwrap();
        let protocol_of = |p: u16| {
            rows.iter()
                .find(|r| r.port == p)
                .expect("row must survive the table rebuild")
                .protocol
                .clone()
        };
        assert_eq!(
            protocol_of(17990),
            "mixed",
            "legacy socks5 is flag-preserving"
        );
        assert_eq!(protocol_of(17991), "http", "http keeps its meaning");
        assert_eq!(
            protocol_of(17992),
            "mixed",
            "an already-mixed row is untouched"
        );

        let ddl: String = pool
            .0
            .lock()
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'port_mappings'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            ddl.contains("DEFAULT 'mixed'"),
            "column default moved to mixed: {ddl}"
        );
        assert!(
            !ddl.contains("DEFAULT 'socks5'"),
            "retired token must not survive: {ddl}"
        );

        // Idempotent: a second pass at the already-current version is a no-op.
        DbPool::migrate(&pool.0.lock()).unwrap();
    }

    #[test]
    fn upsert_port_inserts_then_updates() {
        let pool = DbPool::open_in_memory().unwrap();
        pool.upsert_port(&PortMapping {
            port: 17990,
            protocol: "socks5".into(),
            platform_name: "OpenAI".into(),
            account: "port-17990".into(),
            label: "key-A".into(),
            enabled: true,
            auth_required: true,
        })
        .unwrap();
        pool.upsert_port(&PortMapping {
            port: 17990,
            protocol: "http".into(),
            platform_name: "Anthropic".into(),
            account: "port-17990".into(),
            label: "key-A-updated".into(),
            enabled: false,
            auth_required: true,
        })
        .unwrap();
        let got = pool.get_port(17990).unwrap().unwrap();
        assert_eq!(got.protocol, "http");
        assert_eq!(got.platform_name, "Anthropic");
        assert!(!got.enabled);
    }

    #[test]
    fn distinct_ports_do_not_collide() {
        let pool = DbPool::open_in_memory().unwrap();
        pool.upsert_port(&PortMapping {
            port: 17990,
            protocol: "socks5".into(),
            platform_name: "A".into(),
            account: "p1".into(),
            label: "k1".into(),
            enabled: true,
            auth_required: true,
        })
        .unwrap();
        pool.upsert_port(&PortMapping {
            port: 17991,
            protocol: "socks5".into(),
            platform_name: "B".into(),
            account: "p2".into(),
            label: "k2".into(),
            enabled: true,
            auth_required: true,
        })
        .unwrap();
        let all = pool.list_ports().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].port, 17990);
        assert_eq!(all[1].port, 17991);
    }

    #[test]
    fn delete_port_removes_row() {
        let pool = DbPool::open_in_memory().unwrap();
        pool.upsert_port(&PortMapping {
            port: 18000,
            protocol: "http".into(),
            platform_name: "X".into(),
            account: "p3".into(),
            label: "k3".into(),
            enabled: true,
            auth_required: true,
        })
        .unwrap();
        pool.delete_port(18000).unwrap();
        assert!(pool.get_port(18000).unwrap().is_none());
    }

    #[test]
    fn replace_ports_swaps_full_map() {
        let pool = DbPool::open_in_memory().unwrap();
        pool.upsert_port(&PortMapping {
            port: 17990,
            protocol: "socks5".into(),
            platform_name: "A".into(),
            account: "a".into(),
            label: "".into(),
            enabled: true,
            auth_required: true,
        })
        .unwrap();
        pool.replace_ports(&[PortMapping {
            port: 17991,
            protocol: "http".into(),
            platform_name: "B".into(),
            account: "b".into(),
            label: "x".into(),
            enabled: true,
            auth_required: true,
        }])
        .unwrap();
        let all = pool.list_ports().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].port, 17991);
        assert_eq!(all[0].protocol, "http");
        assert!(pool.get_port(17990).unwrap().is_none());
    }

    /// ADR-0070: the backup snapshot must be a consistent,
    /// self-checked copy of a LIVE database (the pool keeps a WAL writer open),
    /// and must refuse a source that is not a file rather than handing back an
    /// empty snapshot the caller would treat as real.
    #[test]
    fn snapshot_db_readonly_copies_a_live_database_and_refuses_a_missing_source() {
        let dir = std::env::temp_dir().join(format!("resin-db-snapshot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.db");
        let dst = dir.join("dst.db");
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&dst);

        // Live writer: the pool holds the source open in WAL mode, which is
        // exactly the state a bare file copy would copy inconsistently.
        let pool = DbPool::open(&src).unwrap();
        pool.upsert_port(&PortMapping {
            port: 17990,
            protocol: "mixed".into(),
            platform_name: "OpenAI".into(),
            account: "port-17990".into(),
            label: "key-A".into(),
            enabled: true,
            auth_required: true,
        })
        .unwrap();

        snapshot_db_readonly(&src, &dst).unwrap();

        // The snapshot is a standalone database carrying the same row.
        let copy = DbPool::open(&dst).unwrap();
        let rows = copy.list_ports().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].port, 17990);
        assert_eq!(rows[0].protocol, "mixed");
        assert_eq!(rows[0].platform_name, "OpenAI");

        let missing = dir.join("does-not-exist.db");
        assert!(
            snapshot_db_readonly(&missing, &dst).is_err(),
            "a non-file source must be refused"
        );

        drop(copy);
        drop(pool);
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&dst);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every pooled connection carries busy_timeout >= 5s so a
    /// concurrent writer (a second connection, a live snapshot) never turns
    /// into an instant SQLITE_BUSY failure.
    #[test]
    fn open_sets_busy_timeout_at_least_five_seconds() {
        let pool = DbPool::open_in_memory().unwrap();
        let ms: i64 = pool
            .0
            .lock()
            .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
            .unwrap();
        assert!(ms >= 5000, "busy_timeout must be >= 5000ms, got {ms}");
    }

    /// A second connection's write WAITS on a held write txn instead
    /// of failing instantly — the busy_timeout contract in action.
    #[test]
    fn cross_connection_write_waits_through_busy_timeout() {
        let dir = std::env::temp_dir().join(format!("resin-db-busy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("busy.db");
        let _ = std::fs::remove_file(&path);

        // Migrate first via the pool so port_mappings exists for both
        // connections (the holder thread opens a raw Connection).
        let b = DbPool::open(&path).unwrap();

        // Connection A lives on a holder thread (Transaction borrows its
        // Connection, so the txn cannot cross the boundary — the connection
        // itself can). The channel proves BEGIN IMMEDIATE landed before B
        // writes, making the contention deterministic.
        let path2 = path.clone();
        let (begun_tx, begun_rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let mut conn = Connection::open(&path2).unwrap();
            conn.busy_timeout(BUSY_TIMEOUT).unwrap();
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            tx.execute(
                "INSERT INTO port_mappings (port, protocol, platform_name, account, label, enabled, auth_required) VALUES (19000, 'mixed', 'A', 'a', '', 1, 1)",
                [],
            )
            .unwrap();
            begun_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(150));
            tx.commit().unwrap();
        });
        begun_rx.recv().unwrap(); // A holds the write lock now

        // B's write must wait ~150ms through busy_timeout and then succeed,
        // not return SQLITE_BUSY instantly.
        b.upsert_port(&PortMapping {
            port: 19001,
            protocol: "http".into(),
            platform_name: "B".into(),
            account: "b".into(),
            label: "".into(),
            enabled: true,
            auth_required: true,
        })
        .expect("busy_timeout must absorb the held write txn");
        holder.join().unwrap();

        assert!(b.get_port(19001).unwrap().is_some());
        drop(b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// replace_ports must begin its txn with BEGIN IMMEDIATE (write
    /// lock at BEGIN), and the whole write path must stay inside the control-
    /// plane latency budget (D-004: p99 <= 50ms).
    #[test]
    fn replace_ports_begins_immediate_and_meets_p99_budget() {
        let src = std::fs::read_to_string("src/db.rs").expect("db.rs readable from crate root");
        let start = src
            .find("fn replace_ports(")
            .expect("replace_ports present");
        let body = &src[start..];
        let end = body.find("\n    }\n").unwrap_or(body.len());
        assert!(
            body[..end].contains("TransactionBehavior::Immediate"),
            "replace_ports must BEGIN IMMEDIATE"
        );

        // Latency budget check on the real write path: repeated full-map
        // replaces on a file-backed WAL database. Sequential writes isolate
        // the per-commit cost (no artificial contention); p99 over 200
        // samples leaves large headroom for CI-runner jitter while still
        // proving the control-plane write stays well under 50ms.
        let dir = std::env::temp_dir().join(format!("resin-db-p99-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p99.db");
        let _ = std::fs::remove_file(&path);
        let pool = DbPool::open(&path).unwrap();
        let map = |i: usize| PortMapping {
            port: (20000 + i) as u16,
            protocol: "mixed".into(),
            platform_name: "P".into(),
            account: format!("acct-{i}"),
            label: "".into(),
            enabled: true,
            auth_required: true,
        };
        let mut samples = Vec::with_capacity(200);
        for i in 0..200 {
            let rows: Vec<PortMapping> = (0..8).map(|j| map(j)).collect();
            let t = std::time::Instant::now();
            pool.replace_ports(&rows).unwrap();
            samples.push(t.elapsed());
            let _ = i;
        }
        samples.sort();
        let p99 = samples[(samples.len() * 99 / 100).min(samples.len() - 1)];
        assert!(
            p99 <= Duration::from_millis(50),
            "replace_ports p99 must be <= 50ms (D-004), got {:?}",
            p99
        );

        drop(pool);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// R12-E3 register arm (ii), adjudicated r12-wave-f D-002: synthetic
    /// concurrency probe measuring the acquisition-wait p99. The
    /// registered 1 ms arm FIRED on first measurement (CI run
    /// 35959139983, 2026-09-24: p99=9615us over 200 samples, 76/200 >=
    /// 1ms) and discharged to a keep-Mutex verdict - saturation
    /// existence proven, production impact not (a fairness-forced
    /// microbenchmark is not a production criterion; emschwartz/abseil
    /// per the ledger). Post-verdict semantics: this probe is a
    /// regression/A-B instrument only - it asserts instrument sanity
    /// plus a hang-pathology bound (p99 < 250ms - 25x the observed 9.9ms
    /// max, catching an unreleased lock / pathological regression). Arm
    /// (i) - the dev counters on real load - is the primary instrument;
    /// arm (iii)'s probe leg is dropped; arm (iv)'s 2026-10-20 hard date
    /// is unchanged.
    /// R12-H2 (D-004 step1) reframed the target: file-backed reads now go
    /// through the dedicated read conn and never touch the instrumented
    /// writer lock, so the saturation hammer moved from `list_ports` to
    /// the WRITER path (`upsert_port` - the remaining real contention
    /// surface alongside replace_ports/backup/migrate). The probe still
    /// measures acquisition-wait p99 on the single writer lock, then
    /// asserts the step1 invariant: a read hammer on the same file-backed
    /// pool registers ZERO additional acquisitions (reads are invisible
    /// to the instrument by design - the machine-checkable B-side pivot).
    /// Feature-gated like the instrument itself - a default test build
    /// has no instrument to read.
    #[cfg(feature = "db-lock-metrics")]
    #[test]
    fn concurrent_list_ports_lock_wait_probe_reports_p99() {
        let dir = std::env::temp_dir().join(format!("resin-db-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("probe.db");
        let _ = std::fs::remove_file(&path);
        let pool = DbPool::open(&path).unwrap();
        for i in 0..64u16 {
            pool.upsert_port(&PortMapping {
                port: 30000 + i,
                protocol: "mixed".into(),
                platform_name: "P".into(),
                account: format!("a{i}"),
                label: "".into(),
                enabled: true,
                auth_required: true,
            })
            .unwrap();
        }
        pool.lock_wait_reset();

        const THREADS: usize = 8;
        const ROUNDS: usize = 25;
        let mut handles = Vec::with_capacity(THREADS);
        for t in 0..THREADS {
            let p = pool.clone();
            handles.push(std::thread::spawn(move || {
                for r in 0..ROUNDS {
                    // Writer-path hammer: each upsert is one instrumented
                    // acquisition of the single writer lock.
                    p.upsert_port(&PortMapping {
                        port: 31000 + (t * ROUNDS + r) as u16,
                        protocol: "mixed".into(),
                        platform_name: "P".into(),
                        account: format!("t{t}r{r}"),
                        label: "".into(),
                        enabled: true,
                        auth_required: true,
                    })
                    .unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let report = pool.lock_wait_report();
        assert!(
            report.acquisitions >= (THREADS * ROUNDS) as u64,
            "instrument must observe every probe acquisition, got {}",
            report.acquisitions
        );
        let acquisitions_after_writes = report.acquisitions;
        let mut s = report.wait_micros.clone();
        s.sort_unstable();
        assert!(!s.is_empty());
        let p99 = s[(s.len() * 99 / 100).min(s.len() - 1)];
        // Post-verdict instrument line (r12-wave-f D-002): arm (ii) fired
        // 2026-09-24 and discharged to a keep-Mutex verdict, so this
        // print is a regression/A-B readout - printed, not gated; arm (i)
        // on real load is the primary reopen instrument.
        println!(
            "lock-wait probe: p99={p99}us max={}us over_threshold={} samples={} (regression/A-B instrument; arm ii FIRED+discharged 2026-09-24 run 35959139983, keep-Mutex verdict)",
            report.max_wait_micros,
            report.over_threshold,
            s.len()
        );
        assert!(
            p99 < 250_000,
            "hang-pathology bound: acquisition wait p99 must stay < 250ms, got {p99}us"
        );

        // D-004 step1 pivot assertion: the same read hammer that arm (ii)
        // once saturated the lock with now registers ZERO acquisitions -
        // file-backed reads ride the dedicated read conn.
        let mut readers = Vec::with_capacity(THREADS);
        for _ in 0..THREADS {
            let p = pool.clone();
            readers.push(std::thread::spawn(move || {
                for _ in 0..ROUNDS {
                    assert_eq!(p.list_ports().unwrap().len(), 64 + THREADS * ROUNDS);
                }
            }));
        }
        for h in readers {
            h.join().unwrap();
        }
        assert_eq!(
            pool.lock_wait_report().acquisitions,
            acquisitions_after_writes,
            "file-backed reads must not touch the instrumented writer lock"
        );

        drop(pool);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// R12-H2 step0 (D-004): every acquisition attributes its callsite -
    /// the stats record the LAST acquirer as the holder identity a >=1ms
    /// waiter was blocked on. In-memory reads land on the instrumented
    /// fallback, so their callsites attribute identically. r12-wave-i
    /// D-003.1 adds the dedicated read-conn leg: file-backed reads
    /// attribute on the read domain's own holder slot.
    #[cfg(feature = "db-lock-metrics")]
    #[test]
    fn lock_wait_stats_records_last_acquirer_identity() {
        let pool = DbPool::open_in_memory().unwrap();
        let m = |port: u16| PortMapping {
            port,
            protocol: "mixed".into(),
            platform_name: "P".into(),
            account: "a".into(),
            label: "".into(),
            enabled: true,
            auth_required: true,
        };
        pool.upsert_port(&m(17990)).unwrap();
        pool.delete_port(17990).unwrap();
        assert_eq!(
            *pool.2.holder.lock().unwrap(),
            Some("delete_port"),
            "the last acquirer must attribute its callsite"
        );
        pool.list_ports().unwrap(); // fallback read also attributes
        assert_eq!(
            *pool.2.holder.lock().unwrap(),
            Some("list_ports"),
            "a fallback read attributes its callsite too"
        );
    }

    /// R12-H2 step1 (D-004): a file-backed pool serves list/get through the
    /// dedicated read-only connection - a separate handle on the same WAL
    /// database, lazily opened on first read and kept for the pool's life
    /// (hynek: short-lived readers pay the -shm handshake). In-memory pools
    /// stay on the instrumented writer-lock fallback.
    #[cfg(debug_assertions)]
    #[test]
    fn file_backed_reads_use_the_dedicated_read_conn() {
        let dir = std::env::temp_dir().join(format!("resin-db-readconn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("read.db");
        let _ = std::fs::remove_file(&path);

        let pool = DbPool::open(&path).unwrap();
        pool.upsert_port(&PortMapping {
            port: 17990,
            protocol: "mixed".into(),
            platform_name: "OpenAI".into(),
            account: "a".into(),
            label: "".into(),
            enabled: true,
            auth_required: true,
        })
        .unwrap();

        let rows = pool.list_ports().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].platform_name, "OpenAI");
        assert!(
            pool.read_conn_dedicated(),
            "the dedicated conn must be live after the first read"
        );
        assert_eq!(pool.get_port(17990).unwrap().unwrap().port, 17990);
        assert!(pool.get_port(17991).unwrap().is_none());

        // A commit after reads stays visible to the same long-lived conn
        // (each read is a fresh WAL snapshot transaction).
        pool.delete_port(17990).unwrap();
        assert!(pool.list_ports().unwrap().is_empty());

        drop(pool);
        let _ = std::fs::remove_dir_all(&dir);

        let mem = DbPool::open_in_memory().unwrap();
        assert!(
            !mem.read_conn_dedicated(),
            "an in-memory pool must not claim a dedicated read conn"
        );
        assert!(mem.list_ports().unwrap().is_empty());
    }

    /// The dedicated read conn is a second handle on the same WAL file:
    /// readers keep observing committed snapshots while a writer is active,
    /// never queueing on the writer mutex. Functional closed-loop - the
    /// latency shape is the dev-session A/B readout (R12-H2 step1).
    #[test]
    fn dedicated_read_conn_serves_reads_while_writer_is_active() {
        let dir = std::env::temp_dir().join(format!("resin-db-readrace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("race.db");
        let _ = std::fs::remove_file(&path);

        let pool = DbPool::open(&path).unwrap();
        let rows = |shift: u16| -> Vec<PortMapping> {
            (0..8u16)
                .map(|i| PortMapping {
                    port: 21000 + shift * 8 + i,
                    protocol: "mixed".into(),
                    platform_name: "P".into(),
                    account: format!("a{}", shift * 8 + i),
                    label: "".into(),
                    enabled: true,
                    auth_required: true,
                })
                .collect()
        };
        pool.replace_ports(&rows(0)).unwrap();

        let writer = {
            let p = pool.clone();
            std::thread::spawn(move || {
                for i in 0..20u16 {
                    p.replace_ports(&rows(i % 4)).unwrap();
                }
            })
        };
        // Whichever commit is current, the map always holds exactly 8 rows;
        // reads on the dedicated conn keep flowing while replace_ports churns.
        for _ in 0..200 {
            assert_eq!(pool.list_ports().unwrap().len(), 8);
        }
        writer.join().unwrap();

        drop(pool);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
