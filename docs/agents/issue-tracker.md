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
- **Blocking**: a `Blocked by` section in the body listing `#N, #N`. Closed is
  necessary, not sufficient — run [Unblocking: verifying a dependency actually
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
the blocked ticket needs is on the base branch. Before a ticket whose
`Blocked by` section names a dependency is claimed — and before its
`ready-for-agent` label is trusted — run these steps once per dependency and
write each verdict into the ticket. Every step is re-runnable as written;
"<base>" is the commit the work will branch from, resolved after a refresh —
`but pull --check` fetches without touching a branch (a bare `git fetch` is a VCS
write here), then `git rev-parse origin/main`.

1. **Restate the dependency as a behaviour.** One sentence naming what the
   blocked ticket needs from it: the merged fix, the guard, the API shape. Not
   the dependency's title. If the sentence needs a "because the ticket says so",
   the dependency is not understood yet — read it first.
2. **Find the commit that carries it on `<base>`, and call it `<dep-sha>`.**
   The commands print full hashes, because step 3 needs one:

   ```bash
   git log --format='%H %s' <base> -- <path>      # files the behaviour lives in
   git log -S'<symbol>' --format='%H %s' <base>   # when the code itself moved
   ```

   When the dependency landed as a pull request, the commit it put on the base
   is the PR's merge commit — the squash result, not the pre-merge head:

   ```bash
   gh pr view <PR> --json mergeCommit -q .mergeCommit.oid   # full 40-char sha
   ```

   Check that sha rather than the branch head, then
   `git merge-base --is-ancestor <dep-sha> <base> && echo landed`. A dependency
   whose commit is not an ancestor of `<base>` has not arrived, whatever its
   issue state says. This one sha is what steps 3 and 5 use.
3. **Confirm the CI verify run on that commit is green.** Delivery here is
   CI-only, so a local build or a run on a branch that was later rewritten is
   not evidence:

   ```bash
   gh run list --workflow ci --commit <dep-sha> --json conclusion,url
   ```

   `<dep-sha>` is the full 40-character sha from step 2: `--commit` matches the
   full sha only, and a short one returns an empty list — which reads like "no
   run" and is not. The run must be on `<dep-sha>` itself, and its conclusion
   must be `success`; `--commit` queries that one sha, so a green run on a later
   commit of the base branch does not substitute for it. An empty list for the
   full sha — or a run the concurrency group cancelled — is `not met`, not
   "unknown".
4. **Name the guard that keeps it fixed.** For a fix, find the test or guard
   (`git grep -n '<test name>' <base>`) and confirm it is wired into
   `scripts/verify-build.sh`; a fix with no guard re-rots silently and is not a
   met dependency. For a docs-only dependency the evidence is the text in tree —
   `git show <base>:<path>` — plus the ticket's own acceptance note.
5. **Record the verdict in the blocked ticket**, one line per dependency:
   `#N — met: <dep-sha> + <path:line> + <CI run>` or
   `#N — not met: <what is missing>`. Then rewrite the `Blocked by` section to
   the unmet subset — or to `None (can start immediately)` when that subset is
   empty.
6. **Closed but not met → do not start.** Reopen the dependency
   (`gh issue reopen <N> --comment "<missing evidence>"`), or file a follow-up,
   and leave the blocked ticket alone until step 5 records `met` for that
   dependency. Starting because the dependency is "probably fine" is the
   failure this section exists to prevent.
