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

## Docs

| File | Purpose |
|------|---------|
| `docs/MEMORY.md` | Compressed research memory — read first |
| `docs/ARCHITECTURE.md` | Layers, data flow, tech stack, fallback plan |
| `docs/PROJECT_PLAN.md` | Phased delivery (P0–P9) and success criteria |

## Tech stack

| Layer | Technology |
|-------|-----------|
| Desktop shell | Tauri 2 (Rust) |
| Frontend | React 19, TypeScript, ReactFlow 12, Zustand 5, Tailwind CSS |
| Backend | Rust (tokio, axum, reqwest, petgraph, rusqlite) |
| Proxy core | mihomo (sidecar subprocess, REST API control) |

## License

MIT
