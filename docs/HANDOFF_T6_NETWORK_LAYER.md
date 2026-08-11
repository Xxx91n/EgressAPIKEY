# T6 Handoff: Network Layer Build

Date: 2026-08-12
Status: ACTIVE — goal set, execution starting

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
