# ADR-0050: Shrink resin-core public surface to the shell-used set

**Status**: Accepted
**Date**: 2026-08-30
**Related**: ADR-0009 (stub bin, partially superseded), ADR-0024 (SharedGateway deletion precedent), ADR-0036 (whitebox single write entry), ADR-0043 (headless build separation), ADR-0045 (IPC error contract)

## Context

architecture-recovery ticket 04 required verifying which "original kernel" symbols in crates/resin-core are still referenced outside the crate, then deleting the unreferenced ones so the crate public surface stops lying about what the product uses.

Build facts (2026-08-30, CodeGraph 87 files/1454 nodes + workspace-wide symbol census + baseline builds):

- The real headless product binary is the src-tauri egressapikey-headless bin (required-features = ["headless"], entry src-tauri/src/headless_main.rs, ADR-0043). It imports zero resin-core kernel symbols. CI builds it (.github/workflows/ci.yml:100); the resin-core stub bin is in no CI target, no release artifact, and no script deliverable.
- crates/resin-core/src/bin/resin_core.rs is the honest stub ADR-0009 created. Its only consumer of crate API is CoreConfig; its 7 load_config callers are all inside the bin itself (5 of them tests).
- MihomoConfig/MihomoController (mihomo.rs), GatewayState (gateway.rs), lane_index/LaneConfig (lane.rs), LeaseId/LeaseTable (lease.rs), TdEwma (tdewma.rs) have zero references outside resin-core, and inside the crate they are only referenced by each other plus tests that would be deleted together.
- The GUI shell consumes a different, live set: DbPool, IpcError/map_resin_error, PortForwarder, PortMapping, WhiteboxConfig(Store)/WHITEBOX_CONFIG_FILE, NetworkConfig, StreamSensorSnapshot, parse_trace_body_ip, parse_public_ips, ReputationClient/Provider/Snapshot, ResinClient, MAX_LANES, MIN_USER_PORT, MAX_ENTRY_PORTS, enabled_entries_for_restore, and platform::PlatformRegistry (SharedRegistry, managed in src-tauri/src/main.rs).
- The gateway data path runs in the Resin Go sidecar since ADR-0012/0015; ADR-0024 already deleted the shell-side SharedGateway dead code.
- Baseline: all cargo build feature combos green; workspace-wide cargo test was already red before this change (duplicate map_resin_error_* test names + type mismatches in src-tauri/src/commands/mod.rs:3174-3300, ticket 06 scope).

## Decision

Delete from resin-core (all zero-external-reference):

1. The stub bin src/bin/resin_core.rs and its [[bin]] target. This supersedes the ADR-0009 decision to keep the stub: the headless product surface is egressapikey-headless (ADR-0043), and the stub plus CoreConfig is a self-referential face no other crate consumes. A future VPS-parity phase restarts from egressapikey-headless, not from this stub.
2. CoreConfig, sanitize_lanes, MIN_LANES, DEFAULT_LANES (all only used by the stub bin and the deleted lane module).
3. The whole modules mihomo.rs, gateway.rs, lane.rs, lease.rs, tdewma.rs.
4. Rewrite tests/integration.rs to cover the surviving Platform/Account registry composition the shell actually uses (SharedRegistry).
5. Prune crates/resin-core/Cargo.toml: remove axum, hyper, bytes, http, thiserror (zero src use even before this ADR), clap and tracing-subscriber (stub-bin only), fxhash (lane.rs only), and the unused dev-dependencies tower and http-body-util. mockito stays (used by resin_client.rs unit tests).
6. Keep MAX_LANES as the IPC lane-range ceiling (AGENTS.md section 7.5 contract; used by src-tauri commands) and keep platform.rs in full.

AGENTS.md sections 7.5/7.6 and docs/architecture/ARCHITECTURE.md are updated in the same commit. The mihomo controller SSRF guard (Re8) disappears with the module; if mihomo REST control is ever reintroduced, the constructor MUST re-assert loopback-only validation before any IPC wiring.

## Consequences

- cargo build -p resin-core, -p egressapikey-app (default, --features headless, --features custom-protocol) all green after the change; cargo test -p resin-core: 105 lib + 3 integration + 4 proxy_e2e (1 ignored) pass.
- The compile-guard scripts (scripts/build-all.sh, build-all.ps1, verify-build.sh) keep working unchanged: cargo build -p resin-core builds the lib-only crate.
- resin-core no longer ships an axum/hyper dependency tree; the crate is now honestly described as shell-side support.
- The workspace-wide cargo test red described above predates this ADR and belongs to ticket 06 (merge-error-mapping).
