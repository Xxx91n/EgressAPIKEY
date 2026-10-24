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
- **Blocking**: a `Blocked by: #N, #N` line in the body; a ticket is unblocked when
  every issue it lists is closed.
- **Frontier**: `gh issue list --label needs-triage` (or `ready-for-agent` for AFK
  work); lowest number wins.
- **Claim**: `gh issue edit <N> --add-assignee @me` before any work.
- **Resolve**: close with the answer and a context pointer:
  `gh issue close <N> --comment "<answer + link>"`.

PRs as a request surface: **off** — external pull requests do not join the triage
queue.
