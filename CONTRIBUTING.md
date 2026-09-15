# Contributing to EgressAPIKEY

Welcome, and thanks for considering a contribution. EgressAPIKEY is a desktop L7 proxy gateway for AI API keys (Tauri 2 + React 19 shell, Rust `crates/resin-core` backend core, vendored Resin Go sidecar). This guide covers the development environment, the verification workflow, and the conventions a pull request must hold.

## Code of conduct

Participation is governed by the [Code of Conduct](CODE_OF_CONDUCT.md). By taking part in this project you agree to uphold it.

## Reporting issues

- **Security vulnerabilities** — follow [SECURITY.md](SECURITY.md) and use private vulnerability reporting. Never open a public issue for a vulnerability.
- **Bugs** — use the [bug report template](https://github.com/Xxx91n/EgressAPIKEY/issues/new?template=bug_report.md). Include reproduction steps, OS, build source, and sanitized logs (redact API keys and upstream credentials).
- **Feature ideas and open questions** — open a [feature request](https://github.com/Xxx91n/EgressAPIKEY/issues/new?template=feature_request.md) or start a thread in [Discussions](https://github.com/Xxx91n/EgressAPIKEY/discussions).

## Development environment (desktop app)

Prerequisites:

- Node.js 20+ and pnpm (`corepack enable` provides pnpm)
- Rust stable via rustup, with the platform toolchain (MSVC on Windows)
- Tauri v2 system webview dependencies — see the official [Tauri v2 prerequisites](https://tauri.app/start/prerequisites/) (Linux needs `webkit2gtk-4.1` and friends)
- Optional: Go 1.24+, only if you intend to rebuild the vendored Resin sidecar from source instead of fetching the pinned prebuilt binary

Set up and run the desktop app:

```bash
git clone https://github.com/Xxx91n/EgressAPIKEY.git
cd EgressAPIKEY
pnpm install
bash scripts/fetch_resin.sh     # pin the prebuilt Resin sidecar into src-tauri/binaries/
pnpm tauri dev                  # Vite dev server + Tauri shell (GUI)
```

Headless server (same browser control surface — desktop-only commands render disabled; no desktop shell): `pnpm build`, then `cargo build --release -p egressapikey-app --bin egressapikey-headless --features headless`, then run the binary with `--dist dist --binary-dir src-tauri/binaries`. Deployment layouts live in [docs/how-to/HEADLESS_DEPLOYMENT.md](docs/how-to/HEADLESS_DEPLOYMENT.md).

## Verify before you push

```bash
bash scripts/verify-build.sh    # cargo build + tests, tsc + vite build, vitest, i18n coverage
```

CI runs the same script on every push and pull request. Focused checks:

- `pnpm test` — frontend vitest suite
- `pnpm i18n:check` — locale catalog completeness (all 18 base locales vs the canonical `en` catalog)
- `node scripts/license-field-check.cjs` — README / root Cargo.toml / package.json license fields stay GPL-3.0-or-later

## Conventions that gate review

These are enforced by reviewers and CI; full operating rules live in [AGENTS.md](AGENTS.md):

- **i18n full-key coverage** — every user-visible string comes from `src/locales/<locale>/*.json` via `t()` / `Trans`; a new key must land in all 18 base locales in the same commit. Never hard-code locale text in components.
- **Test per behavior** — every public function in `crates/resin-core/` carries a unit test (same file or `crates/resin-core/tests/`); Zustand stores and pure reducers get vitest tests under `src/`; component interactions get Playwright e2e coverage under `e2e/`.
- **License discipline** — the repository is GPL-3.0-or-later ([LICENSE](LICENSE)); the three license fields are machine-checked; vendored third-party components are registered in [THIRD_PARTY.md](THIRD_PARTY.md).
- **Docs routing** — the README is a homepage; deep content belongs in `docs/` (architecture, ADRs, how-tos).

## Commit and branch style

- Branch names are free-form; keep one logical change per branch and pull request.
- Commit subjects stay short and area-scoped (for example, `gateway: lane hash + SSE lease`); the body explains what changed and why.
- Update [AGENTS.md](AGENTS.md) in the same commit when a change alters structure, conventions, or the release protocol.

## Pull requests

- The repository carries a pull request template (`PULL_REQUEST_TEMPLATE.md`); complete its verification checklist before requesting review.
- Link the issue the PR resolves (`Fixes #<issue>`).
- Fill the `## Screenshots / Recordings` section in the template when the PR changes user-visible UI, layouts, or renders; write `N/A` with a one-line reason for backend, IPC, resin-core, docs, or scripts-only PRs.
- Keep diffs reviewable; split unrelated changes into separate PRs.

## Contribution licensing

Contributions are accepted under the project license, **GPL-3.0-or-later**, on an inbound = outbound basis: by submitting a contribution you agree that it is licensed under GPL-3.0-or-later alongside the rest of the repository. No CLA or additional agreement is required (decision point recorded in [ADR-0067](docs/adr/0067-license-layering-provenance.md) D5).
