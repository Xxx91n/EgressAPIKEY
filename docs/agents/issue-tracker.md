# Issue tracker: Local Markdown

Issues and specs for this repo live as markdown files under `.scratch/` at the repo
root (`.scratch/` is gitignored — the tracker is local by design, never committed).

## Conventions

- One feature per directory: `.scratch/<feature-slug>/`
- The spec is `.scratch/<feature-slug>/spec.md`
- Implementation issues are one file per ticket at
  `.scratch/<feature-slug>/issues/<NN>-<slug>.md`, numbered from `01` — never a
  single combined tickets file
- Triage state is recorded as a `**Status:**` line near the top of each issue
  file (see [triage-labels.md](triage-labels.md) for the role strings)
- Blocking edges are a `**Blocked by:** NN, NN` line near the top; a ticket is
  unblocked when every ticket it lists is done
- Handoff prompts for executor-agent windows live at
  `.scratch/<feature-slug>/handoffs/<NN>-<slug>.md`; their delivery reports come
  back at `.scratch/<feature-slug>/reports/<NN>-report.md`
- Comments and conversation history append to the bottom of the file under a
  `## Comments` heading

## When a skill says "publish to the issue tracker"

Create a new file under `.scratch/<feature-slug>/` (creating the directory if
needed).

## When a skill says "fetch the relevant ticket"

Read the file at the referenced path. The user will normally pass the path or
the issue number directly.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a file with one **child** file per ticket.

- **Map**: `.scratch/<effort>/map.md` — the Notes / Decisions-so-far / Fog body.
- **Child ticket**: `.scratch/<effort>/issues/NN-<slug>.md`, numbered from `01`,
  with the question in the body. A `Type:` line records the ticket type
  (`research`/`prototype`/`grilling`/`task`); a `Status:` line records
  `claimed`/`resolved`.
- **Blocking**: a `Blocked by: NN, NN` line near the top. A ticket is unblocked
  when every file it lists is `resolved`.
- **Frontier**: scan `.scratch/<effort>/issues/` for files that are open,
  unblocked, and unclaimed; first by number wins.
- **Claim**: set `Status: claimed` and save before any work.
- **Resolve**: append the answer under an `## Answer` heading, set
  `Status: resolved`, then append a context pointer (gist + link) to the map's
  Decisions-so-far in `map.md`.

## Remote availability and switching to GitHub Issues

The upstream remote is already usable: <https://github.com/Xxx91n/EgressAPIKEY>
(private; `origin`). Local markdown remains the tracker of record — including all
tickets of the `architecture-recovery` round — until the switch conditions below
are met.

Switch to GitHub Issues when either holds:

- external collaborators need to file or read issues (a gitignored `.scratch/` is
  invisible to them), or
- the user explicitly decides to move issue tracking to GitHub.

Switch procedure:

1. Create the label set on the GitHub repo (five state labels from
   [triage-labels.md](triage-labels.md), plus `bug` and `enhancement`):

   ```bash
   gh label create needs-triage
   gh label create needs-info
   gh label create ready-for-agent
   gh label create ready-for-human
   gh label create wontfix
   gh label create bug
   gh label create enhancement
   ```

2. Migrate open tickets, one `gh issue create` per open issue file:

   ```bash
   gh issue create --title "<issue title>" --body-file .scratch/<feature>/issues/NN-slug.md --label "<state-label>"
   ```

3. Replace the body of this file with the GitHub variant (seed template
   `issue-tracker-github.md` in the `setup-matt-pocock-skills` skill), or re-run
   `/setup-matt-pocock-skills` and pick GitHub.
4. Archive or delete `.scratch/<feature>/` per that feature's close-out
   convention once its tickets are migrated.

PRs as a request surface: **off**. Flip this flag in the GitHub template only if
external pull requests should join the triage queue.
