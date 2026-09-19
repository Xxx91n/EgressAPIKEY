# ADR-0072: CI-only build transition - local build mandates retired

Status: ACCEPTED (2026-09-19, round11-grill decision D-006#1)
> Extends (reopens none): the 2026-09-04 user mandate "CI-only build policy" (which this ADR records into the decision ledger); ADR-0019 (build/release standardization - the build-all scripts become pipeline reference). Supersedes the "Hard close-loop (verified P23)" clause (AGENTS.md section 5) and the "Build is part of the commit (verified P23)" clause (AGENTS.md section 6).
> Research basis: atomcode (source=atomcode-q6-governance) - agents.md official FAQ ("explicit user chat prompts override everything"; treat the manual as living documentation), OpenAI Codex AGENTS.md override guidance; the arbitration pattern is revise-the-manual, never let each agent pick a side.

## Context

Two iron laws coexisted and physically contradicted. AGENTS.md section 5 ("Hard close-loop, verified P23") mandated that every source commit yield a freshly-staged exe from a LOCAL build with chunk-hash verification; the 2026-09-04 user mandate ("CI-only build policy") forbids ALL local builds, tests and packaging, accepting only CI runs as build evidence. Any agent landing on the repository had to violate one of them on day one. The two clauses real intent - verifiable, fresh delivery artifacts - was never in conflict; only their execution surface was.

## Decision

D1 - Local build mandates retired. The P23 hard close-loop and build-is-part-of-the-commit clauses are deleted from AGENTS.md; local builds, tests and packaging remain forbidden.
D2 - Delivery evidence re-anchored to CI: the verify job (verify-build.sh) runs on every push; a release-matrix dispatch produces the user-facing portable exe when a testable binary is needed. The chunk-hash freshness check and the MainWindowTitle smoke check apply to CI-built artifacts only.
D3 - scripts/build-all.sh and scripts/build-all.ps1 remain as the canonical pipeline reference (tsc -b && vite build -> cargo -> stage, with the T10-audit STALE BUNDLE hash guard) but must not be executed locally.
D4 - The stale-bundle lesson (P22) is preserved in intent: any pipeline producing a deliverable exe must build the frontend before cargo and fail fast on chunk-hash mismatch. Residual: if a CI release job does not yet carry the chunk-hash check, adding it is the R11-01 residual item.

## Consequences

- Agents read one consistent rule: CI builds; agents verify through CI evidence and dispatch, never local builds.
- Historical notes referencing local builds (P19 MinGW resource fix, P2 debug-vs-release caveat) remain as history only.
- AGENTS.md byte size shrinks slightly; the file remains above the 32 KiB doc budget (pre-existing condition, separate cleanup, out of scope here).
