# AGENTS.cli.md — CLI/IPC Surface Contract for EgressAPIKEY

> Version: nightly audit 2026-08-11 · Status: aligned · Branch codex/rust-port ee4ceb7+

## IPC commands registered in main.rs invoke_handler (45)

| Command | Rust fn | TS wrapper | Validation | Risk | Notes |
|---|---|---|---|---|---|
| gateway_snapshot | async fn | ipcGatewaySnapshot | None required | Low | Read-only Resin /metrics fetch |
| tray_refresh_labels | pub fn | invoke("tray_refresh_labels") | None required | Low | Updates tray i18n labels |
| get_config_dir | pub fn | (direct invoke) | None required | Low | Returns app_config_dir path |
| get_log_dir | pub fn | (direct invoke) | None required | Low | Returns app_log_dir path |
| get_sidecar_logs | pub async fn | (direct invoke) | None required | Low | Reads sidecar log lines |
| get_sidecar_status | pub async fn | ipcGetSidecarStatus | None required | Low | Read-only sidecar health check |
| platform_add | pub async fn | ipcPlatformAdd | validate_short_name | Low | POST /api/v1/platforms |
| platform_remove | pub async fn | ipcPlatformRemove | List-match by name | Low | DELETE /platforms/{id}; cannot delete Default (Resin returns 409) |
| platform_list | pub async fn | ipcPlatformList | None required | Low | GET /platforms |
| platform_list_full | pub async fn | ipcPlatformListFull | None required | Low | GET /platforms returning full objects |
| platform_snapshot | pub async fn | ipcPlatformSnapshot | validate_short_name | Low | Per-platform snapshot echo |
| platform_update | pub async fn | ipcPlatformUpdate | allocation_policy enum check, 64-entry filter cap, 253-char cap | Med | PATCH /platforms/{id}; allocation_policy strictly validated to BALANCED|PREFER_LOW_LATENCY|PREFER_IDLE_IP |
| platform_create_with_fields | pub async fn | ipcPlatformCreateWithFields | validate_short_name + allocation_policy enum + 64-entry regionFilters + 253-char regex entry + stickyTtl 32-char | Med | POST /platforms with full schema; bodies flow through to Resin without modification |
| platform_leases | pub async fn | ipcPlatformLeases | validate_short_name | Low | GET /platforms/{id}/leases via ResinClient |
| account_add | pub async fn | (none) | validate_short_name for platform, assertAuthority(account) | Med | Currently echo; Account concept abolished by ADR-0012 |
| account_bind_ip | async fn | ipcAccountBindIp | assertIp, validate_short_name(platform), validate_short_name(account) | Med | Currently echo |
| process_route_add | pub async fn | ipcProcessRouteAdd | validate name 1..128, no control; target_port range check via validate_port_mapping-like | Med | Persisted via tauri-plugin-store; no live enforcement on Resin |
| process_route_remove | pub async fn | ipcProcessRouteRemove | None required | Low | Removes from tauri-plugin-store |
| process_route_list | pub async fn | ipcProcessRouteList | None required | Low | Reads tauri-plugin-store |
| subscription_add | pub async fn | ipcSubscriptionAdd | URL http(s):// prefix check + length cap 2048 + name validate_short_name | Med | Internal clash-UA fetch + flow-to-block convert + POST /subscriptions |
| subscription_remove | pub async fn | ipcSubscriptionRemove | validate_short_name | Low | DELETE /subscriptions/{id} after list-match |
| subscription_list | pub async fn | ipcSubscriptionList | None required | Low | GET /subscriptions |
| node_pool_snapshot | pub async fn | ipcNodePoolSnapshot | None required | Low | GET /metrics/snapshots/node-pool |
| node_list | pub async fn | ipcNodeList | None required | Low | GET /nodes?limit=500 |
| backup_create | pub async fn | ipcBackupCreate | backups_dir confinement already automatic | Med | Creates ZIP with crypto-random suffix inside app_data/backups; reads settings.json + resin-state DB |
| backup_upload | pub async fn | ipcBackupUpload | URL http(s):// required + canonicalize+starts_with path traversal guard + zip_path must be inside app_data/backups | HIGH fenced | WebDAV upload of pre-existing zip; path traversal closed by P14 fix |
| backup_list | pub async fn | ipcBackupList | URL http(s):// required | Low | WebDAV PROPFIND listing only |
| config_export | pub async fn | ipcConfigExport | None required | Low | Read-only export of user settings JSON |
| config_import | pub async fn | ipcConfigImport | length cap 256KB JSON; auto-backup before apply | Med | Imports JSON config; no zip extraction; no shell pip invokes |
| lease_map | pub async fn | ipcLeaseMap | None required | Low | GET /metrics/realtime/leases reshape |
| ip_reputation_snapshot | pub async fn | ipcIpReputationSnapshot | None required | Low | External API call (per host reputation scoring) |
| port_list | pub async fn | ipcPortList | None required | Low | Reads port_mappings DB |
| port_upsert | pub async fn | ipcPortUpsert | validate_port_mapping (MIN_USER_PORT>=1024 check, protocol enum, name caps, label cap 128) | HIGH under guard | Resin endpoint CRUD; may bind a TCP; uses account field as part of SOCKS5 username {Platform}.{account} |
| port_remove | pub async fn | ipcPortRemove | port >= MIN_USER_PORT (no privileged delete) | Med | Resin endpoint DELETE + DB row removal |
| port_running | pub async fn | ipcPortRunning | None required | Low | Reads forwarder live state |
| port_reload | pub async fn | ipcPortReload | None required | Low | Live whitebox reload |
| port_auth_info | pub async fn | ipcPortAuthInfo | validate_port_segments | Med | Resolves {Platform}.{account} + proxy_token for SOCKS5 auth UI |
| port_health_check | pub async fn | ipcPortHealthCheck | validate_port_segments + protocol enum (http|socks5) | Med | TCP probe + protocol-aware greeting |
| whitebox_path | pub async fn | ipcWhiteboxPath | None required | Low | Returns path to whitebox config file |
| whitebox_get | pub async fn | ipcWhiteboxGet | None required | Low | Reads whitebox port-plan JSON |
| whitebox_reload | pub async fn | ipcWhiteboxReload | None required | Low | Live reload from DB + file |
| stream_sensor_snapshot | pub async fn | ipcStreamSensorSnapshot | None required | Low | Read-only stream sensor stats |
| strategy_config_get | pub async fn | ipcStrategyConfigGet | None required | Low | Reads JSON config file |
| strategy_config_put | pub async fn | ipcStrategyConfigPut | 1..128 char platform_name, region list <=64, subscription list <=64, top_n <= 1000 | Med | Writes egressapikey-strategy.json |
| strategy_apply | pub async fn | ipcStrategyApply | None required | Med | Applies A-class + B-class strategy to live Resin via PATCH region_filters |

## CLI/IPC safety rules (every change MUST honor)

1. No webview-supplied raw string reaches ResinClient construction. The
   admin_token and proxy_token stay Rust-side; webview reaches Resin via
   the ipc wrapper above only. Re8 audit: SSRF guard stays in resin_client.rs/mihomo.rs.

2. proxy_token is now always non-empty (T8 revert of ADR-0027). The
   port_auth_info command returns the credential; do NOT re-introduce an
   empty token path because SOCKS5 negotiation forcing on
   require_proxy_auth_info=true endpoints breaks the handshake. See
   sidecar.rs L264 for the ONE source of truth.

3. Every new #[tauri::command] that takes user input MUST call
   validate_short_name / validate_port_mapping / validate_port_segments
   BEFORE locking State or reaching ResinClient. Read AGENTS.md §7.5.

4. every change that touches src/, src-tauri/, crates/ MUST be closed with
   the full verify chain per AGENTS.md §5: pnpm build + cargo build --release
   + stage exe to release/windows-gui/ + grep Vite chunk hash in exe bytes + smoke
   launch. A pushed source-only commit is NOT "done" — the user runs the exe.

5. i18n 18 locales must stay in lockstep with canonical en catalog.
   pnpm i18n:check must pass. Adding a string means adding the key to ALL 18
   base locales in the same commit.

6. Local code edits use ctx_batch_execute / ctx_execute_file (Node fs) per
   AGENTS.md "File edits must go through ctx (MANDATORY override)". Never
   use apply_patch for content edits. Never use sed -i / perl -pi / redirection
   for whole-file rewrite.

7. Tests must close the loop. AGENTS.md §4: "New behavior without a test is
   blocked." For Rust modules add cargo #[test] in same file; for TS state
   add vitest in sibling .test.ts(x). Non-trivial logic = one runnable check.

8.\$/expect: RwLock unwrap in sidecar.rs is fine (RwLock never poisons);
   Mutex unwrap inside SidecarHandle.child uses .take() + .ok() so a poisoned
   mutex on panic does not blow up the exit path. Do NOT add raw .unwrap()
   on webview-supplied data without defense-in-depth.

9. Backup zip path is confined via canonicalize + starts_with to
   app_data_dir/backups. Crypto-random suffix for predictable-path guessing
   prevention. Reject ../ traversal — see backup_upload at mod.rs L980-993.

10. Sidecar shutdown goes through main.rs RunEvent::Exit -> child.lock().ok()
    + take() + .kill(); the two-phase shutdown logic in sidecar.rs
    two_phase_shutdown_result is unit-tested for all three states (killed
    + dead = ok; not-killed = err; killed + alive = err).

11. State sync across GUI/CLI: GUI components belong to the local appStore +
    useEffect; the Rust side is the single source of truth for the sidecar
    state. There is NO live push from Rust to GUI except for: Tauri events
    "sidecar-status" (payload string "healthy"/"unhealthy") and the invoke
    reply path. Do not wire a second event channel.

12. ADR-0012 route correction (thin shell multi-port) is the architecture
    truth. ROUTE_ID, observed_keys SQLite lane hashes, interceptor.rs are
    SUPERSEDED / deleted. Do not reintroduce them under a new name without
    re-opening ADR-0012.

13. The 2357-line commands/mod.rs file is the IPC layer boundary; if it
    grows past 3000 lines, split into submodules (commands/platforms.rs,
    commands/ports.rs etc.) and re-export via commands/mod.rs. For now the
    file is large but the validation helpers + types are all colocated which
    Ponytail accepts.
