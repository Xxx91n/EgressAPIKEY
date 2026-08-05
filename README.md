# EgressAPIKEY

> Formerly **ai-api-route**. Renamed per ADR-0013.

[中文](README_CN.md)

**Multi-port socks5/http forwarder between AI gateways and upstream v1 providers — exit-IP routing for AI API keys, with a topology canvas.**

A desktop application (Tauri 2 + React 19) that sits between an AI gateway (omniroute/litellm/cliproxy) and upstream v1 providers. It exposes many socks5/http entry ports; each port is the identity for a (platform, account) pair mapped onto a Resin Go sidecar. Topology canvas + allocation strategies route each port to exit IPs. HTTPS upstreams are opaque to a single unified proxy, so multi-port is the correct identity mechanism (ADR-0012).

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
## Platform & IP-channel routing

The **Topology** tab is a three-column key-to-egress canvas (entry port → platforms → IP channels):
- **A: entry proxy port** — the conceptual Resin forward-proxy entry; always connected to every platform.
- **B: platforms** — one node per Resin platform; an edge from a platform to a node-group means the platform's `region_filters` includes that region. Drag-to-connect hot-PATCHes the platform `region_filters` on the live Resin sidecar; deleting an edge removes the region. Every topology drag saves a backup first (防呆).
- **C: IP channels / nodes** — one node per region-grouped Resin node set; shows healthy/total count and routable-node count.

The **Platforms** tab is a dual-pane key-to-platform surface:
- **Left pane**: candidate key combinations (upstream v1 endpoint + API key), stored locally in `settings.json#keyCandidates` — never sent to Resin until activated.
- **Right pane**: live Resin platforms + per-platform leases. Drag a key from the left into the right pane's empty space to create an `auto-{uid}` independent platform (POST /platforms with `BALANCED`); drop a key onto an existing platform to attach it. Solid-border cards are auto-independents; dashed-border cards are manual platforms. Per-platform `allocation_policy` is a 5-label egress policy selector (random/sequential → `BALANCED`, latency → `PREFER_LOW_LATENCY`, quality → `PREFER_IDLE_IP`).

The **Nodes** tab shows the live node-pool snapshot plus a protocol-weight reference card (SSE suitability: http/socks5/vmess=1.0, shadowsocks=0.7, hysteria2/tuic/wireguard=0.1) and an egress-policy guidance card pointing to the Topology canvas where per-platform `allocation_policy` is bound. No GUI-only egress policy selector is invented — Resin v1.1.2 has no per-node strategy endpoint, so the policy selector binds the real platform `allocation_policy` field.


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
