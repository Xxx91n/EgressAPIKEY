# ADR-0045: IPC Error Contract Hardening — IpcError Full Wire + ESLint Guard + Toast Action + Full Tests

## Status
ACCEPTED

## Date
2026-08-18

## Context
Grill T20决策链:
- Q1=A: 全量IpcError重构 (64个command, residual 3+1)
- Q2=A: 预防lint + fault-tolerant (translateError已实现, ESLint rule待建)
- Q3=A: toast action button全量variant (BindConflict→换端口, InvalidStrategy→文档, ResinUpstream→重试)
- Q4=A: 全量translateError + lint守护 (核实发现13个catch已全部用translateError, 实际只需lint rule)
- Q5=A: 全量测试覆盖 (14 cargo + 6 vitest = 20测试)

## Decision
1. **IpcError全量wire**: 3个residual `Result<_, String>` command + 1 helper改为 `Result<_, IpcError>`. IpcError enum (BindConflict/InvalidStrategy/ResinUpstream/Internal) serde externally-tagged, 每个variant带i18n_key.
2. **ESLint no-raw-error-in-toast**: 自定义rule扫描catch block, 禁止raw `e`传入toast/display函数. vitest AST扫描守护防未来回归.
3. **Toast action button**: `showToast(kind, msg, action?)` 第三参数. BindConflict→ipcPortSuggest换端口, InvalidStrategy→查看策略文档, ResinUpstream→重试. Internal→无action.
4. **Full test coverage**: cargo端map_resin_error 13+branch + extract_port_from_residual边界 + IpcError 4 variant serde round-trip. vitest端view级translateError闭环测试.

## Consequences
- 所有IPC error通过typed IpcError到达前端, 前端按variant走不同i18n消息 + UI恢复路径
- ESLint rule防未来新catch block漏走translateError
- Toast action button让用户能直接从错误恢复 (换端口/重试/看文档)
- 20+测试覆盖每个variant + 每种边界条件
- 代价: ~40行Rust机械改动 + ~50行ESLint rule + ~70行toast action + ~200行测试 = ~360行

## Alternatives considered
- Q1 B: 只修bind-conflict路径 — 拒绝, 用户要求全量
- Q2 B: 只加lint不改现有 — 拒绝, 但核实发现现有已全部用translateError
- Q3 B: 只加BindConflict action — 拒绝, 用户要求全量variant
- Q4 B: 只加lint守护 — 拒绝, 但实际工作就是lint only (前提修正后)
- Q5 B: 只覆盖高风险分支 — 拒绝, 用户要求全量
