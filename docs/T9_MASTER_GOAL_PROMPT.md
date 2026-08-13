# T9 Canvas Fix — Master Goal Prompt

遵循以上 grill 构造的所有内容，设定目标，开始大型构建，根据 grill 路线图一次性完成后续安排的所有内容。

始终遵循 AGENTS.md，加载并使用 ctx_*插件，必要时用 1mcp 的 exa/perplexity 联网搜索，不要产生幻觉推理。Ponytail full 模式。
[$ponytail:ponytail](C:\\Users\\Administrator\\.codex\\plugins\\cache\\ponytail\\ponytail\\4.8.4\\skills\\ponytail\\SKILL.md)
[$ultragoal](C:\\Users\\Administrator\\.codex\\skills\\ultragoal\\SKILL.md) 设定目标以防止丢失
以下功能我都我希望你能联网调研出一个类似功能的模板轮子出来，而不是自己开发
细分好先后顺序和验收标准和test闭环，避免只引入但是却没有做到的情况。

请保证以后每次开发过程中涉及细节技术都pwm pro搜索调研，避免幻觉，而且每个平台都要有test闭环。
使用pwm pro 中的gpt56搜索，调研行业内工业级别成熟的模板，工程级别。

## T9 Canvas Fix — COMPLETED (commit c45334c)

### Phase Decision Summary
| Q | Decision | Status |
|---|----------|--------|
| Q1 | C — Subscription-folded C column (collapsible) | DONE |
| Q2 | A — Full-area Handle (drag-to-connect hits card anywhere) | DONE |
| Q3 | A — Custom CanvasControls with i18n tooltips | DONE |
| Q4 | A — Dual A+B strategy badges on platform nodes | DONE |
| Q5 | C — Edge labels show A-class strategy source only | DONE |

### Test Closure
- 181 vitest (14 files) — all green
- 118 cargo (resin-core) — all green
- tsc — green (0 errors)
- i18n:check — 309 keys x 18 locales, all aligned
- Vite build — green, chunk hash BShEnGn1
- Release exe — 12.95 MB, chunk hash verified in exe bytes
- codegraph synced

### Files Changed (21 files, +705 -522)
- src/views/TopologyView.tsx — full rewrite (646 lines)
- src/views/TopologyView.test.tsx — updated tests (24 tests)
- src/locales/*/common.json — 5 new topology keys x 18 locales
- docs/GRILL_T9_CANVAS_FIX_PLAN.md — plan document
