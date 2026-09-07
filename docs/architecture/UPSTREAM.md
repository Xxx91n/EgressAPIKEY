# Upstream Integration

> Read this first whenever your task touches the Resin upstream: wiring an endpoint, upgrading the sidecar, redistributing artifacts, or asking why something is not wired. This page is a **router** — it points at the document that owns your question and never repeats their content.

## The upstream relationship in one paragraph

EgressAPIKEY is a Tauri 2 + React 19 desktop shell over a **vendored Resin Go sidecar** (version pinned in the upstream manifest). The shell and the sidecar are independent works interacting **only** over a loopback REST seam (`ResinClient`, `crates/resin-core/src/resin_client.rs`): no FFI, no in-process linking, no embedding in either direction (mere aggregation, ADR-0067 D3).

## The four documents

| Document | What it owns | Read it when |
| --- | --- | --- |
| [RESIN_API_COVERAGE.md](RESIN_API_COVERAGE.md) | One row per upstream REST route (58 rows: 1 unauthenticated `/healthz` + 57 authenticated) with five-bucket wiring status, legislated by [ADR-0062](../adr/0062-resin-api-coverage.md) | Wiring, auditing, or documenting any Resin endpoint; asking "why is X not wired?" |
| [RESIN_UPSTREAM_MANIFEST.yaml](../RESIN_UPSTREAM_MANIFEST.yaml) | The single source of truth for the bundled Resin version: release assets, compat notes, fetch behavior (ADR-0017) | Upgrading Resin, checking the bundled version, understanding `scripts/fetch_resin.sh` |
| [THIRD_PARTY.md](../../THIRD_PARTY.md) | The redistribution-obligations registry: the two-layer license value per component (declared vs dependency-tree) | Shipping, redistributing, or answering license questions |
| [ADR-0067](../adr/0067-license-layering-provenance.md) | The license-layering decision: shell GPL-3.0-or-later, sidecar two-layer value, mere-aggregation invariant (D1-D5) | Changing the aggregation boundary, licensing, or contribution licensing |

## Decision routing

| Your question | Go to |
| --- | --- |
| Which endpoint do I call? Is X wired? Why is Y blank? | [RESIN_API_COVERAGE.md](RESIN_API_COVERAGE.md) first; deliberate non-adoptions carry written reasons in ADR-0062 D3 |
| What Resin version is bundled? How do I upgrade it? | [RESIN_UPSTREAM_MANIFEST.yaml](../RESIN_UPSTREAM_MANIFEST.yaml) + [ADR-0017](../adr/0017-upstream-resin-version-manifest.md): upgrades are manual (manifest bump + test + breaking-change notes) and the ADR-0017 amendment forces a THIRD_PARTY.md re-scan in the same commit |
| Can I ship this binary? What must a redistribution include? | [THIRD_PARTY.md](../../THIRD_PARTY.md) obligation table + [ADR-0067](../adr/0067-license-layering-provenance.md) D2 |
| Can I link the sidecar in-process or embed its source? | No — [ADR-0067](../adr/0067-license-layering-provenance.md) D3: loopback REST seam only; the ADR must be revisited before any in-process coupling |
| What changed in a release? | [RELEASE_NOTES.md](../RELEASE_NOTES.md) (per-release notes with compatibility + upgrade notes), derived from the [CHANGELOG](../../CHANGELOG.md) |

## Rules this router locks

- The bundled Resin version lives **only** in `docs/RESIN_UPSTREAM_MANIFEST.yaml`; never hardcode a version elsewhere.
- Any claim about a Resin endpoint must check its coverage row first (ADR-0062 D5: the row and the governing ADR update in the same commit as the change).
- Direct reads of Resin's private `request_logs*.db` files are prohibited; the only sanctioned direct read is `backup_create` over `state.db`/`cache.db` (read-only, zip packaging).
- Machine gates: `scripts/upstream-router-check.cjs` verifies this page's link integrity and the RELEASE_NOTES ↔ CHANGELOG version alignment; it is mounted in `scripts/verify-build.sh`.
