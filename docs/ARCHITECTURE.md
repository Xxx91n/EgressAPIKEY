# Architecture - ai-api-route

> Companion to README.md and docs/MEMORY_REUSE_DECISION.md. Concrete module layout, data flow, and tech stack in the actual repository, not the research survey.

## Layers

React 19 Frontend (src/)
- ReactFlow 12 topology canvas
- Zustand 5 state
- Tailwind CSS + react-i18next
- Views: topology, Settings, process route; reuses Resin webui Platform/Account/Subscription patterns

Tauri 2 Shell (src-tauri/)
- sidecar: resin-core ELF/Mach/PE
- system tray + Ghost-style safety net (disable OS proxy on sidecar death)
- panic hook + proxy reset

Resin-pattern Core (crates/resin-core/)
- L7 gateway  axum  (HTTP + SSE)
- Lane hash   FxHash(key) -> N lanes   (default 10, max 50)
- SSE session lock (lane, account) lease
- TD-EWMA-lite  per-authority latency EMA
- upstream pool  reqwest pool_max_idle=0
- Account/IP sticky lease table

mihomo sidecar  (clash-style core)
- subscribed nodes compiled to YAML
- pool_max_idle_per_host(0)  -> fresh TCP per request
- lane = inbound port -> node group -> distinct exit IP

## Data flow - a streaming request

1. OmniRoute client sets HTTPS_PROXY=http://127.0.0.1:7897 (unchanged address) and sends a chat-completions request with an Authorization: Bearer api-key.
2. Core gateway hashes the api-key with FxHash into one of N lanes (default 10, max 50).
3. The lane maps to a mihomo inbound port and an Account record owning an anchored exit IP lease.
4. The core opens the upstream request with reqwest using a per-request connection (pool idle = 0) and proxies the byte stream back.
5. For SSE responses, the lane is locked for the lifetime of the stream. The lease holds the anchored exit IP until the stream ends or errors. On failure the core records a TD-EWMA sample, evicts the lease, and the next request picks a fresh lane/IP.
6. The frontend topology canvas reflects live lane/account/IP state over Tauri events; Settings edits Platform/account/subscription state.

## Tech stack

Desktop shell: Tauri 2 (Rust)
Frontend: React 19, TypeScript 5, Vite 6, ReactFlow 12, Zustand 5, Tailwind CSS 4, react-i18next
Backend: Rust 1.80+ (tokio, axum, reqwest, petgraph, rusqlite, anyhow, thiserror)
Proxy core: resin-core pattern with mihomo sidecar subprocess under REST API control
i18n: react-i18next + i18next-scanner extraction; base en + zh, locale-decoupled message catalog
Tests: cargo test, vitest, playwright
Packaging: tauri-action matrix (Win/Linux/macOS GUI + headless backend target) -> release/

## Fallback plan

If a platform Tauri build is blocked (GUI unavailable on a CI runner), ship the headless backend target for that platform; the desktop GUI remains the supported interactive surface where Tauri builds. The pure-backend bundle is a single resin-core binary plus a generated config.toml.
