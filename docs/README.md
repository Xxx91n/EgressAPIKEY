# docs/ — Documentation Directory

## Structure

| Directory | Purpose | Diataxis quadrant |
|---|---|---|
| `architecture/` | System architecture, design decisions | Explanation |
| `adr/` | Architecture Decision Records (49 ADRs, MADR format) | Reference (decisions) |
| `how-to/` | Operational guides (deployment, release, runbook) | How-to |
| `reference/` | Glossary, API references | Reference |
| `research/` | Research findings, protocol analysis | Explanation |
| `history/` | Completed phase plans, handoffs, prompt archives | Archive (not current) |
| `DOCUMENT_GOVERNANCE_PLAN.md` | This docs structure governance plan | Constraint (top-level) |
| `PONYTAIL_DEBT_LEDGER.md` | Active ponytail debt tracking | Constraint (top-level) |
| `RESIN_UPSTREAM_MANIFEST.yaml` | Resin upstream version manifest | Reference (top-level) |

## Conventions

- All migrations use `git mv` (preserves history, R100 = content unchanged).
- Constraint docs stay at `docs/` top level; history artifacts move to `docs/history/`.
- For the full governance plan, see `DOCUMENT_GOVERNANCE_PLAN.md`.
