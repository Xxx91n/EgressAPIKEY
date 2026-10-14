# docs/ — Documentation Directory

## Structure

| Directory | Purpose | Diataxis quadrant |
|---|---|---|
| `architecture/` | System architecture, design decisions | Explanation |
| `adr/` | Architecture Decision Records (55 ADRs, MADR format; numbering 0001–0080 with gaps — 26 bookkeeping records retired 2026-10-15, recoverable via git history; `0050` exists twice as `0050` + `0050-bis`) | Reference (decisions) |
| `how-to/` | Operational guides (deployment, release, runbook) | How-to |
| `reference/` | Glossary, API references | Reference |
| `research/` | Research findings, protocol analysis | Explanation |
| `PONYTAIL_DEBT_LEDGER.md` | Active ponytail debt tracking | Constraint (top-level) |
| `RESIN_UPSTREAM_MANIFEST.yaml` | Resin upstream version manifest | Reference (top-level) |

## Conventions

- All migrations use `git mv` (preserves history, R100 = content unchanged).
- Constraint docs stay at `docs/` top level. Retired docs are deleted, not archived in-tree — history lives in git.
- `.scratch/` holds session-local governance working files (gitignored).
