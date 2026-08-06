# ADR-0017: Upstream Resin version manifest (YAML, not auto-fetch)

Date: 2026-08-06
Status: ACCEPTED
Context: fetch_resin.{ps1,sh} hardcoded REL=v1.2.0; AGENTS.md has 11 stale v1.1.2 refs; no compat doc.

## Decision

Create docs/RESIN_UPSTREAM_MANIFEST.yaml as the single source of truth
for the Resin sidecar version. Fields: version, sha256 (per-platform),
release_url, repo_url, api_version, breaking_changes, compat_notes.

fetch_resin.{ps1,sh} read version from this YAML instead of hardcoding
REL=... in the script. AGENTS.md stale v1.1.2 references updated to the
manifest version.

Do NOT auto-fetch latest release tag. Each upgrade is a manual manifest
update + test run + breaking-change documentation. This prevents tracking
into an incompatible upstream release.

## Rationale

Resin v1.1 to v1.2 already had breaking API changes (endpoint API added,
port_forwarder downgraded per ADR-0015). Auto-following upstream without
testing would break the shell. clash-verge-rev can auto-fetch because
mihomo API is very stable; Resin is younger and less stable.

pwm research recommended VERSION + COMPAT.md as the lightweight pattern
for Tauri sidecar bundling. YAML manifest is a single-file version of that.

## Alternatives rejected

- clash-verge-rev auto-fetch latest tag: risky for Resin (breaking changes
  between minor versions). Would need compat gate anyway.
- SPDK ABI tracker / kernel-abi-tracker: far too heavy for one binary.

## Tradeoffs

- Manual upgrade process (edit YAML + test + commit)
- Full control over when to upgrade and what to document
- Single file = easy to find, easy to diff between upgrades
