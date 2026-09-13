//! Shell-side port->platform mapping store (ADR-0012).
//!
//! A tiny SQLite store mapping each Entry Port number to a (platform_name,
//! account_string) pair. The multi-port listener reads this on every inbound
//! connection to inject the correct X-Resin-Account header before forwarding
//! to Resin. Reuses the DbPool infrastructure from ADR-0011 (parking_lot::Mutex
//! + WAL + hand-rolled PRAGMA user_version migration).

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

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

#[derive(Clone)]
pub struct DbPool(Arc<Mutex<Connection>>);

impl DbPool {
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("open db: {e}"))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| format!("pragma journal_mode=WAL: {e}"))?;
        Self::migrate(&conn)?;
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("open in-memory: {e}"))?;
        Self::migrate(&conn)?;
        Ok(Self(Arc::new(Mutex::new(conn))))
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
            // Round 8 ticket 13 / D-007 (ADR-0068 D3): the entry-port protocol
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
        let conn = self.0.lock();
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
        let conn = self.0.lock();
        conn.execute(
            "INSERT INTO port_mappings (port, protocol, platform_name, account, label, enabled, auth_required) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(port) DO UPDATE SET protocol = excluded.protocol, platform_name = excluded.platform_name, account = excluded.account, label = excluded.label, enabled = excluded.enabled, auth_required = excluded.auth_required",
            params![m.port, m.protocol, m.platform_name, m.account, m.label, m.enabled as i64, m.auth_required as i64],
        )
        .map_err(|e| format!("upsert port_mappings: {e}"))?;
        Ok(())
    }

    pub fn delete_port(&self, port: u16) -> Result<(), String> {
        let conn = self.0.lock();
        conn.execute("DELETE FROM port_mappings WHERE port = ?1", params![port])
            .map_err(|e| format!("delete port_mappings: {e}"))?;
        Ok(())
    }

    /// Replace the complete port map in one SQLite transaction. The caller has
    /// already validated the desired configuration; a DB error leaves the old
    /// map untouched.
    pub fn replace_ports(&self, mappings: &[PortMapping]) -> Result<(), String> {
        let mut conn = self.0.lock();
        let tx = conn
            .transaction()
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
        let conn = self.0.lock();
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

    /// Round 8 ticket 13 / D-007: a v3-era table carried the pre-change
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
        let pool = DbPool(Arc::new(Mutex::new(conn)));
        let rows = pool.list_ports().unwrap();
        let protocol_of = |p: u16| {
            rows.iter()
                .find(|r| r.port == p)
                .expect("row must survive the table rebuild")
                .protocol
                .clone()
        };
        assert_eq!(protocol_of(17990), "mixed", "legacy socks5 is flag-preserving");
        assert_eq!(protocol_of(17991), "http", "http keeps its meaning");
        assert_eq!(protocol_of(17992), "mixed", "an already-mixed row is untouched");

        let ddl: String = pool
            .0
            .lock()
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'port_mappings'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(ddl.contains("DEFAULT 'mixed'"), "column default moved to mixed: {ddl}");
        assert!(!ddl.contains("DEFAULT 'socks5'"), "retired token must not survive: {ddl}");

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
}
