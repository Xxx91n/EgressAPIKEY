# ADR-0014: Dead code deletion — interceptor/route_id/observed_keys line

Date: 2026-08-05
Status: ACCEPTED
Decision Type: Code hygiene (Ponytail clean break)

## Context

Q10 of the grill confirmed: the interceptor-based identity line is built on
a wrong premise (HTTPS header visibility). ADR-0012 supersedes the
architecture. The user chose (a) delete all dead code — Ponytail clean break.

## Decision

Delete the following code and tests:

1. crates/resin-core/src/interceptor.rs — entire file (axum proxy_handler,
   InterceptorConfig, app(), serve(), all unit tests)
2. crates/resin-core/src/lane.rs — route_id() and normalize_auth() functions
   + their 8 cargo tests (the lane hash-slot concept itself is retained for
   IPC backward compat, but the FxHash three-tuple identity is deleted)
3. crates/resin-core/src/db.rs — observed_keys table schema + upsert + list
   (DbPool struct + open_db + WAL pragma + parking_lot::Mutex infra KEPT
   for reuse as port->platform mapping store)
4. crates/resin-core/tests/a4_3_live.rs — entire file (interceptor e2e test)
5. src-tauri/src/commands/mod.rs — observed_keys IPC command + ObservedKey
   struct + LeaseEntry struct (lease_map command stays, re-targeted to
   port-based account resolution)
6. src/views/TopologyView.tsx — route_id chip rendering in B column
   (replaced by port-based identity rendering)
7. src/views/PlatformsView.tsx — keyCandidates left pane
   (replaced by entry-ports pane)

## What is KEPT

- DbPool(Arc<parking_lot::Mutex<Connection>>) infrastructure — reused for
  port->platform mapping table
- ResinClient (create_platform, list_platforms, active_leases,
  node_pool_snapshot, subscription CRUD, platform_update) — all stay
- All IPC commands that forward to Resin (platform_add/remove/list/update,
  subscription_add/remove/list, node_list, lease_map) — stay
- Ghost safety net, sidecar lifecycle, tray, settings — stay

## ADRs superseded

- ADR-0003 (key+endpoint identification via header) — SUPERSEDED by ADR-0012
- ADR-0011 (B-B-3 route_id-derived key identity + SQLite store) — SUPERSEDED
  by ADR-0012. The SQLite infra is reused; the observed_keys schema is not.
