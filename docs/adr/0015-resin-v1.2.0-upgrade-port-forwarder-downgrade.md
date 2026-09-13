# ADR-0015: Resin v1.2.0 Upgrade — port_forwarder Thin-Shell Downgrade

Status: ACCEPTED
> Date: 2026-08-06
> Supersedes: None (complements ADR-0012)
> Decides: T1 — port_forwarder downgrades to endpoint API forwarder; StreamSensor stays shell-side

## Context

ADR-0012 chose thin-shell multi-port forwarder (port = identity). P2 implemented
`port_forwarder.rs` (791 lines) with hand-rolled SOCKS5/HTTP protocol handling
because Resin v1.1.2's endpoint management code returned 404 in the release binary.

Resin v1.2.0 (released 2026-08-01) ships the endpoint management feature live:
- `GET /api/v1/endpoints` — list all inbound endpoints (default + custom)
- `POST /api/v1/endpoints` — create + immediately start a custom listener
- `GET /api/v1/endpoints/{id}` — read one
- `PATCH /api/v1/endpoints/{id}` — update port or capabilities (hot-reload)
- `DELETE /api/v1/endpoints/{id}` — delete + close listener

Endpoint schema: `{id, port, allow_management, allow_proxy, allow_http_forward,
allow_http_reverse, allow_socks5, source, read_only, status, last_error}`

`LEGACY_V0` removed; `RESIN_AUTH_VERSION` empty defaults to `V1`; project already
sets `V1` — no impact.

### Overlap Analysis (source-level)

| Dimension | port_forwarder.rs (P2) | Resin v1.2.0 endpoint API |
|-----------|------------------------|---------------------------|
| Port listener | Tauri Rust `TcpListener::bind` | Resin Go sidecar native |
| SOCKS5 | Hand-rolled minimal CONNECT (269-403) | Production full SOCKS5 + auth |
| HTTP proxy | Hand-rolled CONNECT + absolute-form (404-458) | Full HTTP routing layer |
| Connection lifecycle | Shell process dies → all ports die | "close hijacked connections on shutdown" |
| Port CRUD persistence | Shell SQLite `port_mappings` | Resin `state.db` `endpoints` table |
| **StreamSensor AI flow** | **Shell-only** (SSE/WS/unary classification) | Resin does NOT inspect AI flow content |

**70% overlap** (listener + protocol + CRUD). **30% shell-unique** (StreamSensor).

## Decision

**T1 (Thin-Shell Downgrade):**

1. Upgrade `fetch_resin` REL from `v1.1.2` → `v1.2.0`; re-fetch sidecar binary.
2. `port_forwarder.rs` deletes: `TcpListener::bind`, `handle_socks5`, `handle_http`,
   `handle_client`, `read_socks_addr`, `socks5_connect_authed`, `spawn_port`,
   `stop_port`, `reload` — all protocol/listener code (~500 lines).
3. `port_forwarder.rs` keeps: `PortForwarder` struct (now holds `ResinClient` +
   `StreamSensor`), `stream_snapshot()`, `running_ports()` (now queries endpoint API),
   `resin_identity()`, `MAX_ENTRY_PORTS`, `MIN_USER_PORT`, `detect_protocol()`.
4. `ResinClient` gains 5 endpoint methods: `list_endpoints`, `create_endpoint`,
   `update_endpoint`, `delete_endpoint`, `get_endpoint`.
5. 5 IPC commands (`port_list/upsert/remove/running/reload`) convert from
   shell-DB ops to Resin endpoint API forwarders — same pattern as `platform_add`
   in G2 phase 2.
6. Shell SQLite `port_mappings` table **stays** as port→platform binding metadata
   (Resin endpoint schema has no `platform_name` field; shell owns business mapping).
7. StreamSensor wiring unchanged: `observe_headers(accept, upgrade, content_type)`
   called from any place that sees plaintext HTTP headers — independent of listener.

### Why Not Delete port_forwarder.rs Entirely?

StreamSensor is the shell's unique value. Resin cannot do AI flow classification.
Keeping a thin `PortForwarder` struct that holds `StreamSensor` + delegates CRUD to
Resin is the minimum viable surface. Deleting the file would orphan StreamSensor.

### Why Not Keep Full port_forwarder.rs?

Hand-rolled SOCKS5/HTTP is a minimal subset ("hand-rolled minimal" — file's own
ponytail note). Production edge cases (SOCKS5 BIND, UDP ASSOCIATE, HTTP chunked
encoding, CONNECT timeout) are real risks. Resin's 100k-node-verified protocol stack
owns this.

## Consequences

- `cargo test -p resin-core --lib` gains 5 endpoint mockito tests; loses ~5
  port_forwarder protocol unit tests (net zero or slight gain).
- Frontend `PortMapping` type in `src/lib/ipc.ts` aligns with endpoint schema
  (adds `allow_management`, `allow_http_reverse`, `status`, `last_error` fields).
- `port_forwarder.rs` shrinks from 791 → ~250 lines.
- Release exe must be rebuilt after this change (AGENTS §5 hard close-loop).
- No IPC contract break: `port_list/upsert/remove/running/reload` signatures
  stay the same; only the internal implementation changes from shell-DB to
  Resin API forwarder.
