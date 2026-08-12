# ADR-0028: WhiteboxConfig network + DNS struct (T6-1)

Date: 2026-08-12
Status: ACCEPTED
Supersedes: None (complements ADR-0012 — port=identity leaves the Resin sidecar in charge of the data plane; this ADR defines how the shell tunes sidecar network behavior without forking Resin)

## Context

The T6 grill plan (docs/GRILL_T6_NETWORK_LAYER_PLAN.md) exposed a gap: the shell-side `WhiteboxConfig` only carried entry_ports and had no way to probe/control Resin's network-layer behavior. Resin v1.2.0 itself exposes a set of env-var knobs for DNS, connection pooling, probing, and bypass rules — but no shell input surface existed.

## Decision

Extend `crates/resin-core/src/whitebox_config.rs` with a `network: NetworkConfig` block carrying seven optional fields mirroring the Resin env-var surface:

1. `dns_upstreams: Vec<String>` — DoH failover chain (RESIN_NODE_DNS_UPSTREAMS)
2. `max_idle_conns: Option<u32>` — transport pool size (RESIN_PROXY_TRANSPORT_MAX_IDLE_CONNS)
3. `max_idle_conns_per_host: Option<u32>` — per-host pool cap (RESIN_PROXY_TRANSPORT_MAX_IDLE_CONNS_PER_HOST)
4. `idle_conn_timeout_secs: Option<u64>` — pool idle TTL (RESIN_PROXY_TRANSPORT_IDLE_CONN_TIMEOUT)
5. `probe_timeout_secs: Option<u64>` — node probe deadline (RESIN_PROBE_TIMEOUT)
6. `probe_concurrency: Option<u32>` — node fan-out (RESIN_PROBE_CONCURRENCY, 1..=10000)
7. `proxy_bypass: Vec<String>` — bypass rules (RESIN_PROXY_BYPASS)

`validate_network()` enforces non-empty entries, length caps (dns_upstreams 512, proxy_bypass 253), and the probe_concurrency 1..=10000 range — at the parsing boundary, NOT inside Resin.

`sidecar.rs` reads the block via `read_network_config(app_data)` and conditionally injects the seven env vars only when set, so default behavior (Resin v1.2.0 defaults) is preserved when the user has not edited the whitebox JSON.

## Consequences

- No Resin fork — reuses upstream releases unchanged.
- Whitebox JSON is the single user-facing surface for network tuning; the shell's Settings card (T6-3) writes it atomically with a "Reset to Default" button that clears the block.
- The Resin sidecar's defaults already ship domain-validated DoH (e.g. `https://doh.pub/dns-query`); users opt-in to override by editing the Settings card.
- validate_network runs before sidecar boot so a bad value surfaces as a parse error, not a runtime hang.

## Test closed-loop

- `crates/resin-core/src/whitebox_config.rs` ships 5 cargo unit tests for the validation path: empty dns entry, max_idle_conns=0, probe_concurrency out-of-range, valid multi-entry config, proxy_bypass length cap.
- `src-tauri/src/sidecar.rs` ships 3 cargo unit tests for the read helper: missing-file (returns default), parses-whitebox-json, corrupt-json (returns default).
- All tests pass under `cargo test -p resin-core --lib` (122 passed across the crate).

## Oversight

This ADR records only T6-1. Sidecar env injection (T6-2), GUI Settings card (T6-3), and the rest of the T6 plan are referenced in docs/GRILL_T6_NETWORK_LAYER_PLAN.md and docs/HANDOFF_T6_NETWORK_LAYER.md.
