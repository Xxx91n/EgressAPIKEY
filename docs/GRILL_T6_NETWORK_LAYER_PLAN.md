# GRILL T6: Network Layer — Grill Decisions & Execution Plan

Date: 2026-08-12
Status: GRILL COMPLETE — awaiting user "start building" instruction
Grill round: T6 (Q1-Q5)

## Grill Decisions Summary

| Q | Topic | Decision | Resin fork? | Shell-side | Core-side |
|---|-------|----------|-------------|------------|-----------|
| Q1 | DNS configuration exposure | A: whitebox JSON add dns_upstreams[] -> sidecar.rs inject RESIN_NODE_DNS_UPSTREAMS -> GUI Settings DNS editor + "reset to default" | No | Yes | No |
| Q2 | Network-layer whitebox config | A: extend WhiteboxConfig add network{dns_upstreams, max_idle_conns, max_idle_conns_per_host, idle_conn_timeout, probe_timeout, probe_concurrency, proxy_bypass} -> sidecar.rs inject -> Settings "Network Layer" card | No | Yes | No |
| Q3 | End-to-end test tool | C: dual-layer — unit test (mockito local echo server) + integration test (#[ignore] real 1.1.1.1 via sidecar) + GUI "Diagnostics" button (port_health_check + real exit IP probe to http://1.1.1.1/cdn-cgi/trace) | No | Yes | No |
| Q4 | Network-layer gaps coverage | Shell does #1 (Windows firewall detect) + #2 (GUI read Resin API show 0-nodes-available) + #4 (IPC read Resin request_logs DB show exit IP per request). #3 YAGNI. #5/#6/#7/#8 already covered by Resin natively or Q1/Q2. | No | Yes | No |
| Q5 | Stability + IPC + latency | B: A (gap fill: IpcError ResinUpstream full wiring + sidecar restart banner + port rebind sync) + B (network diagnostics GUI panel: sidecar PID/port/healthz/IPC latency/request log tail) | No | Yes | No |

## Codebase Audit Facts (verified via ctx_execute + ctx_batch_execute)

### Resin v1.2.0 already provides (NO fork needed)
- RESIN_NODE_DNS_UPSTREAMS: DoH failover chain (4 defaults), envStringSlice
- RESIN_PROXY_TRANSPORT_MAX_IDLE_CONNS (1024), MAX_IDLE_CONNS_PER_HOST (64), IDLE_CONN_TIMEOUT (90s)
- RESIN_PROXY_BYPASS: proxy bypass rules (envDelimitedStringSlice)
- RESIN_PROBE_TIMEOUT (15s), RESIN_PROBE_CONCURRENCY (1000)
- RESIN_RESOURCE_FETCH_TIMEOUT (30s)
- RESIN_DEFAULT_PLATFORM_STICKY_TTL (7d)
- RESIN_GEOIP_UPDATE_SCHEDULE ("0 7 * * *")
- Request log system: request_logs SQLite DB with egress_ip field, auto-rotation, auto-cleanup, memory queue
- Lease persistence: leases table in state.db, survives restart
- 503 + X-Resin-Error: NO_AVAILABLE_NODES when all exit nodes fail
- IPv6 support in core

### Shell current state (verified)
- sidecar.rs injects 7 RESIN_* env vars (ADMIN_TOKEN, PROXY_TOKEN, AUTH_VERSION, LISTEN_ADDRESS, PORT, STATE_DIR, CACHE_DIR, LOG_DIR)
- whitebox_config.rs: WhiteboxConfig{version, entry_ports} — only ports, no network/DNS
- port_health_check IPC: TCP connect + SOCKS5 greeting / HTTP GET probe, has 1 cargo test
- resin_client.rs: 20 async methods (platform/subscription/node/endpoint CRUD + leases + pool snapshot)
- IpcError enum exists (BindConflict, InvalidStrategy, ResinUpstream, Internal) — NOT fully wired to all 44 commands (P5 leftover)
- sidecar-status event exists (healthy/unhealthy) — no "restarting" state
- Crash restart with bounded backoff (3x, 1s/2s/4s) — implemented
- Orphan kill on RunEvent::Exit — P22 fix, implemented
- Shell DB (egressapikey.db) and Resin DB (state.db/cache.db) separate — no SQLite lock contention

### Enterprise template references (pwm pro gpt56_sol)
- clash-verge-rev: whitebox YAML exposes dns.enable/listen/enhanced-mode/fake-ip/nameserver/fallback
- sing-box: JSON dns.servers/rules/strategy
- Industry pattern: curl --socks5-hostname 127.0.0.1:PORT http://1.1.1.1/cdn-cgi/trace for e2e proxy test
- Tauri sidecar localhost REST: 1-5ms latency, control-plane only, no data-plane impact
- clash-verge-rev shell-to-core: REST + event-driven (not gRPC)

## Execution Plan (dependency graph)

T6-1 (WhiteboxConfig network + DNS extension)
  -> T6-2 (sidecar.rs env injection)
  -> T6-3 (GUI Settings network + DNS card)
  -> T6-4 (E2E test: unit mockito + integration #[ignore] + GUI diagnostics button) [parallel with T6-5]
  -> T6-5 (Windows firewall detect + 0-nodes banner + request log tail IPC) [parallel with T6-4]
  -> T6-6 (IpcError ResinUpstream full wiring + restart banner + port rebind sync)
  -> T6-7 (Network diagnostics GUI panel: sidecar PID/port/healthz/IPC latency/request log tail)
  -> T6-8 (Closed-loop tests: cargo + vitest per capability)
  -> T6-9 (Build + smoke + codegraph sync + push)

## Item Detail

### T6-1: WhiteboxConfig network + DNS extension
- File: crates/resin-core/src/whitebox_config.rs
- Add to WhiteboxConfig struct: pub network: NetworkConfig (serde default)
- New struct NetworkConfig with dns_upstreams, max_idle_conns, max_idle_conns_per_host, idle_conn_timeout_secs, probe_timeout_secs, probe_concurrency, proxy_bypass
- validate(): dns_upstreams non-empty if set, max_idle_conns >= 1, probe_concurrency 1..=10000
- Test: cargo test whitebox_config network validation
- ADR: new ADR-0028

### T6-2: sidecar.rs env injection
- File: src-tauri/src/sidecar.rs
- After RESIN_LOG_DIR, read WhiteboxConfig network section and inject 7 env vars conditionally
- Test: cargo test sidecar env injection conditions

### T6-3: GUI Settings network + DNS card
- File: src/views/SettingsView.tsx
- New card "Network Layer" with editable fields + "Reset to Default" button
- i18n: network.* keys across 18 locales
- Test: vitest SettingsView network card renders + save round-trip

### T6-4: E2E test tool
- Rust unit test: crates/resin-core/tests/proxy_e2e.rs with mockito local echo server
- Rust integration test: #[ignore] test_probe_real_1_1_1_1 via sidecar
- GUI: Diagnostics button calling ipcProbeExitIp -> shows exit IP + latency
- New IPC: probe_exit_ip(port, protocol) -> reqwest through proxy to http://1.1.1.1/cdn-cgi/trace
- Test: cargo test proxy_e2e + vitest diagnostics button

### T6-5: Windows firewall + 0-nodes banner + request log tail
- Windows firewall: new IPC check_firewall_status() -> PowerShell Get-NetFirewallProfile (read-only)
- 0-nodes banner: TopologyView reads ipcNodePoolSnapshot, red banner if 0 healthy nodes
- Request log tail: new IPC request_log_tail(limit) -> reads Resin request_logs DB
- Test: cargo test request_log_tail + vitest 0-nodes banner

### T6-6: IpcError full wiring + restart banner + port rebind sync
- IpcError ResinUpstream: wire to remaining commands (P5 leftover)
- Restart banner: sidecar-status event add "restarting" payload
- Port rebind sync: after restart, re-fetch port list, sync port_mappings
- Test: cargo test IpcError variants + vitest restart banner

### T6-7: Network diagnostics GUI panel
- New view or Settings sub-section "Diagnostics"
- Shows: sidecar PID, API port, healthz last check, IPC latency, request log tail
- Test: vitest diagnostics panel renders + data refresh

### T6-8: Closed-loop tests per capability
- cargo + vitest per item + i18n check + tsc green

### T6-9: Build + smoke + sync
- pnpm build -> cargo build --release -> grep chunk hash -> smoke -> codegraph sync -> push

## Constraints
- Ponytail full: smallest diff, root cause not symptom, deletion over addition
- Every platform (cargo + vitest) must have test closed-loop
- pwm pro gpt56 for technical detail research (avoid hallucination)
- No fork Resin — all shell-side
- i18n: 18 locales, all new keys
- AGENTS.md sections 5/6/7: build + push + CRLF per commit
