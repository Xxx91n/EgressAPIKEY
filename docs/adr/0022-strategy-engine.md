# ADR-0022: Shell-Side Strategy Engine (A-class + B-class)

Status: ACCEPTED
**Date**: 2026-08-09

## Context

User requires two categories of egress strategies:
- A-class (IP enters platform): manual + auto (region, quality, subscription), liveness gate mandatory
- B-class (port selects exit IP): single=fixed, multi=random/round_robin/low_latency
- Both must be whitebox (config file + GUI)

Resin v1.2.0 has allocation_policy (3 values) + region_filters/regex_filters (static).
Active probing + circuit breaker already handles liveness gate.

## Decision

New module `crates/resin-core/src/strategy_engine.rs` (~600 lines):
- A-class: poll /nodes -> filter by strategy -> PATCH /platforms region_filters/regex_filters
- B-class: subscribe leases -> bias selection (OsRng random, counter round-robin, EWMA low-latency)
- Whitebox: egressapikey-strategy.json (hotswap-config atomic backup)
- No fork of Resin

## Consequences
- strategy_engine.rs is the shell's differentiated value
- Separate whitebox file for strategy config (SoC from port mapping)
- GUI needs strategy selection panel per platform