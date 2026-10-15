# EgressAPIKEY — Agent Instructions

EgressAPIKEY is a Tauri 2 + React 19 desktop app: an L7 proxy gateway specialized for AI API keys. Each key maps to a Resin (Platform, Account) pair; the vendored Resin Go sidecar guarantees a distinct sticky exit IP per pair.

## Run

- Dev: `pnpm install` → `bash scripts/fetch_resin.sh` → `pnpm tauri dev` — full setup in [CONTRIBUTING.md](CONTRIBUTING.md)
- Verify: `bash scripts/verify-build.sh` — the same script CI runs
- Headless deployment: `docs/how-to/HEADLESS_DEPLOYMENT.md`
- Delivery builds are **CI-only** (ADR-0072): local compile/test/packaging is never delivery evidence

## Stack

Rust core `crates/resin-core` (tokio, reqwest, rusqlite) · Tauri 2 shell `src-tauri` · frontend `src/` (React 19, TypeScript, Vite, Tailwind, ReactFlow 12, Zustand 5, react-i18next ×18 locales) · vendored Resin Go sidecar

## Layout

`crates/resin-core/` backend core · `src-tauri/` Tauri shell · `src/` frontend · `docs/` documentation · `scripts/` build + CI scripts · `e2e/` Playwright suites · `.scratch/` + `.codex-tmp/` are gitignored session-local dirs — never commit them

## Pointers

- Issue tracker: local markdown under `.scratch/` — declaration in `docs/agents/issue-tracker.md`
- Domain docs: `CONTEXT.md` (glossary) · `docs/adr/` · `docs/architecture/` · `docs/agents/`
- Agent operating rules (tool routing, CodeGraph, file integrity): `docs/agents/README.md`
- Review conventions (i18n / tests / CI-only / license / IPC): `CONTRIBUTING.md`
- VCS discipline: GitButler `but` for all VC writes — never `git` mutations directly; **never push or open PRs unless the user asks**; do not rewrite pushed/shared history; never revert changes this agent did not make
