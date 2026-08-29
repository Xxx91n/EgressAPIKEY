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

### Config Authority (配置权威)

> This subsection legislates the three-layer configuration authority model. It
> EXTENDS decided ADRs and reopens none of them: ADR-0036 (strategy whitebox
> single write entry), ADR-0039 SS2 (read-side strategy contract), ADR-0042
> S2/S6 (whitebox port truth + restore from whitebox). Where this subsection and
> an ADR appear to conflict, the ADR wins and this subsection gets fixed.

Provenance: 2026-08-29 architecture candidate report + atomcode research #1
(15 sources: clashverge.dev, clash wiki, mihomo wiki, sing-box, v2rayN).
Desktop proxy clients have converged on three config layers plus a merge
chain, and on one rule: **the authoritative source is the write entry**. The
CFW↔Verge merge-semantics incompatibility (upstream issue #847) is the
standing lesson — borrow the layering, not the field semantics.

| Layer | Storage | Sole write entry | Authority semantics |
| --- | --- | --- | --- |
| L1 GUI preferences | `settings.json` (`app_config_dir()`, tauri-plugin-store) | Webview `src/lib/settings.ts` + `lightweight_get/set` / `get_log_level` / `set_log_level` commands | Preference only; never influences proxy behavior |
| L2 Whitebox user-editable | `egressapikey-strategy.json` + `egressapikey-ports.json` (`app_config_dir()`) | Strategy: `strategy_config_put` then `strategy_apply`. Ports: `WhiteboxConfigStore` (`crates/resin-core/src/whitebox_config.rs`, hotswap-config atomic write) | The file IS the truth; GUI edits and external edits converge here (ADR-0036) |
| L3 Resin runtime | Resin sidecar state (`state.db`, leases, listeners); shell-side partner `egressapikey.db` (`port_mappings` table) | ResinClient REST seam only (`crates/resin-core/src/resin_client.rs`) | Derived and rebuildable from L2 at any time; execute authority, never a config source |

Write entry = authority, per layer: an L1 key must not change what the proxy
does (if it would, it belongs in L2); L2 is the only layer where a
user-authored file is read back and honored; L3 is never authored by hand —
needing to edit L3 state to change behavior is a bug. No new config storage
may be introduced without re-legislating this subsection. Logs
(`app_log_dir()`, Resin request logs) are observability data, not a layer.

Two distinct write pipelines currently live behind L2 (facts from the
2026-08-29 candidate report, verified in code; `strategy_config_put` /
`strategy_apply` / `port_upsert` / `whitebox_reload` in
`src-tauri/src/commands/mod.rs`):

- Strategy: GUI or JSON edit → `strategy_config_put` (validate, write
  `egressapikey-strategy.json`) → `strategy_apply` (compute plan, PATCH Resin
  `region_filters`, auto-clean stale platform entries and write the file
  back). Destination: ticket 10 converges read/validate/apply/read-back into
  one StrategyService module (ADR-0036 stays in force).
- Ports: `port_upsert` / `port_remove` / `port_toggle` /
  `whitebox_save_network` → `WhiteboxConfigStore` (atomic write +
  validate-before-swap + file watch) → accepted files applied to
  `egressapikey.db` + listeners as one transaction; boot seeds the store from
  the DB, and `restore_ports_from_whitebox` re-POSTs `/api/v1/endpoints` to
  Resin after a sidecar restart (ADR-0042 S6). Three-party sync: whitebox
  JSON ↔ `egressapikey.db` ↔ Resin endpoints, orchestrated in
  `src-tauri/src/main.rs`.

Current wiring (before ticket 07 — views poll and merge across stores):

```mermaid
flowchart TD
    V[Settings / Platforms / Topology views] --> S[L1 settings.json]
    V --> ST[L2 strategy.json] --> AP[strategy_apply] --> R[(L3 Resin runtime)]
    V --> WB[L2 ports.json] <--> DB[(egressapikey.db)] --> R
    V -->|5s poll + view-layer merge| R
```

Target state (one write entry per layer + one authoritative read-back):

```mermaid
flowchart TD
    L1[L1 settings.json GUI prefs] --> GUI[GUI]
    GUI -->|sole write entry| L2[L2 strategy.json + ports.json]
    L2 -->|apply| L3[L3 Resin runtime, rebuildable]
    SNAP[authoritative snapshot read-back] --> GUI
    SNAP --> L2
    SNAP --> L3
```

Effective-snapshot read-back contract (ticket 07 prerequisite): a single deep
IPC (authoritative-snapshot semantics) returns the merged effective
configuration in ONE call, reading three sources (L2 strategy JSON, L2 ports
JSON + `egressapikey.db`, L3 Resin runtime) and marking divergence per
platform/port. Each entry is in exactly one of three states:

- `consistent` — whitebox and Resin runtime agree.
- `divergent` — both sides readable but disagree (apply failed or was
  overridden); the snapshot carries both values and never silently picks one.
- `missing` — one side is absent (e.g. a platform exists in the whitebox but
  not in Resin, or a port's Resin endpoint vanished).

The snapshot is the ONLY sanctioned cross-store merge point. View layers
consume it and must not re-merge stores. Implemented 2026-08-30 (ticket 07,
ADR-0051): the former `TopologyView.tsx` `sync()`-time merge of
`cfgRaw.platforms` into Resin platform rows is deleted; the view polls the
`authoritative_snapshot` IPC, and the merge lives in
`crates/resin-core/src/snapshot.rs` (pure, unit-tested against the three
legislated fixtures). Adding a new view-layer merge is a review blocker.

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
