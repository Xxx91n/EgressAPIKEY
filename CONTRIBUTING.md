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
bash scripts/fetch_resin.sh # pin the prebuilt Resin sidecar into src-tauri/binaries/
pnpm tauri dev # Vite dev server + Tauri shell (GUI)
```

Headless server (same browser control surface — desktop-only commands render disabled; no desktop shell): `pnpm build`, then `cargo build --release -p egressapikey-app --bin egressapikey-headless --features headless`, then run the binary with `--dist dist --binary-dir src-tauri/binaries`. Deployment layouts live in [docs/how-to/HEADLESS_DEPLOYMENT.md](docs/how-to/HEADLESS_DEPLOYMENT.md).

## Verify before you push

```bash
bash scripts/verify-build.sh # cargo build + tests, tsc + vite build, vitest, i18n coverage
```

CI runs the same script on every push and pull request. Focused checks:

- `pnpm test` — frontend vitest suite
- `pnpm i18n:check` — locale catalog completeness (all 18 base locales vs the canonical `en` catalog)
- `node scripts/i18n-content-check.cjs` — user-visible copy content: every locale/key value must still match its fingerprint in `docs/I18N_CONTENT_MANIFEST.json`
- `node scripts/license-field-check.cjs` — README / root Cargo.toml / package.json license fields stay GPL-3.0-or-later

## Conventions that gate review

These rules are authoritative here and are enforced by reviewers and CI (`scripts/verify-build.sh` mounts the machine checks):

- **i18n full-key coverage** — every user-visible string in `src/` comes from `src/locales/<locale>/*.json` via `t()` / `Trans`; never hard-code locale text in components. A new key must land in **all 18 base locales** in the same commit (`pnpm i18n:check` fails the build otherwise); the `Locale` union in `src/store/appStore.ts` and the `ALL` list in `scripts/i18n-check.cjs` stay in lockstep with the `src/locales/` directories. Coverage is the key set; values are gated separately by `scripts/i18n-content-check.cjs`, whose manifest stores per locale/key a `d` (sha256/12 of the raw value — what the gate compares) and a `t` (a code-point transcription, `[U+00A0]` for invisible characters — what the reviewer reads). Any value change — including a key added to all 18 locales at once — fails CI until it is approved with `node scripts/i18n-content-check.cjs --update`, whose manifest diff is the approval record; never hand-edit the manifest.
- **Test per behavior** — every public function in `crates/resin-core/` carries a unit test (same-file `#[cfg(test)]` or `crates/resin-core/tests/`); Zustand stores and pure reducers get vitest tests under `src/`; component interactions get Playwright e2e coverage under `e2e/`. The evaluator for the whole repo is `bash scripts/verify-build.sh` (what CI runs on every push/PR).
- **CI-only delivery** — local builds are forbidden as delivery evidence: a source commit is finished when the CI verify job proves it, and a testable exe comes from a release-matrix dispatch. Never hand out a locally-built binary; never claim a UI change is testable before its CI-built artifact exists.
- **Bilingual README pair** — `README.md` is the EN-canonical homepage skeleton (preserve its section order; route deep content to `docs/`); `README_CN.md` is the structure-locked CN mirror — update both sides in the same commit. The `## License` body stays exactly `GPL-3.0-or-later`.
- **License discipline** — the repository is GPL-3.0-or-later via [LICENSE](LICENSE) (verbatim GNU text — never reword); the README / workspace `Cargo.toml` / `package.json` license fields are machine-checked (`scripts/license-field-check.cjs`); vendored third-party components register in [THIRD_PARTY.md](THIRD_PARTY.md) with the two-layer declared-vs-tree value. Mere-aggregation invariant: shell ↔ sidecar interact only over the loopback REST seam — no FFI, no in-process linking, no source embedding. On any `docs/RESIN_UPSTREAM_MANIFEST.yaml` version bump, re-verify license facts against upstream originals at that tag and update `THIRD_PARTY.md` + the manifest in the same commit.
- **IPC & security contracts** — every `#[tauri::command]` validates input at the IPC layer before calling resin-core (lane range `0..MAX_LANES`, length caps ≤253 + NUL rejection, latency cap, `http(s)://` URL prefix); TS wrappers in `src/lib/ipc.ts` validate-then-invoke and treat Rust responses as untrusted. The headless router keeps **every** route behind `security_guard` in `src-tauri/src/headless_main.rs` (`--auth-token` on `/api/v1/*` + `/metrics/*`, Host/Origin allowlist, CSPRNG token — never register a route above the `.layer(...)` call).
- **Docs routing** — the README is a homepage; deep content belongs in `docs/` (architecture, how-tos). Retired docs are deleted, not archived in-tree — history lives in git.

## Commit and branch style

- Branch names are free-form; keep one logical change per branch and pull request.
- Commit subjects stay short and area-scoped (for example, `gateway: lane hash + SSE lease`); the body explains what changed and why.
- Update this file (conventions) and the pointer docs (`AGENTS.md`, `docs/agents/`) in the same commit when a change alters structure, conventions, or the release protocol.

## Pull requests

- The repository carries a pull request template (`PULL_REQUEST_TEMPLATE.md`); complete its verification checklist before requesting review.
- Link the issue the PR resolves (`Fixes #<issue>`).
- Fill the `## Screenshots / Recordings` section in the template when the PR changes user-visible UI, layouts, or renders; write `N/A` with a one-line reason for backend, IPC, resin-core, docs, or scripts-only PRs.
- Keep diffs reviewable; split unrelated changes into separate PRs.

## Contribution licensing

Contributions are accepted under the project license, **GPL-3.0-or-later**, on an inbound = outbound basis: by submitting a contribution you agree that it is licensed under GPL-3.0-or-later alongside the rest of the repository. No CLA or additional agreement is required (repo policy: inbound = outbound under the project license, no CLA).
