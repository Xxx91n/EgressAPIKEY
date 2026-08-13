# ADR-0030: Cross-Platform Build/Compile/Test Compatibility

> **Status**: ACCEPTED
> **Date**: 2026-08-14
> **Supersedes**: None (complements ADR-0019)

## Context

The project has grown from Windows-only to a three-platform release matrix. However, the local build scripts have accumulated platform-specific hardcoding: build-all.sh has no test step, flow guard only works on Windows, fetch_resin.sh uses cwd-relative paths, CI portable job matrix.bin uses lowercase but Cargo.toml bin name is uppercase, no SHA256 checksums generated locally, and verify-build.sh runs cargo test --workspace which fails on Windows host.

## Decision

1. Keep build-all.ps1 as-is (Q1-C). Only fix build-all.sh for cross-platform.
2. build-all.sh gains test steps (Q2-A): cargo test -p resin-core --lib + vitest run + i18n:check.
3. verify-build.sh gets CI/local split (Q5-A): CI=true -> full workspace; local -> resin-core only.
4. Three-platform flow guard (Q3-A): Windows direct, macOS open, Linux with DISPLAY shield.
5. Backend tar.gz staged locally (Q4-A): bin name from Cargo.toml, not hardcoded.
6. Unified binary naming (Q6-A + Q9-A + Q10-A): EgressAPIKEY (GUI), egressapikey-headless (headless), resin-core (backend).
7. fetch_resin.sh SCRIPT_DIR anchoring (Q7-A): repo root resolved from script location, not cwd.
8. SHA256 checksums (Q8-A): generated for all staged artifacts.

## Consequences

- build-all.sh becomes the single cross-platform local build entry point (bash).
- build-all.ps1 remains Windows-only convenience but no longer the canonical path.
- CI portable job matrix.bin corrected to match Cargo.toml bin names.
- verify-build.sh no longer fails on Windows dev host.
