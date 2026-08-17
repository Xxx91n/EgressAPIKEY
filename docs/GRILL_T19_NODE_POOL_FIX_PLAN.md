# GRILL T19 — Node Pool Page Fix Plan

> Canon: ADR-0044 (decisions) + CONTEXT.md deltas (4 terms) + this file (5-phase execution).
> Source grill rounds: R1 (Q1-Q4) with 3 atomcode research passes (node-pool-UX / resin-sub-scheduler / probe-params).
> Authority: ADR-0036 strategy-single-source-truth still holds; this plan layers per-node probe + local-subscription refresh on top, no strategy pipeline change.

## Decisions locked

| Q | Topic | Answer |
|---|-------|--------|
| Q1 | Default collapse + hide unhealthy + latency sort + delay> syntax | A — collapse Set seeded after first refresh, hide-unhealthy toggle reusing `isHealthy`, 3-state latency sort reusing `reference_latency_ms`, search supports `delay>100` / `delay=timeout` filters |
| Q2 | Why node pool never updates | A — Path 丙: shell re-fetches remote Clash YAML (UA rotate) + `clash_yaml_to_proxies_block` convert + PATCH subscription content; new `subscription_refresh(name)` IPC + per-group refresh button |
| Q3 | Per-node ping like clash-verge | A — wire to Resin native per-node endpoint: `POST /nodes/{hash}/actions/probe-egress` (egress IP + EWMA latency) and `/actions/probe-latency` (sync). New `ResinClient::probe_node_egress` + `probe_node_latency` + `node_probe(node_hash, kind)` IPC + per-card test button |
| Q4 | Whitebox probe params + default behavior | 甲+丙+default group concurrency probe, params whitebox editable. Default = manual single-probe (甲), per-group batch test button (丙), concurrency=10 / timeout=10s / test_url=`https://www.gstatic.com/generate_204` / batch_on_load=false. All four knobs persisted in `settings.json#nodeProbe` + surfaced in Settings page. Resin hot-update knobs (`max_consecutive_failures`, `max_latency_test_interval`, `max_egress_test_interval`, `latency_test_url`, `latency_authorities`, `p2c_latency_window`, `latency_decay_window`) surfaced via `system_config_get/patch` already in ResinClient. Parameter sources: atomcode research 3 (Resin ProbeManager defaults + clash-verge delayManager). |

## Parameter defaults (sources: atomcode 3)

| Knob | Default | Source | Whitebox surface |
|------|---------|--------|------------------|
| `nodeProbe.concurrency` | 10 | clash-verge-rev hard cap `min(concurrency, n, 10)` | settings.json + Settings panel |
| `nodeProbe.timeout_ms` | 10000 | clash-verge `DEFAULT_DELAY_TIMEOUT = 10000`; shell takes min(10s, Resin RESIN_PROBE_TIMEOUT=15s) |
| `nodeProbe.test_url` | `https://www.gstatic.com/generate_204` | Resin default `latency_test_url` (hot-update via `/system/config`) |
| `nodeProbe.batch_on_load` | false | clash-verge #979 — full auto-ping is community pain point; 甲 = manual by default |
| Resin `max_consecutive_failures` | 3 | Resin default | Settings panel → `/system/config` PATCH |
| Resin `max_latency_test_interval` | 1h | Resin default | Settings panel → `/system/config` PATCH |
| Resin `max_egress_test_interval` | 24h | Resin default | Settings panel → `/system/config` PATCH |
| Resin `latency_authorities` | [gstatic.com, google.com, cloudflare.com, github.com] | Resin default | Settings panel → `/system/config` PATCH |

## Phase execution plan

### Phase 1 — Default collapse + hide-unhealthy + latency sort + delay> filter

**Files**: `src/views/NodesView.tsx` (L84 collapsed Set, L112 filtered useMemo, new sort state, new `hideUnhealthy` toggle, new `parseDelayQuery` helper)

**Work**:
1. `collapsed` Set seeded with every `sub name` after first `refresh()` returns (first paint = all collapsed). Current `new Set()` = all expanded → flip to seed-all.
2. New `hideUnhealthy` boolean state, default false, toggle chip in header. When true, `filtered` excludes nodes where `!isHealthy(n)`.
3. New `sortMode` 3-state: 'default' → 'latency-asc' → 'latency-desc' → cycle. Header sort chip. `filtered` sorts by `reference_latency_ms` (null = Infinity timeout-first in asc, -Infinity in desc).
4. New `parseDelayQuery(q: string)` pure helper. Recognizes `delay>N`, `delay<N`, `delay=timeout`, `delay=error` (case-insensitive). Returns `{ text: string, delayFilter?: (ms) => boolean }`. `filtered` combines text + delay filter.
5. `search` state input + 2 new header chips (hide-unhealthy 🩺, sort ↑/↓/default) + collapse-all/expand-all buttons already exist.

**i18n keys** (18 locales × 6 keys = 108 new entries):
- `nodes.hideUnhealthy` (en: "Hide unhealthy", zh: "隐藏无效节点")
- `nodes.sortDefault` (en: "Sort: default", zh: "排序：默认")
- `nodes.sortLatencyAsc` (en: "Sort: latency ↑", zh: "排序：延迟↑")
- `nodes.sortLatencyDesc` (en: "Sort: latency ↓", zh: "排序：延迟↓")
- `nodes.delayFilterHint` (en: "Use delay>100 or delay=timeout", zh: "可用 delay>100 或 delay=timeout 筛选")
- `nodes.collapseAllDefault` (en: "Groups are collapsed by default", zh: "订阅分组默认折叠")

**Loop test** (vitest, `src/views/NodesView.test.tsx` +5 assertions):
- `parseDelayQuery` pure helper: `delay>100` admits 200, rejects 50; `delay=timeout` admits 9999/null, rejects 200; plain text passes through
- `hideUnhealthy` toggle excludes `failure_count>0` nodes from `filtered`
- 3-state `sortMode` cycle produces ascending / descending order on `reference_latency_ms`
- `collapsed` Set seeded with all sub names after first refresh (integration test against mock refresh)
- `delay=error` filter matches nodes with missing `reference_latency_ms`

**Estimate**: ~110 lines TS, 0 Rust, 0 IPC, 6 i18n keys ×18 locales.

### Phase 2 — Subscription refresh IPC (local re-fetch + PATCH content)

**Files**: `crates/resin-core/src/resin_client.rs` (new `refresh_subscription_content(id, content)` → PATCH `/api/v1/subscriptions/{id}`), `src-tauri/src/commands/mod.rs` (new `subscription_refresh(name)` command: list → match name → id → fetch_clash_subscription(url) → clash_yaml_to_proxies_block → PATCH), `src/lib/ipc.ts` (`ipcSubscriptionRefresh(name)`), `src/views/NodesView.tsx` (per-group refresh button on sub title row)

**Work**:
1. Rust `ResinClient::refresh_subscription_content(id, content)` — PATCH body `{content, source_type:"local"}` to `/api/v1/subscriptions/{id}`. Reuses existing bearer+loopback guards. 1 new mockito test.
2. Rust `subscription_refresh(name: String, app)` command: `list_subscriptions()` → match by name → fetch `subscription.url` (already stored by earlier `subscription_add`? verify shape) via existing `fetch_clash_subscription` (clash UA rotate) → `clash_yaml_to_proxies_block` → `refresh_subscription_content(id, block)`. Every step `tracing::info!`/`warn!`. 1 new cargo unit test.
3. TS `ipcSubscriptionRefresh(name)` wrapper, validates name length+control chars. `NodesView.tsx` group title row gains a "🔄" button (i18n `nodes.refreshSub`) that calls the wrapper and triggers `refresh()` poll to re-read.
4. If subscription has no stored url (legacy): toast `nodes.refreshNoUrl` + no-ops.

**i18n keys** (18 × 3 = 54 new): `nodes.refreshSub`, `nodes.refreshingSub`, `nodes.refreshNoUrl`.

**Loop test**: 1 new cargo (mockito happy PATCH), 2 vitest (refresh button calls wrapper with name; `assertShortName` guard fires on empty). Existing `ipc.test.ts` pattern.

**Estimate**: ~50 Rust + ~25 TS, 3 i18n keys ×18.

### Phase 3 — Per-node probe IPC (Resin native endpoint)

**Files**: `crates/resin-core/src/resin_client.rs` (new `probe_node_egress(node_hash)` + `probe_node_latency(node_hash)`), `src-tauri/src/commands/mod.rs` (new `node_probe(node_hash, kind)`), `src/lib/ipc.ts` (`ipcNodeProbe(hash, kind)`), `src/views/NodesView.tsx` (per-card test button → local latency + egress_ip update)

**Work**:
1. Rust `ResinClient::probe_node_egress(hash)` — POST `/api/v1/nodes/{hash}/actions/probe-egress` (returns `{egress_ip, latency_ms_ewma}`). `probe_node_latency(hash)` — POST `/actions/probe-latency` (sync, returns `{latency_ms}`). Both reuse bearer+loopback. 2 new mockito tests.
2. Rust `node_probe(node_hash: String, kind: String, app)` — validate hash length ≤128 + hex/control reject; validate `kind` ∈ `{"egress","latency"}`; forward. Cancellation token from `SidecarHandle` (same as health probe). 1 new cargo unit test.
3. TS `ipcNodeProbe(hash, kind)` wrapper, 2 new vitest (forward + reject invalid kind).
4. `NodesView.tsx` per-card "📡" button on hover row. Calls `ipcNodeProbe(hash, 'latency')` first (quick), updates the row's `reference_latency_ms` + color; optional "出口 IP" button → `ipcNodeProbe(hash, 'egress')` fills `egress_ip`. In-flight = `testing` state (spinner). Error toast: `nodes.probeError`.

**i18n keys** (18 × 4 = 72): `nodes.probeLatency`, `nodes.probeEgress`, `nodes.probing`, `nodes.probeError`.

**Loop test**: 2 cargo mockito + 2 vitest + 1 component test (button click → wrapper called → state updates).

**Estimate**: ~60 Rust + ~40 TS, 4 i18n keys ×18.

### Phase 4 — Per-group batch probe + whitebox config knobs

**Files**: `src/lib/settings.ts` (new `loadNodeProbe` + `saveNodeProbe`), `src/views/NodesView.tsx` (per-group "批量测速" button + concurrency gate), `src/views/SettingsView.tsx` (new "节点探测参数" panel exposing `nodeProbe` + Resin `/system/config` knobs), `src/lib/ipc.ts` (no new wrapper, uses `ipcNodeProbe` + existing `system_config_get/patch` if exposed — verify)

**Work**:
1. `settings.ts` new `loadNodeProbe()` / `saveNodeProbe(cfg)` reading/writing `settings.json#nodeProbe` with defaults: `{concurrency:10, timeout_ms:10000, test_url:"https://www.gstatic.com/generate_204", batch_on_load:false}`. Config struct in `src/lib/types.ts`.
2. `NodesView.tsx` per-group "批量测速" button (i18n `nodes.batchProbe`). On click: read `loadNodeProbe()`, gate concurrent inflight probes to `concurrency` cap, timeout each at `timeout_ms` (AbortController), results update each row's `reference_latency_ms`. Skip if `batch_on_load === false` on initial load (甲 default). 3 new vitest: concurrency cap respected, timeout fires, batch_on_load=false skips auto-batch.
3. `SettingsView.tsx` new "节点探测参数" panel card. 4 inputs: concurrency (number 1-50), timeout_ms (number 1000-30000), test_url (string/url), batch_on_load (toggle). Save button writes via `saveNodeProbe()`. Also a "Resin 探测参数" sub-card exposing the 7 hot-update knobs (`max_consecutive_failures`, `max_latency_test_interval`, `max_authority_latency_test_interval`, `max_egress_test_interval`, `latency_test_url`, `latency_authorities`, `p2c_latency_window`, `latency_decay_window`) via `system_config_get` (load on mount) + `system_config_patch` (save). Check existing `ipcSystemConfigGet`/`ipcSystemConfigPatch` wrappers — if missing, add in this phase. i18n: `settings.nodeProbe` namespace.
4. Validate: concurrency 1-50, timeout_ms 1000-30000 at both TS + Rust boundary (if any). `test_url` must `http(s)://` prefix.

**i18n keys** (18 × 10 = 180): `nodes.batchProbe`, `nodes.batchProbing`, `settings.nodeProbeTitle`, `settings.nodeProbeConcurrency`, `settings.nodeProbeTimeout`, `settings.nodeProbeTestUrl`, `settings.nodeProbeBatchOnLoad`, `settings.resinProbeTitle`, `settings.resinMaxFailures`, `settings.resinLatencyInterval`.

**Loop test**: 3 vitest + 1 component (Settings panel round-trip load → edit → save → reload).

**Estimate**: ~80 TS, 0 Rust (barring `system_config` wrapper gap), 10 i18n keys ×18.

### Phase 5 — Full gate build + test + exe smoke + AGENTS.md + i18n:check

**Files**: `AGENTS.md` (new §31 "P25-T19 node-pool fixes"), `docs/CONTEXT.md` (4 new terms), i18n:check gate

**Work**:
1. `pnpm i18n:check` = 363+ (6+3+4+10) = 386 keys / 18 locales green.
2. `pnpm exec tsc --noEmit` green.
3. `pnpm test` = baseline + P1 (5) + P2 (2) + P3 (4) + P4 (4) = 15 new vitest.
4. `cargo test -p resin-core --lib` = baseline + P2 (1) + P3 (3) = 4 new cargo.
5. `pnpm build` (tsc + vite) green, new chunk hash.
6. `cargo build --release -p egressapikey-app --features custom-protocol` → exe staged at `release/windows-gui/EgressAPIKEY.exe` (host triple `x86_64-pc-windows-msvc`).
7. Grep new Vite chunk hash in staged exe bytes (hard close-loop, AGENTS §5).
8. Smoke launch: `MainWindowTitle='EgressAPIKEY'`, resin.exe child alive, /healthz 200.
9. `codegraph sync .` re-index.
10. `git add` + commit message in `.codex-tmp/commit-msg.txt` + `git commit -F` + `git push`.
11. AGENTS.md §31 documents: default-collapse, hide-unhealthy, delay> syntax, subscription_refresh, per-node probe endpoint, batch probe gate, nodeProbe config knob, Resin `/system/config` plumbing, test counts, i18n key count.

**CONTEXT.md deltas** (4 new terms):
- `defaultCollapsed` — NodesView groups collapse by default after first refresh (seed-all `collapsed` Set). Industry mental model: metacubexd `hideUnAvailableProxies`, clash-verge #979.
- `subscriptionRefresh` — shell-initiated local subscription content refresh. Shell re-fetches remote Clash YAML (UA rotate), converts via `clash_yaml_to_proxies_block`, PATCHes `/api/v1/subscriptions/{id}` content. Resin `/actions/refresh` on local source is a no-op re-parse, so this path is necessary for real remote updates.
- `nodeProbe` — manual or batch per-node ping via Resin `/actions/probe-egress` or `/actions/probe-latency`. Whitebox settings.json knobs: `concurrency`=10, `timeout_ms`=10000, `test_url`=gstatic generate_204, `batch_on_load`=false. Sources: clash-verge `DEFAULT_DELAY_TIMEOUT` + Resin `latency_test_url`.
- `probeKind` — enum `{"egress","latency"}`. `egress` returns exit IP + EWMA latency (heavier, 24h internal cap); `latency` is sync quick ping. Both go through the same `node_probe` IPC, dispatched by `kind`.

**Estimate**: ~30 docs, 4 CONTEXT terms, gate verification.

## ADR-0044 (decisions locked)

1. **S1 — Default collapse + hide-unhealthy + latency sort**: `collapsed` Set seeded after first refresh. Industry has no default-collapse but `hideUnAvailable` is the most-requested missing feature; we combine the two. 3-state sort reuses `reference_latency_ms`.
2. **S2 — Local subscription refresh via shell re-fetch**: local source_type means Resin re-parses in-memory content; to actually pull fresh remote nodes the shell must re-fetch + re-POST content. Rejected: auto-poll subscriptions from shell (Resin `update_interval` already exists for remote source_type — but our shell uses local so this path does not apply). Rejected: fork Resin to add a local-refresh remote hook (A4-1 style; overkill for this phase).
3. **S3 — Per-node probe via Resin native endpoint**: `probe-egress` and `probe-latency` exist in Resin v1.2.0 (`ProbeEgressSync`/`ProbeLatencySync`). Shell just wires the endpoint through; no probe logic in the shell. Rejected: shell-side mihomo `/proxies/{name}/delay` (we have no mihomo API surface for nodes-by-hash; Resin owns node identity).
4. **S4 — Whitebox probe knobs + default 甲+丙 with group concurrency**: defaults from clash-verge + Resin industry research (`concurrency=10`, `timeout=10s`, `test_url=gstatic`, `batch_on_load=false`). Resin hot-knobs surfaced separately via `/system/config`. Rejected: auto-batch on first load (clash-verge #979 community pain point); manual is the polite default.

## Rejected alternatives (recorded for the audit trail)

- Virtual scrolling for node cards: no industry precedent (clash-verge/metacubexd/v2rayN all use group collapse + filter, not virtual rows); we have `VIRTUAL_THRESHOLD=50` already.
- Auto-batch probe on load: clash-verge #979 community rejects; passive 默认 manual.
- Shell-side TCP ping tool: would bypass Resin's health state; Resin `probe_node_egress` returns Resin's own EWMA so the shell stays consistent with routing decisions.
- Fork Resin Go to add time/url/env-var knobs: `RESIN_PROBE_CONCURRENCY` and `RESIN_PROBE_TIMEOUT` are EnvConfig (not hot-update); `/system/config` knobs (max_*, latency_*) ARE hot-update so we surface those instead and leave env-only knobs to startup.
