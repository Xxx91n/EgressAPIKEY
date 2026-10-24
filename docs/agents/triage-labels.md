# Triage Labels

The skills speak in terms of five canonical triage roles. This file maps those
roles to the actual label strings used in this repo's issue tracker.

This repo uses the default vocabulary: each label string equals its canonical
role name.

| Label in mattpocock/skills | Label in our tracker | Meaning |
| -------------------------- | -------------------- | ---------------------------------------- |
| `needs-triage` | `needs-triage` | Maintainer needs to evaluate this issue |
| `needs-info` | `needs-info` | Waiting on reporter for more information |
| `ready-for-agent` | `ready-for-agent` | Fully specified, ready for an AFK agent |
| `ready-for-human` | `ready-for-human` | Requires human implementation |
| `wontfix` | `wontfix` | Will not be actioned |

Because the tracker is GitHub Issues
([issue-tracker.md](issue-tracker.md)), "applying a label" means calling the
tracker API: `gh issue edit <N> --add-label "<label>"` (remove with
`--remove-label`). Listing what is waiting is `gh issue list --label <label>`.

When a skill mentions a role (e.g. "apply the AFK-ready triage label"), use the
corresponding label string from this table.
