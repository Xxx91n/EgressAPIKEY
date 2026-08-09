# ADR-0026: accountId-encoded B-class strategy + IpcError contract + trace skeleton

Date: 2026-08-10
Status: ACCEPTED
Supersedes: None (complements ADR-0012, ADR-0014, ADR-0022)

## Context

Grill T5 round (2026-08-10) produced 10 decisions across three axes:
1. Enterprise trace infrastructure (Q1-Q5): trace_id passthrough, double-layer spans
2. IPC error contract (Q6-Q8): IpcError enum with i18n_key, 44 command refactor
3. B-class strategy alignment (Q9-Q10): protocol-aware prober + accountId encoding

The B-class strategy catalog (strategy.rs StrategyId, 6 variants) maps to Resin's
3-value allocation_policy. Scheme c was chosen: the shell encodes B-class strategy
information into the Resin account string, leveraging Resin's per-account lease
isolation to achieve different exit IP selection behavior per strategy.

Resin's identity_v1.go parseV1PlatformAccountIdentity treats account as an opaque
string. The routing router.go RouteRequest uses account only for lease lookup
(account="" means random, else sticky). Resin does not interpret account content
for node selection -- it only uses it as a lease key. This enables scheme c:
the shell can encode strategy metadata into account without Resin needing to
understand it.

ADR-0014 deleted interceptor.rs, route_id, observed_keys -- scheme c does NOT
revive these. It uses the port-based identity path from ADR-0012: the shell's
port-to-platform mapping assigns account strings; no HTTP header parsing.

## Decision

### Part 1: Trace Skeleton (Q1-Q5)
- Frontend: invokeWithTrace(cmd, args) wrapper in src/lib/ipc.ts injects
  crypto.randomUUID() as __trace_id into every invoke() args object.
- Backend: extract_trace_id() helper in src-tauri/src/trace.rs, trace_ipc_span()
  creates tracing::info_span! per command. ResinClient methods use #[instrument].
- No new crates (tracing + uuid already in deps).

### Part 2: IpcError Contract (Q6-Q8)
- New IpcError enum in crates/resin-core/src/ipc_error.rs with 4 variants:
  BindConflict(port), InvalidStrategy(value,accepted), ResinUpstream(status,excerpt),
  Internal(msg). Each carries an i18n_key string + template params.
- Externally-tagged serde (serde(tag="kind", content="data")).
- map_resin_error() maps Resin HTTP responses to IpcError variants.
- All 44 #[tauri::command] return Result<T, IpcError> instead of Result<T, String>.
- Frontend extractIpcErr() + discriminated union + i18n key routing.

### Part 3: B-Class Strategy Scheme C (Q10)
- The shell encodes B-class strategy into the account string used with Resin.
- Account format: port_label + "::" + strategy_tag where strategy_tag is one of:
  - random -> Resin allocation_policy BALANCED (random node, per-account lease)
  - rr-N -> shell rotates N accounts per request (round-robin across leases)
  - latency -> Resin PREFER_LOW_LATENCY
  - fixed -> single account, single lease, no rotation
- strategy_engine.rs owns the account_for_bclass(policy, port_label) function.
- GUI B-class selector sends the strategy tag; port creation embeds the account.
- Resin treats account as opaque; the shell owns strategy semantics.
- NO interceptor, NO route_id, NO header parsing (ADR-0014 respected).

### Part 4: Bug Fixes (Bug 1-4)
- Bug 1: build-all.ps1 exact match Name -eq "egressapikey-headless.exe"
- Bug 2: PlatformsView b_class default "random" (not "balanced")
- Bug 3: IpcError::BindConflict (covered by Part 2)
- Bug 4: port_health_check protocol-aware (SOCKS5 greeting / HTTP CONNECT / no-auth)

## Consequences

- Trace skeleton adds ~50 lines Rust + ~20 lines TS per command, zero new crates.
- IpcError refactor touches all 44 commands (~150 lines Rust + ~50 lines TS).
- B-class scheme c adds ~100 lines in strategy_engine.rs + ~30 lines GUI.
- 23 new tests (14 cargo + 9 vitest) covering every error path.
- Resin stays un-forked; every upstream release can be re-vendored.
- The account string is the shell's private strategy channel into Resin's
  lease system -- future B-class strategies only need new strategy_tags.