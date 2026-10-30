# Issue tracker: GitHub

Issues for this repo live on GitHub: <https://github.com/Xxx91n/EgressAPIKEY/issues>
(private repo, `origin`). Triage state is the label set defined in
[triage-labels.md](triage-labels.md).

## When a skill says "publish to the issue tracker"

```bash
gh issue create --title "<title>" --label needs-triage --body "<body>"
```

## When a skill says "fetch the relevant ticket"

```bash
gh issue view <number>
```

## Wayfinding operations

- **Map**: a GitHub issue whose body carries the Notes / Decisions-so-far / Fog
  body; child tickets are linked from it.
- **Child ticket**: one `gh issue create` per question, with a `Type:` line in the
  body (`research` / `prototype` / `grilling` / `task`).
- **Blocking**: a `Blocked by: #N, #N` line in the body. Closed is necessary, not
  sufficient — run [Unblocking: verifying a dependency actually
  shipped](#unblocking-verifying-a-dependency-actually-shipped) before acting on
  the ticket.
- **Frontier**: `gh issue list --label needs-triage` (or `ready-for-agent` for AFK
  work); lowest number wins.
- **Claim**: `gh issue edit <N> --add-assignee @me` before any work.
- **Resolve**: close with the answer and a context pointer:
  `gh issue close <N> --comment "<answer + link>"`.

PRs as a request surface: **off** — external pull requests do not join the triage
queue.

## Unblocking: verifying a dependency actually shipped

`closed` records that a conversation ended; it does not record that the behaviour
the blocked ticket needs is on the base branch. Before a ticket with a
`Blocked by:` line is claimed — and before its `ready-for-agent` label is
trusted — run these steps once per dependency and write each verdict into the
ticket. Every step is re-runnable as written; "<base>" is the commit the work
will branch from (`git rev-parse origin/main`).

1. **Restate the dependency as a behaviour.** One sentence naming what the
   blocked ticket needs from it: the merged fix, the guard, the API shape. Not
   the dependency's title. If the sentence needs a "because the ticket says so",
   the dependency is not understood yet — read it first.
2. **Find the commit on the base branch.** `git log --oneline <base> --
   <path>` for the files the behaviour lives in, `git log -S'<symbol>'
   --oneline <base>` for when the code itself moved, then
   `git merge-base --is-ancestor <dep-sha> <base> && echo landed`. A dependency
   whose commit is not an ancestor of `<base>` has not arrived, whatever its
   issue state says.
3. **Confirm the CI verify run on that commit is green.** Delivery here is
   CI-only, so a local build or a run on a branch that was later rewritten is
   not evidence:

   ```bash
   gh pr view <PR> --json mergeCommit -q .mergeCommit.oid
   gh run list --workflow ci --commit <dep-sha> --json conclusion,url
   ```

   The run must be on the dependency's merge commit (or a descendant of it on
   the base branch), and its conclusion must be `success`.
4. **Name the guard that keeps it fixed.** For a fix, find the test or guard
   (`git grep -n '<test name>' <base>`) and confirm it is wired into
   `scripts/verify-build.sh`; a fix with no guard re-rots silently and is not a
   met dependency. For a docs-only dependency the evidence is the text in tree —
   `git show <base>:<path>` — plus the ticket's own acceptance note.
5. **Record the verdict in the blocked ticket**, one line per dependency:
   `#N — met: <dep-sha> + <path:line> + <CI run>` or
   `#N — not met: <what is missing>`. Then rewrite the `Blocked by:` line to the
   unmet subset, and delete it only when that subset is empty.
6. **Closed but not met → do not start.** Reopen the dependency (or file a
   follow-up) quoting the missing evidence, and leave the blocked ticket alone
   until step 5 reads `met` in the dependency's own ticket. Starting because the
   dependency is "probably fine" is the failure this section exists to prevent.
