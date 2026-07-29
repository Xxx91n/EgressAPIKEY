# Architecture - ai-api-route

> Companion to README.md and docs/MEMORY_REUSE_DECISION.md. Concrete module layout, data flow, and tech stack.

## Architecture: Path A (fork Resin as Tauri sidecar)

Decision: see docs/MEMORY_REUSE_DECISION.md. Path A embeds the Go Resin binary as a Tauri sidecar; the Rust shell does lifecycle + Ghost safety net + IPC forwarding.

## Layers

React 19 Frontend (src/)
- ReactFlow 12 topology canvas with live sidecar-status banner
- Zustand 5 state (persisted to tauri-plugin-store via src/lib/settings.ts)
- Tailwind CSS + react-i18next (18 base locales)
- 5 tabs: Topology / Platforms / Subscriptions / ProcessRoute / Settings
- Typed IPC wrappers in src/lib/ipc.ts (validate-then-invoke, AGENTS S7.6)

Tauri 2 Shell (src-tauri/)
- sidecar.rs: boot_resin + spawn_health_poll (3s, 3-fail clear-os-proxy)
- commands/mod.rs: 19 #[tauri::command] forwarding to Resin via ResinClient
- tray.rs: i18n tray (18 locales), click-to-show

Resin Go Sidecar (bundle.externalBin)
- Control-plane API + proxy + webui on single loopback port
- Admin token: Rust-only trust, never crosses to webview

Resin-pattern Core (crates/resin-core/)
- resin_client.rs: loopback-only async REST client (SSRF guard + Bearer)
- mihomo.rs: controller wrapper (per-call DTO, not instantiated)
- lane.rs/lease.rs/tdewma.rs/platform.rs/gateway.rs: deprecated DTOs (S7.3)

## Data flow

1. boot_resin allocates port, spawns Go sidecar, polls /healthz
2. Frontend IPC -> #[tauri::command] validates -> ResinClient -> Resin REST
3. Ghost health poll: 3s /healthz, 3 fails -> tray red + clear OS proxy
4. TopologyView subscribes sidecar-status -> red banner on unhealthy

## Tech stack

Desktop shell: Tauri 2 (Rust)
Frontend: React 19, Vite 6, ReactFlow 12, Zustand 5, Tailwind 4, react-i18next
Sidecar: Resin Go binary (github.com/Resinat/Resin v1.1.2)
Tests: cargo test (54), vitest, playwright
Packaging: tauri build --features custom-protocol -> release/

## Fallback

If a platform Tauri build is blocked, ship the headless backend target.
