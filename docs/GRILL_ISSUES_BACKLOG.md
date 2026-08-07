# GRILL ISSUES BACKLOG (Route-corrected — 2026-08-05)

> ADR-0012 (route correction) + ADR-0013 (rename to EgressAPIKEY) + ADR-0014
> (dead code deletion) are ACCEPTED. This backlog reflects the post-correction
> state. Items marked SUPERSEDED are kept for audit trail only.

## Q1-Q11 Decision Summary (this grill round)

| Q | Topic | Decision | ADR |
|---|---|---|---|
| Q1 | Reuse Resin vs build new | Reuse Resin (Ponytail) | ADR-0012 |
| Q2 | Architecture: thin shell A vs fork C | Path A (thin shell, not fork) | ADR-0012 |
| Q3 | HTTP vs SOCKS5 identity | HTTP for unified port; multi-port for per-key isolation | ADR-0012 |
| Q4 | GUI + whitebox config sync | A+B both (GUI for masses, whitebox for power users, atomic sync) | ADR-0012 |
| Q5 | Config hot-reload mechanism | hotswap-config (community-vetted wheel) | ADR-0012 |
| Q6 | Commit granularity | Small step commits (Ponytail) | — |
| Q7 | Strategy layer + AI stream sensor + IP reputation | Modular pluggable strategies (not global); independent AI stream sensor board; IP reputation via external API wheels (IPQualityScore/AbuseIPDB/ip-api) | ADR-0012 |
| Q8 | IP reputation scoring | Use open-source mature wheels, not self-built; IPQualityScore (5K/mo free) + AbuseIPDB (1K/day) + ip-api.com (45/min) | ADR-0012 |
| Q9 | Project rename | EgressAPIKEY (no collision, audited 5 sources) | ADR-0013 |
| Q10 | Dead code: delete vs keep vs refactor | (a) Delete all dead code (Ponytail clean break) | ADR-0014 |
| Q11 | Backlog audit:失效/保留/重定义 | See tables below | — |

## 失效项 (SUPERSEDED — kept for audit trail only)

| Item | Reason |
|---|---|
| C1-1 (observed_keys SQLite + route_id GUI chips) | route_id based on reading Authorization header; HTTPS makes this impossible. SUPERSEDED by ADR-0012. |
| ADR-0003 (key+endpoint identification via header) | Same premise error. SUPERSEDED by ADR-0012. |
| ADR-0011 (B-B-3 route_id-derived key identity) | Same. SUPERSEDED by ADR-0012. SQLite infra reused; observed_keys schema deleted. |
| P24-A4-3 (axum interceptor) | Entire file deleted per ADR-0014. |
| C2-3 (keyCandidates cross-view persistence) | keyCandidates concept (manual endpoint+apiKey input) abolished. Replaced by Entry Ports. |
| C2-4 (auto vs manual platform card visual) | key candidate concept gone; platform card visual stays but binds to ports not keys. |
| TopologyView B-column route_id chip rendering | Deleted. B column now renders port-based identity. |

## 保留项 (unaffected by route correction)

| Item | Status | Notes |
|---|---|---|
| C1-2 (hot-connect atomic PATCH + state refresh) | confirmed | Topology edge PATCH semantics unchanged |
| C1-3 (drag-delete edge = remove region_filters) | confirmed | Idempotent removal unchanged |
| C1-4 (viewport memory + initial fitView) | confirmed | Pure frontend, identity-agnostic |
| C2-1 (subscription drag reorder persists) | done=607460a | Unaffected |
| C2-2 (subscription rename/delete coexistence) | done=192bc82 | Unaffected |
| C2-5 (tray i18n real-time sync) | done=55f6583 | Unaffected |
| C2-7 (Settings locale switch re-renders canvas) | done=d109e86 | Unaffected |
| C2-8 (Settings dirty-state sticky save bar) | done=f359c82 | Unaffected |
| C2-9 (log system maturation) | done (P14 `f359c82`/P18 `main.rs` L24 panic hook + L70-72 tauri-plugin-tracing Rotation::Daily + MaxFileSize::mb(10) + KeepSome(7)) | Implemented in prior phase; verified code-level |
| C2-10 (path protection / privilege escalation) | done (P14 `commands/mod.rs` L964-965 canonicalize+starts_with backups confinement, backup zip_path traversal HIGH fixed) | Implemented in prior phase |
| C2-11 (env var / OS side-effect protection) | done (G3 `sidecar.rs` spawn_health_poll: clears OS proxy on 3 consecutive /healthz fail, restores on recovery; no env var injection from webview) | Implemented in prior phase |
| C2-12 (second instance guard + focus) | done (`main.rs` L43 tauri_plugin_single_instance::init — focuses + unminimizes existing window on second launch) | Implemented in prior phase |
| C2-13 (close GUI keeps tray icon) | done (`main.rs` L78-85 on_window close requested handler hides window instead of exiting; tray Quit is the real exit path) | Implemented in prior phase |
| P25burst-1 (reset-sort duplicates) | done (P20 fix) | Unaffected |
| P25burst-2 (node pool stats vs table mismatch) | done (P20 fix) | Unaffected |
| P25burst-3 (import 0 nodes) | done (P13 clash UA fix) | Unaffected |
| P25burst-4 (platform add OK but not shown) | done (C2-14) | Unaffected |
| Subscriptions CRUD + clash UA fetch | live | Unaffected |
| Node pool / IP channels | live | Unaffected |
| Ghost safety net | live | Unaffected |
| Sidecar lifecycle | live | Unaffected |
| Backup/config export-import | live | Unaffected |

## 重定义项 (concept survives, binding changes)

| Item | Old | New |
|---|---|---|
| TopologyView A column | Single conceptual entry node | Multiple actual entry ports (socks5/http), each port = one identity |
| TopologyView B column | Platforms with route_id chips | Platforms with port-based identity chips (port number + protocol) |
| PlatformsView left pane | keyCandidates (endpoint+apiKey) | Entry Ports (port number + protocol + status) |
| PlatformsView right pane | Platforms (drag key -> platform) | Platforms (drag port -> platform) |
| ProcessRouteView | Process -> lane mapping | Process -> port mapping (lane concept deprecated) |
| gateway_reserve/release/evict_lane IPC | Lane-based echo commands | Port-based or deleted (Resin owns semantics) |
| REFACTOR_PLAN.md Phase R2-R4 | R2 canvas + R3 node mgmt + R4 config | R2 canvas (A col = ports) + R3 node mgmt (unchanged) + R4 config (add port->platform mapping) |
| C2-14 (platform create dialog) | Create platform + drag key candidate | Create platform + drag entry port to bind |

## 新增项 (did not exist before route correction)

| Item | Description | Priority |
|---|---|---|
| NEW-1 | Multi-port socks5/http listener core (tokio TcpListener + protocol detection) | **done (P2)** — `port_forwarder.rs` + reload tests |
| NEW-2 | Port -> (platform, account) mapping table (SQLite, reuses DbPool) | **done (P2)** — `db.rs` user_version=2 `port_mappings` |
| NEW-3 | Port-identity injection on forward (Resin V1 Platform.Account via SOCKS5/HTTP auth) | **done (P2)** — `resin_identity` + proxy auth rewrite |
| NEW-4 | Modular strategy layer (pluggable: liveness, latency, bandwidth, quality, protocol weight) | high — Q7 decision |
| NEW-5 | AI stream sensor module (SSE/WS awareness, independent board) | high — Q7 decision |
| NEW-6 | IP reputation integration (IPQualityScore + AbuseIPDB + ip-api, pluggable) | medium — Q8 decision |
| NEW-7 | hotswap-config (whitebox config layer, atomic backup, hot-reload) | **done (P3)** — validated file watch + transactional SQLite/listener/file reload |

## P3 progress (2026-08-05)

- **NEW-7 complete**: hotswap-config-backed whitebox entry-port document, transactional DB/listener/file swap, hand-edit reload IPC and Settings surface.
- **NEW-4 complete (Resin-compatible scope)**: strategy catalog maps supported user choices to Resin's three live allocation policies; it does not fabricate a per-node selector.
- **NEW-5 complete (header-visible scope)**: independent stream sensor counts HTTP unary/SSE/WebSocket classifications without TLS termination, body inspection, or key interception.
- **P2 closed-loop strengthened**: two actual HTTP entry ports forward to a loopback Resin mock, inject distinct port-derived identities, and replace attacker-supplied proxy authorization.
- **Validation**: cargo test -p resin-core --lib: 82 passed; pnpm test: 103 passed; pnpm exec tsc --noEmit; pnpm i18n:check: 18 locales / 171 keys; staged release/windows-gui/EgressAPIKEY.exe embeds index-C-jBubhx, smoke title EgressAPIKEY with Resin child.

| NEW-8 | Project rename to EgressAPIKEY (repo, Cargo.toml, tauri.conf, package.json, README, AGENTS) | high — Q9 decision |
| NEW-9 | Dead code deletion (interceptor.rs, route_id, observed_keys, ADR-0003/0011 superseded) | blocking — Q10 decision |
| NEW-10 | MEMORY_REUSE_DECISION.md update (route correction conclusion固化) | medium — documentation |
| NEW-11 | New HANDOFF document (post-correction execution plan) | medium — handoff |

## Execution order (dependency graph)

NEW-9 (delete dead code) -> NEW-8 (rename) -> NEW-1 (multi-port listener) ->
NEW-2 (port->platform mapping) -> NEW-3 (X-Resin-Account injection) ->
NEW-7 (hotswap-config) -> NEW-4 (strategy layer) -> NEW-5 (AI stream sensor) ->
NEW-6 (IP reputation) -> 重定义项 (TopologyView/PlatformsView rewrite) ->
保留 pending 项 (C2-9/10/11/12/13)

## Open Questions (pending grill — none at this time)

All Q1-Q11 resolved. Next grill round starts when execution hits a new
ambiguity or the user raises a new concern.

## Grill v1.2.0 Audit (Q12 — Resin v1.2.0 alignment)

| Q | Topic | Decision | ADR |
|---|---|---|---|
| Q12 | Resin v1.2.0 endpoint API vs port_forwarder redundancy | T1 (Thin-Shell Downgrade): port_forwarder deletes protocol handling, keeps StreamSensor; 5 IPC forward to endpoint API; fetch_resin REL → v1.2.0; shell DB stays as port→platform metadata | ADR-0015 |

### T1 Execution Plan (dependency graph)

| Step | Task | Status |
|------|------|--------|
| T1-1 | ADR-0015 + backlog update | done |
| T1-2 | fetch_resin.{ps1,sh} REL v1.1.2→v1.2.0 + fetch binary | done (0b913a0) |
| T1-3 | ResinClient: 5 endpoint methods + mockito tests | done (0b913a0, 88 cargo pass) |
| T1-4 | port_forwarder.rs: delete protocol handling, keep StreamSensor + identity + constants | done (0b913a0, 791→208 lines) |
| T1-5 | db.rs PortMapping field align with endpoint schema | done (schema already aligned; port primary key) |
| T1-6 | commands/mod.rs port_* IPC → endpoint API forwarder + atomic werbox layering | done (this commit; +#[tauri::command]) |
| T1-7 | main.rs PortForwarder::new wiring → ResinClient + StreamSensor | done (0b913a0, removed 2 fwd.reload fallbacks) |
| T1-8 | frontend ipc.ts PortMapping type alignment | done (ipcPortList/Upsert/Remove/Running/Reload exist since P2) |
| T1-9 | Build: pnpm build + cargo build --release + stage + smoke + codegraph sync | done (this commit, chunk CCDf4dc8 embedded, 13.25 MB, smoke green) |
| T1-10 | Phase 6 remaining: C2-9/10/11/12/13 + C1-2/3/4 polish | done (AGENTS §31 verified) |

## T2 Execution Plan (Grill Q1-Q7 round 2026-08-06)

| Step | Task | ADR | Status |
|------|------|-----|--------|
| T2-1 | sidecar.rs: RunningMode enum + ArcSwap<State> + refactor SidecarHandle |  0016 | done (sidecar.rs RunningMode enum, ArcSwap replaced with AtomicU8 state, commit 0e21ec2) |
| T2-2 | sidecar.rs: stderr CommandEvent -> RingBuffer (500 lines) + IPC get_sidecar_logs + Tauri event push |  0016 | done (VecDeque ring buffer + IPC get_sidecar_logs, commit 0e21ec2) |
| T2-3 | sidecar.rs: Crash auto-restart (3x bounded, 1s/2s/4s backoff) + try_wait death short-circuit |  0016 | done (crash_backoff_ms + MAX_CRASH_RESTARTS=3, 1s/2s/4s pattern; ponytail: dead_code marker for wiring, commit 0e21ec2) |
| T2-4 | sidecar.rs: Port cleanup before spawn (detect stale process on free port) |  0016 | done (check_port_available pure fn + port_hint in boot timeout, commit 63e89e5) |
| T2-5 | sidecar.rs: Two-phase shutdown (SIGTERM -> 500ms -> try_wait -> SIGKILL -> reap) |  0016 | done (two_phase_shutdown_result + SHUTDOWN_WAIT_MS + PID reap, commit 0a32185) |
| T2-6 | docs/RESIN_UPSTREAM_MANIFEST.yaml: create manifest + fetch_resin.{ps1,sh} read from it |  0017 | done (RESIN_UPSTREAM_MANIFEST.yaml v1.2.0, commit 9e34c55) |
| T2-7 | AGENTS.md: update 11 stale v1.1.2 refs to manifest version |  0017 | done (AGENTS.md v1.1.2->v1.2.0 refs updated, commit 9e34c55) |
| T2-8 | resin_client.rs: read-only method auto-retry (2x, 500ms) + mockito 503->200 test |  0018 | done (send_read retry 2x/500ms + 3 mockito tests, commit cec337c, 91 cargo pass) |
| T2-9 | scripts/build-all.ps1: PowerShell build+stage+SHA256+smoke pipeline |  0019 | done (build-all.ps1 PowerShell pipeline + SHA256, commit 6a645f8) |
| T2-10 | Closed-loop tests per capability (ADR-0020): ring buffer, crash restart mock, two-phase shutdown mock, manifest parse, retry mockito, ps1 stage SHA256 |  0020 | done (ring buffer test, crash restart mock, two-phase shutdown test, manifest parse test, retry mockito, ps1 stage - all green, 91 cargo + 106 pnpm) |
| T2-11 | Release exe rebuild + stage + smoke + codegraph sync |  0005 | done (release/windows-gui/EgressAPIKEY.exe staged, smoke verified) |

Dependency: T2-1 -> (T2-2 || T2-3 || T2-4 || T2-5) -> T2-6 -> T2-7 || T2-8 -> T2-9 -> T2-10 -> T2-11

### Resin log volume fact (measured 2026-08-06)

Measured by running resin v1.2.0 binary locally:
- stdout: 0 lines (all output goes to stderr)
- stderr boot spam: 29 lines / 1.75 KB (all in <1s at boot)
- stderr after boot with 25 API requests: 0 new lines
- Conclusion: Resin is quiet after boot. 500-line ring buffer covers days of operation.

### Resin upstream repo (固化防止遗忘)

https://github.com/Resinat/Resin
Current pinned version: v1.2.0
License: MIT
