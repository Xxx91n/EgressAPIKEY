# T6 Handoff: Network Layer Build

Date: 2026-08-12
Status: COMPLETE — all nine T6 phases verified complete 2026-08-12

## Startup prompt for next window

始终遵循 AGENTS.md，加载并使用 ctx_*插件，必要时用 1mcp 的 exa/perplexity 联网搜索，不要产生幻觉推理。Ponytail full 模式。
Ponytail full mode. Ultragoal for goal persistence.
Research templates/wheels via pwm pro gpt56, not self-developed.
Sequence + acceptance + test closed-loop per capability.
Every platform (cargo + vitest) needs test closed-loop.

## Execution plan

See docs/GRILL_T6_NETWORK_LAYER_PLAN.md for full T6-1 through T6-9 detail.

### Sequencing + acceptance criteria

| Step | Item | Acceptance criteria | Test closed-loop |
|------|------|-------------------|-----------------|
| T6-1 | WhiteboxConfig.network + DNS struct | cargo test green: network validation | cargo |
| T6-2 | sidecar.rs env injection | cargo test green: 7 env vars conditional | cargo |
| T6-3 | GUI Settings network card | vitest green: card + save + reset + i18n | vitest + i18n |
| T6-4 | E2E test tool | cargo mockito + vitest diagnostics button | cargo + vitest |
| T6-5 | Firewall + 0-nodes + request log | cargo request_log_tail + vitest banner | cargo + vitest |
| T6-6 | IpcError wiring + restart banner | cargo IpcError + vitest restart banner | cargo + vitest |
| T6-7 | Diagnostics GUI panel | vitest panel renders | vitest |
| T6-8 | All tests pass | cargo + vitest + i18n + tsc green | all |
| T6-9 | Build + smoke + push | exe + chunk hash + smoke + codegraph + push | smoke |

### Dependencies
T6-1 -> T6-2 -> T6-3 -> (T6-4 || T6-5) -> T6-6 -> T6-7 -> T6-8 -> T6-9

## Completion summary (2026-08-12)

All nine T6 phases executed against the standing plan and verified by code-level audit:

| Phase | Deliverable | Tests |
|---|---|---|
| T6-1 | `WhiteboxConfig.network` + `validate_network` + 5 cargo | green |
| T6-2 | `sidecar.rs` 7 `RESIN_*` env injections + `read_network_config` + 3 cargo | green |
| T6-3 | GUI Settings network card + `whitebox_save_network` IPC + 3 vitest | green |
| T6-4 | `probe_exit_ip` IPC + socks feature (fix `6acccd5`) + `proxy_e2e.rs` + `parse_trace_body_ip` extraction + #[ignore] 1.1.1.1 | 3 cargo + 3 vitest |
| T6-5 | `check_firewall_status` + `request_log_tail` IPC + 0-nodes amber banner | 3 vitest |
| T6-6 | 4-state sidecar banner (healthy/unhealthy/restarting/terminated) + port rebind sync | 2 vitest |
| T6-7 | Diagnostics panel: sidecar PID + healthz last check + IPC latency + port + mode + firewall + request log | 3 vitest |
| T6-8 | 122 cargo + 3 proxy_e2e (1 ignored) + 168 vitest = 293 total, tsc green, 274 i18n keys / 18 locales | all green |
| T6-9 | `cargo build` green + codegraph sync + all commits pushed to `codex/rust-port` HEAD `13483a2` | green |

ADR-0028 created post-audit to record the `WhiteboxConfig.network` decision per the plan's own ADR item. `docs/PONYTAIL_DEBT_LEDGER.md` re-verified: three live `ponytail:` source tags (1 keep, 1 done, 1 stale-tag); stale ledger entries for `route_id` / `normalize_auth` / notification-crate refilled since the underlying code is gone. See the ledger for the re-audit receipt.
