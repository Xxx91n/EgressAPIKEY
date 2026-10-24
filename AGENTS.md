# EgressAPIKEY — Agent Instructions

EgressAPIKEY is a Tauri 2 + React 19 desktop app: an L7 proxy gateway for AI API keys. Each key maps to a Resin (Platform, Account) pair; the vendored Resin Go sidecar guarantees a distinct sticky exit IP per pair. Backend core `crates/resin-core` (Rust) · Tauri shell `src-tauri` · frontend `src/` (React 19 + TypeScript).

## Run

- Dev: `pnpm install` → `bash scripts/fetch_resin.sh` → `pnpm tauri dev` (full setup: [CONTRIBUTING.md](CONTRIBUTING.md))
- Verify: `bash scripts/verify-build.sh` — the same script CI runs. Delivery is CI-only: a change is done when the CI verify job is green, never when a local build passes.
- VCS writes go through GitButler `but`; never push or open PRs unless the user asks.

## Do not edit

`resin/` (gitignored upstream reference clone) · `src-tauri/binaries/` (fetched sidecar) · `dist/` · `target/` · `src/bindings.ts` (tauri-specta generated — edit the Rust doc comments instead and regenerate)

## Pitfalls you cannot see from the code

- Port = identity: one entry port per (platform, account). Mode A (desktop): the shell forwarder injects the port's credential; Mode B (headless): Resin binds natively and the client presents `Platform.Account` once.
- Whitebox is truth: `egressapikey-strategy.json` / `egressapikey-ports.json` each have exactly ONE writer module; writes are validated, versioned (`backup/` ring of 10), audited, generation-counted. Reconcile is one-way — the whitebox always wins; desired-state entries leave only through explicit deletion.
- The Resin admin token never crosses into the webview; every upstream call goes through the loopback-only `ResinClient` seam. Never read Resin's private `*.db` files — the single exception is `backup_create` reading `state.db`/`cache.db` read-only for zip packaging.
- IPC boundary: validate every input on BOTH the TS and Rust sides (length caps, control chars, port ranges); errors are typed `IpcError` with i18n keys, never raw strings. Read-only GETs auto-retry; writes retry only on 5xx/network.
- i18n: a user-visible string lands in all 18 locales in the same commit (`pnpm i18n:check`).
- Runtime wiring detail: `docs/agents/architecture-state.md` · L1/L2/L3 key inventory: `docs/agents/storage-locations.md` · vocabulary: `GLOSSARY.md`

## Agent skills

- Issue tracker: GitHub — conventions in [docs/agents/issue-tracker.md](docs/agents/issue-tracker.md)
- Triage labels: [docs/agents/triage-labels.md](docs/agents/triage-labels.md)
- Domain docs: [docs/agents/domain.md](docs/agents/domain.md)
