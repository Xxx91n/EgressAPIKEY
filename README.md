# AI API Route

[中文](README_CN.md)

**AI API key specialized proxy pool with a topology canvas.**

A desktop application (Tauri 2 + React 19) that provides an L7 proxy gateway for AI API keys. Each key is hashed into a lane, and every request opens a fresh TCP connection through mihomo to guarantee distinct exit IPs — solving the same-domain/same-IP collision problem that plagues AI API key pools.

## Quick start

```powershell
pnpm install
pnpm tauri dev
```

## Key design

- **N lanes** (default 10, max 50) — keys hash into lanes, not 1:1 port mapping
- **Per-request TCP** — `pool_max_idle_per_host(0)` guarantees fresh connections
- **SSE session stickiness** — stream locks node until completion, auto-switches on failure
- **Zero adaptation** — OmniRoute users change one proxy address; client code unchanged
- **Backup safety** — backups write into the per-user app data dir (not the shared system temp), with a path-confinement guard so a compromised webview cannot exfiltrate arbitrary files via the WebDAV upload path

## Docs

| File | Purpose |
|------|---------|
| `docs/MEMORY_REUSE_DECISION.md` | Compressed research memory — read first |
| `docs/ARCHITECTURE.md` | Layers, data flow, tech stack, fallback plan |
| `docs/PROJECT_PLAN.md` | Phased delivery (P0–P9) and success criteria |
| `docs/RELEASE.md` | Release pipeline: CI matrix, artifact groups, iOS-class note |

## Tech stack

| Layer | Technology |
|-------|-----------|
| Desktop shell | Tauri 2 (Rust) |
| Frontend | React 19, TypeScript, ReactFlow 12, Zustand 5, Tailwind CSS |
| Backend | Rust (tokio, axum, reqwest, rusqlite) |
| Proxy core | mihomo (sidecar subprocess, REST API control) |

## License

GPL-3.0-or-later

## i18n

Decoupled catalog under `src/locales/<locale>/*.json`. 18 base locales today (`en`, `zh`, `ja`, `es`, `fr`, `de`, `ko`, `ru`, `pt`, `ar`, `it`, `nl`, `pl`, `tr`, `vi`, `th`, `id`, `hi`); `en` is the canonical key set. `pnpm i18n:scan` extracts keys; `pnpm i18n:check` fails the build on any missing/extra locale key vs `en`. New user-visible strings must touch every base locale in the same commit; see AGENTS.md `/init conventions` section 3.

## Tests & build

```powershell
bash scripts/verify-build.sh   # cargo build+test, tsc, vite build, vitest, i18n check
bash scripts/build-all.sh      # local reproduction of the CI matrix (backend tarball + GUI if tauri-cli installed)
```

## Release artifacts

CI builds five artifact groups into `release/` (published to the GitHub Release): Windows GUI, Linux/debian GUI, macOS GUI (universal/arm64), plus per-OS headless backend tarballs. iPadOS cannot run a Tauri desktop shell; the Apple-silicon desktop sibling is the macOS `.dmg`. See `docs/RELEASE.md`.
