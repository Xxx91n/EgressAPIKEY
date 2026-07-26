# Project Plan - ai-api-route

> Phased delivery (P0-P9) and success criteria. Living document; update when phases land.

## Phases

### P0 - Repo foundation (DONE at init)
- git init, .gitattributes LF policy, .gitignore, AGENTS.md /init expansion
- docs/: ARCHITECTURE.md, PROJECT_PLAN.md, MEMORY_REUSE_DECISION.md
- Success: clean `git diff --check`, AGENTS.md documents context-mode + codegraph + i18n + CI/CD + push protocol.

### P1 - Core proxy kernel (crates/resin-core/)
- Lane hash (FxHash, N=10 max=50)
- SSE session lease (lane, account, exit-ip)
- TD-EWMA-lite EMA per authority
- reqwest upstream pool with pool_max_idle_per_host(0)
- Platform/Account model + sticky exit IP lease table
- axum HTTP gateway (proxy + SSE forwarding) on 127.0.0.1
- mihomo sidecar REST control (compile subscription -> YAML -> reload)
- Process-route entry (optional third entry) - Tauri-managed per-process proxy
- Success: cargo test green for lane hash, lease eviction, TD-EWMA sample, SSE lock lifecycle.

### P2 - Tauri shell (src-tauri/)
- sidecar lifecycle (spawn resin-core, read 15s handshake JSON with api_port/token)
- system tray, single-instance lock
- Ghost safety net: on sidecar crash, clear OS proxy state, surface tray warning
- Tauri commands: get_state, edit_platform, edit_account, subscribe, open_topology
- Success: `pnpm tauri dev` launches GUI talking to sidecar; sidecar crash clears proxy.

### P3 - Frontend reuse + topology canvas
- Scaffold React 19 + Vite + Tailwind + Zustand
- Port Resin webui patterns: Platform grid, Account drawer, Subscription import (Rust-backed)
- Topology canvas (ReactFlow 12) - No-flush live lane/account/IP graph from Tauri events
- Settings view - Platforms, accounts, subscriptions, i18n locale, ports
- Process route view - per-app subprocess routing table editor
- Success: vetkomst browse topology + edit Platform + import subscription persisted to Rusqlite.

### P4 - i18n full coverage
- react-i18next, decoupled catalog under src/locales/<locale>/*.json
- i18next-scanner extraction in pnpm scripts
- CI check: no hard-coded user-visible English in components (lint)
- Success: `pnpm i18n:check` reports 100% en+zh key coverage.

### P5 - Tests
- Rust unit (crates/resin-core/*) + integration (tests/)
- vitest for Zustand stores + topology reducers
- playwright e2e: launch app, import subscription, verify lane hash + SSE lock visible
- Success: cargo test + vitest + playwright exit 0 on CI.

### P6 - CI/CD multi-platform packaging
- GitHub Actions matrix: windows-latest, ubuntu-latest, macos-latest
- tauri-action builds GUI installer per platform into release/ (named)
- Headless backend target: cross build resin-core for each OS, tar to release/<os>-backend.tar.gz
- iOS-class "GUI" target on macos-latest uses macOS arm64 dmg as supported desktop GUI; document that iOS iPad cannot run a Tauri desktop shell but macOS dmg is the iOS-compatible Apple-silicon desktop sibling
- Success: release/ contains windows-gui, linux-gui, macos-gui, and <os>-backend artifacts.

### P7 - Documentation + AGENTS.md freeze
- README.md / README_CN.md reflect actual commands
- AGENTS.md includes /init conventions: context-mode routing, codegraph mandatory use, i18n coverage rules, CI/CD artifact naming, push-after-change rule, test rules
- Success: docs match running app; new contributor can build following README.

### P8 - Polish + accessibility
- Keyboard navigation for topology + Settings
- System tray menu actions mapped to Tauri commands
- Crash handler reports path + opens issue link

### P9 - Release 0.1
- Tag v0.1.0, attach release/ artifacts to GitHub Release
- Changelog from git log

## Success criteria (overall)
- Evaluator: `bash scripts/verify-build.sh` exits 0
- cargo build + cargo test + pnpm build + pnpm test pass
- release/ contains Windows GUI, Linux/debian GUI, macOS GUI, and pure-backend artifacts for each OS
- AGENTS.md documents the full /init convention set enforced from P0
