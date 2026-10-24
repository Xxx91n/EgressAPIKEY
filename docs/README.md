# docs/ — Documentation Directory

## Structure

| Directory | Purpose | Diataxis quadrant |
|---|---|---|
| `architecture/` | System architecture, design decisions | Explanation |
| `how-to/` | Operational guides (deployment, release, runbook) | How-to |
| `reference/` | Glossary, API references | Reference |
| `research/` | Research findings, protocol analysis | Explanation |
| `RESIN_UPSTREAM_MANIFEST.yaml` | Resin upstream version manifest | Reference (top-level) |

## Conventions

- All migrations use `git mv` (preserves history, R100 = content unchanged).
- Constraint docs stay at `docs/` top level. Retired docs are deleted, not archived in-tree — history lives in git.
- Session-local governance working files are never committed (there is no in-tree scratch directory).
