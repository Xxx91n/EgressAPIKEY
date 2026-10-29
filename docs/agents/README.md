# docs/agents/ — Agent Instruction Layer

Files here are **agent instructions** (read by AI agents when executing code tasks).
They are NOT user documentation. For user docs, see `docs/` parent.

## Agent operating rules

No rule below blocks on a tool being present. Each entry states what to do when
the tool is missing: degrade and continue, never stop.

### CodeGraph (preferred first step for code exploration)

The index lives at `.codegraph/` (gitignored, created by `codegraph init`).
Before reading files for "how does X work / where is X / what calls Y", try the
CLI from the repository root:

- `codegraph explore "<query>"` — relevant symbols' source plus call paths
- `codegraph query "<symbol>"` — symbol search
- `codegraph node <symbol>` — one symbol's source with its caller/callee trail
- `codegraph files` — file structure as the index sees it
- `codegraph status` — the freshness check. It prints `✓ Index is up to date`,
  or a `### Pending sync:` block naming each pending file and its edit age.
- `codegraph sync .` — only for the two cases the tool itself calls manual: the
  watcher is disabled (a sandbox that blocks it, or `CODEGRAPH_NO_DAEMON=1`), or
  the index is being queried from a script outside an agent session and needs a
  pre-flight sync (`--quiet` exists for git hooks). Otherwise the watcher plus
  the connect-time catch-up already took the change in — including edits from
  another editor or a `git pull`. Recommended, never a gate: CI installs no
  index.

When CodeGraph is not fully available:

- No CodeGraph MCP server in the session → use the CLI above. Every command
  listed there is a CLI invocation, so exploration never depends on the MCP
  server. `codegraph --help` additionally documents `explore` and `node` as
  producing the same output as the `codegraph_explore` and `codegraph_node` MCP
  tools.
- `codegraph` is not on PATH, or this checkout has no `.codegraph/` index →
  explore with the built-in read/search tools and continue. A missing index is
  not a failure and never blocks delivery.
- Count the source a CodeGraph command returned as already read only while the
  index is current. Staleness has an explicit signal: an MCP response that
  touches a still-pending file prepends a `⚠️` banner naming it and says to read
  it directly, and `codegraph status` lists pending files under
  `### Pending sync:`. No banner and no pending entry means the source is safe
  to treat as read; otherwise re-read the files the change touched.

### Windows host facts

- Inline `$` is swallowed by the PowerShell transport and the command still
  exits 0. Put PowerShell logic in a `.ps1` and run it with `-File`.
- Git revs containing `^` (`HEAD^`, `<rev>^`) passed to a shell-backed Node
  call — `exec` / `execSync`, or `spawn` / `execFile` with `shell: true` — go
  through cmd.exe, where `^` is the escape character: it is eaten without an
  error and the wrong revision is read in silence. `spawn` with an argv array
  and no `shell` is unaffected. Use an explicit `~1` rev, or compare blob ids
  with `git rev-parse <rev>:<path>`.

### File integrity

- New files: UTF-8 no BOM, LF endings (CRLF allow set: `.bat`, `.ps1`, `.cmd`). `git diff --check` must be clean before commit.
- Re-read the affected region before destructive edits; after two failed patch attempts, do one whole-file rewrite and verify bytes.
- Session-local dirs (`.scratch/`, `.codex-tmp/`, `.opencode/`, `.zcode/`, `.omx/`) never enter git tracking. `.gitignore` is the authority for that list, not this line — before a broad `git add`, verify a tool's new dir with `git check-ignore -v <path>`; a dir missing from `.gitignore` gets swept in with everything else. Untrack leftovers with `git rm -r --cached <path>` (files on disk are preserved, never deleted).

### Working etiquette

- If uncommitted changes appear that this agent did not make, treat them as user/external — do not revert them.
- Subagents: off by default; use them only when the session's user enables them, otherwise execute single-threaded.
- AtomCode research, when that CLI is installed, runs serialized — one active run at a time. Without it use the built-in web search/fetch tools.

## Index

| File | Purpose |
|---|---|
| `architecture-state.md` | Runtime wiring, sidecar lifecycle, ResinClient, IPC retarget, CI pipeline |
| `issue-tracker.md` | Issue tracker declaration: GitHub Issues |
| `domain.md` | Domain doc layout (`GLOSSARY.md`) and consumer rules |
| `triage-labels.md` | Triage label vocabulary: five default state roles |
| `storage-locations.md` | L1/L2/L3 config layers, key inventory, write paths |
| `community-files.md` | CONTRIBUTING / ISSUE_TEMPLATE / PR_TEMPLATE / SECURITY / CODE_OF_CONDUCT inventory |

`phase-history.md` was retired 2026-10-15 (process narrative; phase outcomes live in `CHANGELOG.md` and git history). The trigger-line register, the wave taskbooks and the frozen evidence bundles were removed 2026-10 (governance layer retirement).

## Legacy AGENTS.md section pointers

Older docs cite former AGENTS.md section numbers — the file
was slimmed on 2026-10-15. Map: `§7.5` / `§7.6` (IPC validation, SSRF, manifest)
→ `CONTRIBUTING.md` "IPC & security contracts";
`§Storage locations` → `storage-locations.md`; `§11` license → `CONTRIBUTING.md`
license discipline + `THIRD_PARTY.md`; all other `§N` → git history of
`AGENTS.md` (pre-2026-10-15).
