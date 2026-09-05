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

## Amendment (2026-09-05, architecture-recovery ticket 01 / ADR-0067 D2): dependency-tree license scan

Every version bump in this manifest MUST also re-run the dependency-tree
license scan for the sidecar and update `THIRD_PARTY.md` in the same commit.
The declared upstream license and the dependency-tree truth are two
different facts (ADR-0067 D1/D2): Resin v1.2.0 declares MIT, but its
dependency tree carries GPL-3.0-or-later obligations via
`github.com/sagernet/sing-box` (pinned in `resin/go.mod`; sing-box's own
LICENSE is a GNU GPLv3-or-later application notice) — hence the manifest's
`license` field records the two-layer value, not a bare SPDX string.

Concretely, on bump: quote the upstream `go.mod` and `LICENSE` originals at
the exact new tag URL (never from memory), diff the resulting layer value
against `THIRD_PARTY.md`'s registry row, and if declared vs
dependency-tree evidence disagree, the bump is blocked until the registry
and the manifest fields agree. This closes the "declared MIT, ships GPL"
mismatch class at the one place versions change.
