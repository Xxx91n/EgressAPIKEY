# docs/agents/ — Agent Instruction Layer

Files here are **agent instructions** (read by AI agents when executing code tasks).
They are NOT user documentation. For user docs, see `docs/` parent.

## Agent operating rules (mandatory)

### context-mode routing

- File edits (including patches) go through `ctx_batch_execute` / `ctx_execute_file`, not apply_patch.
- `ctx_*` first; fall back to builtins only when `ctx_*` can't do the job. Read-to-analyze / search / large grep: `ctx_batch_execute` or `ctx_search` — never Get-Content/Select-String into context. For data analysis use `ctx_execute(code)` and print only the answer.
- Web/HTTP: `ctx_fetch_and_index(url, source)` then `ctx_search(queries)`. curl/wget/inline HTTP are forbidden.
- Shell is fine for bounded execution: git ops, mkdir, rm, mv, cd, ls, builds, tests.
- Windows paths in the ctx sandbox: forward-slash form `D:/Aworker/EgressAPIKEY/...`; the `shell` language routes to `pwsh.exe` (PowerShell 7), NOT bash; inline `$` is stripped by the host transport — put PowerShell logic in a `.ps1` and run with `-File`.
- After resume: `ctx_search(sort:"timeline")` before asking the user anything.
- Output artifacts as files + path + one-line summary; never inline large content into chat.

### CodeGraph (first step for code exploration)

The index lives at `.codegraph/` (gitignored). Before reading files for "how does X work / where is X / what calls Y", query CodeGraph first:

- MCP: `codegraph_explore` with `projectPath: "D:\Aworker\EgressAPIKEY"`
- CLI: `codegraph explore "<query>"` / `codegraph query "<symbol>"` / `codegraph node <symbol>` / `codegraph files`
- After any code change, run `codegraph sync .` in the same session (recommended, not a gate — CI holds no index)
- Treat codegraph-returned source as already-Read; do not re-open those files in the same session

### File integrity

- New files: UTF-8 no BOM, LF endings (CRLF allow set: `.bat`, `.ps1`, `.cmd`). `git diff --check` must be clean before commit.
- Re-read the affected region before destructive edits; after two failed patch attempts, do one whole-file rewrite and verify bytes.
- Never inline `$` PowerShell logic; write a `.ps1` and run with `-File`.
- Session-local dirs (`.scratch/`, `.codex-tmp/`, `.zcode/`, `.omx/`, `context-mode/`) never enter git tracking — each has a literal `.gitignore` line; untrack leftovers with `git rm -r --cached <path>` (files on disk are preserved, never deleted).

### Working etiquette

- If uncommitted changes appear that this agent did not make, treat them as user/external — do not revert them.
- Subagents: off by default; use them only when the session's user enables them, otherwise execute single-threaded.
- AtomCode research runs are serialized — one active run at a time.

## Index

| File | Purpose |
|---|---|
| `architecture-state.md` | Runtime wiring, sidecar lifecycle, ResinClient, IPC retarget, CI pipeline |
| `issue-tracker.md` | Issue tracker declaration: local markdown under `.scratch/`, switch path to GitHub Issues |
| `domain.md` | Domain doc layout (`CONTEXT.md` + `docs/adr/`) and consumer rules |
| `triage-labels.md` | Triage label vocabulary: five default state roles |
| `trigger-line-register.md` | Live register of armed/parked trigger lines + write-back history |
| `evidence/` | Frozen evidence bundles cited by register rows |
| `storage-locations.md` | L1/L2/L3 config layers, key inventory, write paths |
| `ipc-manifest.md` | Machine-checked IPC command manifest (regenerated — do not hand-edit) |
| `community-files.md` | CONTRIBUTING / ISSUE_TEMPLATE / PR_TEMPLATE / SECURITY / CODE_OF_CONDUCT inventory |
| `round12-wave-l-next-round.md` | Live committed watch taskbook |

`phase-history.md` was retired 2026-10-15 (process narrative; phase outcomes live in `CHANGELOG.md` and git history).

## Legacy AGENTS.md section pointers

Older docs (ADRs, CONTEXT.md) cite former AGENTS.md section numbers — the file
was slimmed on 2026-10-15. Map: `§7.5` / `§7.6` (IPC validation, SSRF, manifest)
→ `CONTRIBUTING.md` "IPC & security contracts" + `ipc-manifest.md`;
`§Storage locations` → `storage-locations.md`; `§11` license → `CONTRIBUTING.md`
license discipline + `THIRD_PARTY.md`; all other `§N` → git history of
`AGENTS.md` (pre-2026-10-15).
