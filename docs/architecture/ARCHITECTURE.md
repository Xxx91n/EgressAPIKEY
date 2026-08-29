# Architecture - EgressAPIKEY

> Companion to README.md and docs/architecture/MEMORY_REUSE_DECISION.md. Concrete module layout, data flow, and tech stack.

## Architecture: Path A (fork Resin as Tauri sidecar)

Decision: see docs/architecture/MEMORY_REUSE_DECISION.md. Path A embeds the Go Resin binary as a Tauri sidecar; the Rust shell does lifecycle + Ghost safety net + IPC forwarding.

## Layers

React 19 Frontend (src/)
- ReactFlow 12 topology canvas with live sidecar-status banner
- Zustand 5 state (persisted to tauri-plugin-store via src/lib/settings.ts)
- Tailwind CSS + react-i18next (18 base locales)
- 5 tabs: Topology / Platforms / Subscriptions / ProcessRoute / Settings
- Typed IPC wrappers in src/lib/ipc.ts (validate-then-invoke, AGENTS S7.6)

Tauri 2 Shell (src-tauri/)
- sidecar.rs: boot_resin + spawn_health_poll (3s, 3-fail clear-os-proxy)
- commands/mod.rs: 44 #[tauri::command] (platforms/subscriptions/strategy/process_routes/port_auth/port_health/nodes/config_export/whitebox/stream_sensor/ip_reputation) forwarding to Resin via ResinClient
- tray.rs: i18n tray (18 locales), click-to-show

Resin Go Sidecar (bundle.externalBin)
- Control-plane API + proxy + webui on single loopback port
- Admin token: Rust-only trust, never crosses to webview

Resin-pattern Core (crates/resin-core/) - shell-side support crate (ADR-0050)
- resin_client.rs: loopback-only async REST client (SSRF guard + Bearer)
- whitebox_config.rs: egressapikey-ports.json store (ADR-0036); db.rs: SQLite port mapping
- port_forwarder.rs / port_health.rs / strategy_engine.rs / stream_sensor.rs / ip_reputation.rs / ipc_error.rs: live shell support modules
- platform.rs: Platform/Account registry (shell SharedRegistry state)
- lane/lease/tdewma/gateway/mihomo modules + CoreConfig + stub bin: deleted in ADR-0050 (zero external references)

## Data flow

1. boot_resin allocates port, spawns Go sidecar, polls /healthz
2. Frontend IPC -> #[tauri::command] validates -> ResinClient -> Resin REST
3. Ghost health poll: 3s /healthz, 3 fails -> tray red + clear OS proxy
4. TopologyView subscribes sidecar-status -> red banner on unhealthy

## Tech stack

Desktop shell: Tauri 2 (Rust)
Frontend: React 19, Vite 6, ReactFlow 12, Zustand 5, Tailwind 4, react-i18next
Sidecar: Resin Go binary (github.com/Resinat/Resin v1.2.0)
Tests: cargo test (100), vitest (123), playwright
Packaging: tauri build --features custom-protocol -> release/

## Fallback

If a platform Tauri build is blocked, ship the headless backend target.
