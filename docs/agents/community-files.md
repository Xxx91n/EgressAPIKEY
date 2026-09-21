# Community & contribution files

Moved out of `AGENTS.md` (progressive disclosure).

- `CONTRIBUTING.md` is the contributor entry: desktop dev-environment setup, verify-build usage, convention pointers, and the contribution-licensing statement (inbound = outbound GPL-3.0-or-later, no CLA — lands the ADR-0067 D5 decision).
- `.github/ISSUE_TEMPLATE/`: bug + feature markdown templates whose `name`/`about` frontmatter must stay legal (non-empty) — GitHub community-profile checklist is a hard condition on it; `config.yml` disables blank issues and routes questions to Discussions.
- `PULL_REQUEST_TEMPLATE.md` (root): PR verification checklist (verify-build, tests-per-behavior, i18n x18, license-field-check, docs same-commit).
- `SECURITY.md`: supported versions (latest `main`) + GitHub private vulnerability reporting — never public issues.
- `CODE_OF_CONDUCT.md`: Contributor Covenant 2.1 verbatim; enforcement contact = maintainer via GitHub profile.
- GitHub Discussions enabled (`has_discussions=true`, gh api). GOVERNANCE / FUNDING / SUPPORT stay deferred (spec D-07 P2).
