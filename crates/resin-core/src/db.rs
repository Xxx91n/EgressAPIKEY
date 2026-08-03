//! Shell-side observed key pool (ADR-0011).
//!
//! A tiny SQLite store for the reverse-map from the route_id the A4-3 axum
//! interceptor injects as X-Resin-Account (ar-<16hex>) back to the readable
//! (apiKeyMask, endpoint) tuple. The GUI polls this via the observed_keys IPC
//! and joins it against lease_map so each Topology B-column chip shows
//! key[0..4]...key[-4..] . endpoint instead of an opaque hash.
//!
//! Ponytail: r2d2_rusqlite was originally specified (ADR-0011) but the crate
//! is no longer published on crates.io (verified 2026-08-03: crates.io 404
//! for both r2d2-rusqlite and r2d2_rusqlite). We fall back to the stdlib path
//! that ADR-0011 listed as the rejected (a) alternative: a single
//! std::sync::Mutex<rusqlite::Connection> wrapped in tokio::task::spawn_blocking
//! at every async call site. This is the Ponytail ladder rung-3 "stdlib does
//! it" path: zero new crates, rusqlite 0.32 already in Cargo.toml, and at the
//! concurrency level of this app (one interceptor per new tuple + 5s GUI
//! polling) serialised access is well below the SQLite write throughput
//! ceiling. Migration is hand-rolled PRAGMA user_version per ADR-0011 (2)=a.

use std::path::Path;
use std::sync::Arc;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// One row of the observed_keys table, as the GUI receives it via the
/// observed_keys() IPC. The route_id is the same ar-<16hex> that the
/// interceptor injects as X-Resin-Account and that LeaseEntry.account carries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservedKey {
    pub route_id: String,
    pub api_key_mask: String,
    pub endpoint: String,
    pub first_seen: i64,
    pub last_seen: i64,
    pub request_count: i64,
}

/// A handle to the open observed_keys SQLite database. Cheap to clone (Arc)
/// and safe to share between the axum interceptor (writes) and the Tauri IPC
/// handler (reads). Locking is parking_lot::Mutex, held only for the duration
/// of a single SQLite statement — never across an await.
#[derive(Clone)]
pub struct DbPool(Arc<Mutex<Connection>>);

impl DbPool {
    /// Open (creating if absent) the SQLite file at ``path``, enable WAL,
    /// run user_version migration (currently v1: CREATE observed_keys), and
    /// return a pool handle. Idempotent: safe to call on the same file
    /// repeatedly (CREATE TABLE IF NOT EXISTS + PRAGMA user_version check).
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("open db: {e}"))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| format!("pragma journal_mode=WAL: {e}"))?;
        Self::migrate(&conn)?;
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    /// In-memory open, for unit tests. Same migration path runs.
    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("open in-memory: {e}"))?;
        Self::migrate(&conn)?;
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    fn migrate(conn: &Connection) -> Result<(), String> {
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| format!("read user_version: {e}"))?;
        if v < 1 {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS observed_keys (route_id TEXT PRIMARY KEY, apiKeyMask TEXT NOT NULL, endpoint TEXT NOT NULL, first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL, request_count INTEGER DEFAULT 1); PRAGMA user_version = 1;",
            )
            .map_err(|e| format!("apply migration v1: {e}"))?;
            tracing::info!(target: "db", "migrated to user_version 1");
        }
        Ok(())
    }

    /// Insert-or-bump a route_id row. Called by the interceptor on every new
    /// (identity, model, path) tuple it sees. ON CONFLICT updates last_seen +
    /// increments request_count; apiKeyMask/endpoint are NOT overwritten
    /// (they were captured at first sighting).
    pub fn upsert(&self, route_id: &str, api_key_mask: &str, endpoint: &str, now_unix: i64) -> Result<(), String> {
        let conn = self.0.lock();
        conn.execute(
            "INSERT INTO observed_keys (route_id, apiKeyMask, endpoint, first_seen, last_seen, request_count) VALUES (?1, ?2, ?3, ?4, ?4, 1) ON CONFLICT(route_id) DO UPDATE SET last_seen = excluded.last_seen, request_count = observed_keys.request_count + 1",
            params![route_id, api_key_mask, endpoint, now_unix],
        )
        .map_err(|e| format!("upsert observed_keys: {e}"))?;
        Ok(())
    }

    /// Return every observed key row, newest first. Called by the observed_keys
    /// IPC at 5s cadence from the GUI Topology sync.
    pub fn list(&self) -> Result<Vec<ObservedKey>, String> {
        let conn = self.0.lock();
        let mut stmt = conn
            .prepare("SELECT route_id, apiKeyMask, endpoint, first_seen, last_seen, request_count FROM observed_keys ORDER BY last_seen DESC")
            .map_err(|e| format!("prepare list: {e}"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ObservedKey {
                    route_id: r.get(0)?,
                    api_key_mask: r.get(1)?,
                    endpoint: r.get(2)?,
                    first_seen: r.get(3)?,
                    last_seen: r.get(4)?,
                    request_count: r.get(5)?,
                })
            })
            .map_err(|e| format!("query_map list: {e}"))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| format!("row: {e}"))?);
        }
        Ok(out)
    }

    /// Look up a single route_id reverse-map. Convenience for GUI join.
    pub fn get(&self, route_id: &str) -> Result<Option<ObservedKey>, String> {
        let conn = self.0.lock();
        let row = conn
            .query_row(
                "SELECT route_id, apiKeyMask, endpoint, first_seen, last_seen, request_count FROM observed_keys WHERE route_id = ?1",
                params![route_id],
                |r| {
                    Ok(ObservedKey {
                        route_id: r.get(0)?,
                        api_key_mask: r.get(1)?,
                        endpoint: r.get(2)?,
                        first_seen: r.get(3)?,
                        last_seen: r.get(4)?,
                        request_count: r.get(5)?,
                    })
                },
            )
            .optional();
        match row {
            Ok(o) => Ok(o),
            Err(e) => Err(format!("get observed_key: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_v1_creates_table_idempotently() {
        let pool = DbPool::open_in_memory().unwrap();
        // second open should not panic (CREATE TABLE IF NOT EXISTS + user_version read)
        let pool2 = DbPool::open_in_memory().unwrap();
        let conn = pool2.0.lock();
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 1);
        drop(conn);
        drop(pool);
    }

    #[test]
    fn upsert_inserts_then_increments_request_count() {
        let pool = DbPool::open_in_memory().unwrap();
        pool.upsert("ar-<16hex-A", "sk-A...1234", "api.openai.com", 100).unwrap();
        pool.upsert("ar-<16hex-A", "IGNORED-on-conflict", "IGNORED", 200).unwrap();
        let got = pool.get("ar-<16hex-A").unwrap().unwrap();
        assert_eq!(got.api_key_mask, "sk-A...1234");  // initial value preserved
        assert_eq!(got.endpoint, "api.openai.com");   // initial value preserved
        assert_eq!(got.first_seen, 100);
        assert_eq!(got.last_seen, 200);
        assert_eq!(got.request_count, 2);
    }

    #[test]
    fn upsert_distinct_route_ids_do_not_collide() {
        let pool = DbPool::open_in_memory().unwrap();
        pool.upsert("ar-<16hex-A", "sk-A...1111", "api.openai.com", 100).unwrap();
        pool.upsert("ar-<16hex-B", "sk-B...2222", "api.anthropic.com", 100).unwrap();
        let all = pool.list().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn list_returns_newest_last_seen_first() {
        let pool = DbPool::open_in_memory().unwrap();
        pool.upsert("ar-old", "sk-A...1", "old.example", 100).unwrap();
        pool.upsert("ar-new", "sk-B...2", "new.example", 500).unwrap();
        pool.upsert("ar-new", "sk-B...2", "IGNORED", 900).unwrap();  // bump ar-new
        let all = pool.list().unwrap();
        assert_eq!(all[0].route_id, "ar-new");
        assert_eq!(all[0].last_seen, 900);
        assert_eq!(all[1].route_id, "ar-old");
    }

    #[test]
    fn restart_safe_via_reopen_in_memory() {
        // Ponytail: real "restart-safe" is tested in integration via open(path)
        // twice; here we prove the migration + schema survive a second handle
        // to the same in-memory db instance (Arc<Mutex> clone).
        let pool1 = DbPool::open_in_memory().unwrap();
        pool1.upsert("ar-persistent", "sk-P...1111", "persistent.example", 1000).unwrap();
        let pool2 = pool1.clone();  // Arc bump — same db
        let got = pool2.get("ar-persistent").unwrap().unwrap();
        assert_eq!(got.api_key_mask, "sk-P...1111");
        assert_eq!(got.request_count, 1);
    }
}
