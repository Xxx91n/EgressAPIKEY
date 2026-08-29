# docs/agents/ — Agent Instruction Layer

Files here are **agent instructions** (read by AI agents when executing code tasks).
They are NOT user documentation. For user docs, see `docs/` parent.

| File | Purpose |
|---|---|
| `architecture-state.md` | Runtime wiring, sidecar lifecycle, ResinClient, IPC retarget, CI pipeline (AGENTS.md §11-17) |
| `phase-history.md` | Implementation phase records P9-P26, T7-T22, R1-R2, C1-C2 (AGENTS.md §18-66) |
| `issue-tracker.md` | Issue tracker declaration: local markdown under `.scratch/`, switch path to GitHub Issues |
| `domain.md` | Domain doc layout (single-context: `CONTEXT.md` + `docs/adr/`) and consumer rules |
| `triage-labels.md` | Triage label vocabulary: five default state roles |

These files were extracted from AGENTS.md to keep the root file under the 32 KiB
Codex `project_doc_max_bytes` limit (progressive disclosure principle).
