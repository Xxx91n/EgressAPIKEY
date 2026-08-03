# ADR-0011: Observed Key Pool — B-B-3 route_id-derived key identity + SQLite store

Date: 2026-08-03
Status: ACCEPTED
Decision Type: Storage architecture + GUI identity model

## Context

Grill Q10–Q12 resolved three linked decisions about how the desktop shell
identifies and stores per-(api key, upstream endpoint) traffic tuples while
the Resin Go sidecar owns the egress-IP-sticky lease layer.

1. **Q10/A10 = B-B-3 (originalist)**: B-column boxes on the Topology canvas
   are keyed by `route_id(normalize_auth(key), body.model, path)` — the same
   FxHash three-tuple the P24-A4-3 axum interceptor injects as
   `X-Resin-Account: ar-<16hex>`. This dogfoods the interceptor's own
   identifier into the GUI so the user sees the same "unique data stream"
   the shell uses for routing. Closest to the user's stated "millisecond-
   level unique identification of every (api key + upstream endpoint)
   stream" intent; heaviest implementation.

2. **Q11/A11 = (c) SQLite observed_keys table**: The reverse-map
   (route_id -> readable tuple) must outlive the process. Three options
   were weighed (in-memory HashMap, settings.json array, SQLite table).
   SQLite wins on engineering-ground: WAL-mode concurrent reads, query
   index, append-mostly happy path, restart-safe, and future VPS
   headless parity reuses the same DB. This is the project's **first
   real SQLite instance** (rusqlite was a Cargo dep but never
   instantiated per AGENTS §Storage locations).

3. **Q12/A12 = r2d2_rusqlite pool + hand-written user_version + reuse backup**:
   - Connection lifecycle: `r2d2` + `r2d2-rusqlite` pool. Engineering-grade
     canonical choice (pwm pro research, 2025): pooled connection reuse,
     serialized writes, blocking cost encapsulated inside connections,
     pool-size tuning + WAL play together. Mutex<Connection> rejected —
     serialization is unnecessary at this concurrency level and the pool
     path is more evolvable (write queue, monitoring).
   - **Implementation revision (2026-08-03)**: r2d2-rusqlite is no
     longer published on crates.io (404 for both "r2d2-rusqlite" and
     "r2d2_rusqlite"). The landed implementation falls back to
     parking_lot::Mutex<rusqlite::Connection> (the originally-rejected
     (a) alternative). The Decision text below retains the original
     Q12 answer for audit trail; the "Implementation Revision" section
     at the end of this ADR is the current source of truth.
   - Schema migration: **hand-written `PRAGMA user_version`** + `CREATE
     TABLE IF NOT EXISTS` idempotent startup check. pwm pro research
     rejected `rusqlite_migration` and `refinery` for *this* scale: one
     append-only table, low-churn schema, single-app. Adding an
     external migration crate costs more in dep + version lock +
     learning curve than it saves. user_version match branch grows
     manually when the schema evolves.
   - WAL + location + backup: `PRAGMA journal_mode=WAL` on, file at
     `app_config_dir()/ai-api-route.db` (beside settings.json), the
     `.db` + `.db-wal` + `.db-shm` triple is packed into the existing
     P14 backup zip (the schema already carries app_data; no new
     backup directory or flow).

## Decision

The shell maintains an `observed_keys` SQLite table keyed by
`route_id` (the ar-<16hex> injected toward Resin). The interceptor
writes new tuples to it on every request it sees (INSERT OR IGNORE —
route_id is deterministic so the same tuple never duplicates). The
Topology canvas reads via a new `observed_keys()` IPC and joins the
`LeaseEntry.account` (ar-<16hex>) back to a readable
`(apiKeyMask[0..4]+...+apiKeyMask[-4..], endpoint)` chip per platform
card. Unassigned route_ids render in a B-column "Unassigned" area
ready to be dragged onto a platform.

Schema v1:
```sql
CREATE TABLE IF NOT EXISTS observed_keys (
  route_id        TEXT PRIMARY KEY,
  apiKeyMask      TEXT NOT NULL,
  endpoint        TEXT NOT NULL,
  first_seen      INTEGER NOT NULL,
  last_seen       INTEGER NOT NULL,
  request_count   INTEGER DEFAULT 1
);
PRAGMA user_version = 1;
```

Cargo adds `r2d2` + `r2d2-rusqlite`. No `rusqlite_migration` /
`refinery` dep. A `DbPool(Arc<Pool<SqliteConnectionManager>>)` lives in
Tauri State; `open_db()` does WAL pragma + user_version match migration
+ pool init in one shot, called once from `main.rs .setup()` before
`manage(DbPool)`. The axum interceptor cfg holds an `Arc<DbPool>`
clone for `observed_key_upsert` calls inside `proxy_handler`.
   **Note**: the actual DbPool in crates/resin-core/src/db.rs is
   Arc<parking_lot::Mutex<Connection>>, not Arc<Pool<...>> -- see
   "Implementation Revision" at the end of this document.

## Consequences

- **First SQLite instance lands.** AGENTS §Storage locations must be
  updated from "Database: none yet" to "Database: observed_keys SQLite
  at app_config_dir()/ai-api-route.db (WAL). rusqlite through DbPool
  in main.rs setup. Schema migration = hand-written PRAGMA user_version."
- **GUI 不像玩具 (the user's core complaint)**: each Topology B-column
  box now carries an explicit `(key mask + endpoint)` identity derived
  from the same route_id the interceptor uses — the canvas reflects real
  routing state, not a stale Resin platform name.
- **Ponytail cost discipline**: r2d2 + WAL + backup reuse *are* the
  engineering layer; rusqlite_migration was the "too much dep" branch
  pwm and the user both rejected. The first SQLite landing is
  deliberately small (one table, hand-written migration) so future
  grills can extend the user_version match without a crate swap.
- **Future VPS parity**: the same observed_keys schema + DbPool is
  reusable on a headless path. If ADR-0009 ever activates, the DB
  layer is already in place; we don't redo storage twice.
- **Accepted trade-off**: first-screen "blank B column" is eliminated
  by SQLite persistence — the pool survives app restart, so the GUI
  opens with the last session's observed tuples already joined. Only
  the very first run on a fresh machine shows an empty canvas until
  one request flows through the interceptor.

## Closed-loop tests required at execution time

- Cargo: `INSERT OR IGNORE` idempotency, `SELECT` reverse-lookup hit,
  schema migration idempotency across consecutive `open_db` calls,
  restart-safe (tempfile reopen reads prior row).
- Vitest: TopologyView mock `observed_keys` IPC return + `lease_map`
  join -> box renders mask+endpoint not raw hash.
- The existing `route_id` + `normalize_auth` cargo tests (lane.rs)
  remain the alg side of the contract; this ADR adds the *storage*
  side closure.

## Implementation Revision (2026-08-03)

**Status**: supersedes the connection-lifecycle portion of the Q12/A12
decision above. The r2d2-rusqlite pool was the *planned* path; the
*landed* path is parking_lot::Mutex<rusqlite::Connection>.

**Root cause**: during C1-1 execution the r2d2-rusqlite crate was
found to be no longer published on crates.io (verified 2026-08-03:
crates.io returns 404 for both "r2d2-rusqlite" and "r2d2_rusqlite").
The crate was removed from the registry, so adding it as a dependency
would break the build. The fallback is the stdlib-adjacent path that
ADR-0011 originally listed as the rejected (a) alternative.

**Landed implementation** (crates/resin-core/src/db.rs):
- pub struct DbPool(Arc<parking_lot::Mutex<Connection>>) keyed off
  parking_lot::Mutex (not std::sync::Mutex -- parking_lot is a
  no-dependency fast mutex already in the workspace via
  crates/resin-core/Cargo.toml).
- Every async call site wraps the lock acquisition in
  tokio::task::spawn_blocking so the axum event loop is never
  blocked on the SQLite lock. The lock is held only for the duration
  of the SQL statement.
- open_db(path) does WAL pragma + PRAGMA user_version match
  migration + Connection init in one shot, called once from
  main.rs .setup() before app.manage(DbPool).
- The axum interceptor cfg holds an Arc<DbPool> clone for
  observed_key_upsert calls inside proxy_handler.

**Why the downgrade is safe at this concurrency level**:
- One interceptor write per *new* (key, endpoint, model, path) tuple
  -- route_id is deterministic FxHash, so INSERT OR IGNORE is a
  no-op on repeats. Write rate is bounded by distinct-tuple arrival,
  not request rate.
- One GUI SELECT * FROM observed_keys every 5s.
- SQLite serialised write throughput >> this arrival rate.
- The Mutex serialisation cost is invisible; a pool would only matter
  at >100 concurrent writers, which this single-user desktop shell
  will never see.

**Ponytail rationale**: the stdlib-adjacent path (zero new crates,
rusqlite 0.32 already in Cargo.toml) is the ladder rung-3 "stdlib
does it" path. The originally-rejected (a) alternative turned out
to be the only viable path once r2d2-rusqlite disappeared from
crates.io. The Q12/A12 (1)=b answer is preserved above for audit
trail; this section is the current source of truth for what the
code does.

**Future migration hook**: if r2d2-rusqlite is ever re-published or
a maintained fork appears, or if concurrency rises (multi-window,
VPS headless parity per ADR-0009), the swap from Mutex<Connection>
to Pool<SqliteConnectionManager> is local to db.rs -- the DbPool
public API (open_db, list, upsert) is unchanged.
