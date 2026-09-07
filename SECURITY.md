# Security Policy

## Supported versions

Only the latest revision of `main` receives security fixes; there is no tagged release yet, so no prior version is supported.

## Reporting a vulnerability

Use GitHub private vulnerability reporting: repository **Security** tab, then **Report a vulnerability**. Do not open a public issue or pull request for anything you believe is exploitable. Include the affected surface (desktop shell, headless server, or Resin sidecar seam), the version or commit, reproduction steps, and impact, and redact API keys and credentials from any logs you attach.

## Response timeline

When a private vulnerability report lands, maintainers commit to the following windows (aligned with the upstream Resin security disclosure window):

- **T+72h acknowledgement** — a maintainer acknowledges receipt, opens an internal triage ticket, and assigns an owner within 72 hours of the report.
- **T+30d fix or status update** — for confirmed vulnerabilities, a fix or an interim mitigation lands within 30 days, accompanied by a public advisory drafted for the GitHub Security Advisories tab. If the investigation is still in progress at day 30, the maintainer posts a written status update (scope, residual risk, next checkpoint) in the same advisory thread.

Out-of-window updates do not block the disclosure — the maintainer continues the triage thread and ships the fix as soon as the patch is verified. The window is a commitment to acknowledgement cadence, not a deadline that releases the reporter's right to public disclosure after a reasonable further delay.

## Hall of Fame

Researchers who report a confirmed, in-scope vulnerability are listed here with their consent. The list is empty until the first report ships through the channel above.

| Reporter | Date | Advisory | Scope |
| --- | --- | --- | --- |
| _no entries yet_ | — | — | — |

If you would like to be acknowledged but prefer a handle or an organization name over a personal name, state that in the initial report.
