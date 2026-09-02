# EgressAPIKEY Glossary

> **Superseded by [CONTEXT.md](../CONTEXT.md)** — kept here as the original term-history record; new terms are added to CONTEXT.md directly per ADR-0013.
>
> **Stale-reference warning**: this file is a historical record — entries
> may reference symbols or behaviors deleted since they were written
> (e.g. the platform_snapshot IPC, route_id/lane.rs, the A4-3 interceptor
> header injection; see ADR-0014 / ADR-0015 / ADR-0050 and
> architecture-recovery ticket 23). Never treat such a reference as
> current; CONTEXT.md and docs/adr/ are the as-built truth.
>
> Domain terms used across ADRs, AGENTS.md, and docs/. Maintained by the
> grill-with-docs / domain-modeling workflow. Add a term when an ADR or
> plan introduces a term that is not self-explanatory.

## closed-loop test

A test that proves a feature works end-to-end through the real code path it
claims to cover, not just a mocked stub. For backend IPC: cargo unit test +
mockito integration test. For GUI: vitest component test mocking invoke().
A feature without a closed-loop test is "black-box / toy" by the user is
standard (ADR-0003 context).

## live-sidecar e2e

A closed-loop test that runs against the REAL Resin Go sidecar binary
(resin-x86_64-pc-<abi>.exe), not a mockito stand-in. Proves the shell
injects headers the real binary actually honors, and the real binary
produces leases with the expected egress IPs. Stronger than mockito-only.
ADR-0006 item 3 requires this for the A4-3 interceptor.

## routable-view

> **Stale reference (historical)**: the `platform_snapshot` IPC mentioned
> below was deleted (architecture-recovery ticket 23). This entry is kept
> as term history only; current truth lives in CONTEXT.md.

Resin admin endpoint: GET /api/v1/platforms/{id}/routable-view. Returns the
list of nodes a platform can actually route to (after region_filters + node
health). The shell is platform_snapshot IPC currently returns an empty
Vec<Account>; wiring it to routable-view is ADR-0006 item 1 / HANDOFF
next-deep-work-1.

## route_id

> **Deleted symbols (historical)**: `route_id()` and `lane.rs` no longer
> exist — ADR-0014 deleted the interceptor identity line (route_id,
> normalize_auth, the A4-3 interceptor) and ADR-0050 deleted the remaining
> lane.rs module. This entry is kept as term history only.

A stable u64 hash of the three-tuple (normalized_auth, body.model,
request.path) computed by route_id() in crates/resin-core/src/lane.rs.
The shell injects X-Resin-Account: ar-<16hex of route_id> so Resin anchors
egress IP per unique (api key + upstream endpoint) pair. ADR-0003.

## X-Resin-Account

> **Superseded mechanism (historical)**: the A4-3 interceptor described
> below was deleted by ADR-0014, and ADR-0015 confirmed the thin-shell
> forwarder — the shell no longer injects this header per (key, model);
> port identity reaches Resin via the port→(platform, account) mapping
> (see CONTEXT.md「Entry Port Mapping」).

HTTP header Resin uses as the highest-priority account identity source
(source priority: X-Resin-Account > URL identity segment > fixed_header).
The shell (A4-3 interceptor) sets this from route_id; the value is an
opaque business account string from Resin is perspective, not the upstream
API key. Strip-then-inject: any inbound client-supplied X-Resin-Account is
removed before the route_id-derived value is injected.

## mainline A / mainline B

ADR-0006. Mainline A = function closure (make the software not a toy).
Mainline B = release standardization (fill release/ + tag v0.1.0).
Sequential: B starts only after A is closed-loop verified.

## observed_keys

The shell-side SQLite table (ADR-0011) storing the reverse map from
route_id (ar-<16hex>) to readable (apiKeyMask, endpoint, first_seen,
last_seen, request_count). Append-only. Backed up inside the P14 zip
alongside settings.json. Lives at app_config_dir()/egressapikey.db (WAL).

## DbPool

The Arc<r2d2::Pool<SqliteConnectionManager>> wrapper managed into Tauri
State from main.rs .setup(). Shared by the axum interceptor (writes
observed_keys on new route_id) and the observed_keys() IPC handler
(services the GUI 5s sync). r2d2_rusqlite per AGENTS ADR-0011 — pool
size 4, WAL enabled, hand-written PRAGMA user_version migration.
