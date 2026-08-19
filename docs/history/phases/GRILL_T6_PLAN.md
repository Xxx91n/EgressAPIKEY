> **Superceded by `docs/GRILL_T6_NETWORK_LAYER_PLAN.md`** (2026-08-12). This earlier plan defined T6 (Q1-Q6) as an architecture-cleanup + unified-bugfix round and is preserved as a historical audit trail only. The live T6 work was re-executed under the Network Layer plan, which re-organized the same Q1-Q5 caps into a clean T6-1 through T6-9 schedule with closed-loop test gates and a handoff doc. Do NOT cite this file as the current plan; cite `docs/GRILL_T6_NETWORK_LAYER_PLAN.md`.

# GRILL T6 PLAN — Architecture Cleanup + Unified Bug Fix

> **Date**: 2026-08-10  
> **Grill round**: T6 (Q1-Q6 all resolved, handoff authorized)  
> **Supersedes**: T5 trace_id + IpcError refactoring (P29/P30)  
> **Ponytail**: shortest working diff, reuse existing helpers, no new deps  

---

## Executive Summary

Grill T6 identified 6 bugs from user's live testing of `release/windows-gui/EgressAPIKEY.exe`.  
The previous agent's handoff claimed "all resolved" — **code-level audit proved 5 of 6 were NOT landed**.  
This document is the consolidated plan table for the execution phase.

---

## Bug → Root Cause → Fix → Test Mapping

### Bug 1: HTTP port shows "unreachable" despite being alive

| Field | Value |
|-------|-------|
| **User evidence** | A6: "1791端口 HTTP状态已开放且正常工作，验证结果收到404 Not Found，证明该端口运行HTTP协议服务。GUI依旧显示端口不可达" |
| **Root cause** | `port_health_check` at `mod.rs:1820` sends `CONNECT 127.0.0.1:{port} HTTP/1.1` — Resin's HTTP proxy doesn't support CONNECT tunneling, returns 404/error → code marks unreachable |
| **Fix** | Change HTTP probe from `CONNECT` to `GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n` — any HTTP response (200/404/400) proves the port is alive |
| **Location** | `src-tauri/src/commands/mod.rs` line ~1820 |
| **Test** | cargo unit test: mock TCP listener returns `HTTP/1.1 404 Not Found\r\n\r\n` + assert `reachable: true, socks5_ok: false, protocol_mismatch: false` |

### Bug 2: Strategy "unknown variant `balanced`" error

| Field | Value |
|-------|-------|
| **User evidence** | "平台内什么策略我都设置不了，报错：strategy config invalid: unknown variant `balanced`" |
| **Root cause** | `src/lib/policy.ts` still exists with a 3-branch switch using Resin native values (`BALANCED`/`PREFER_LOW_LATENCY`/`PREFER_IDLE_IP`). `ALLOCATION_POLICIES` in `ipc.ts:84` has Resin-native values. `PlatformsView.tsx:434` displays via `policyToI18nKey(pol)` from the old `policy.ts`. The shell 6-option `StrategyId` (random/sequential/latency/quality/bandwidth/protocol_weight) was committed in i18n + Rust but **never wired into the GUI select**. The `<select>` sends `BALANCED` as the value, but the Rust `strategy_config_put` command validates against the shell 6-option enum, rejecting `balanced` (lowercase, from i18n label being confused with the value). |
| **Fix** | (a) Delete `src/lib/policy.ts` — replace `policyToI18nKey` with `strategyToI18nKey` using the 6 committed `strategy.*` i18n keys. (b) Replace `ALLOCATION_POLICIES` in `ipc.ts` with `STRATEGY_IDS = ["random","sequential","latency","quality","bandwidth","protocol_weight"]`. (c) New `strategyToResinPolicy(): StrategyId → AllocationPolicy`: random/sequential→B+ALANCED, latency→PREFER_LOW_LATENCY, quality→PREFER_IDLE_IP, bandwidth→BALANCED, protocol_weight→BALANCED. (d) `ipcPlatformUpdate` translates `StrategyId → AllocationPolicy` before PATCH. (e) `PlatformsView` and `TopologyView` use `strategyToI18nKey` + `STRATEGY_IDS`. |
| **Location** | `src/lib/policy.ts` (DELETE), `src/lib/ipc.ts` L84-85, L163-171, `src/views/PlatformsView.tsx` L433-434, `src/views/TopologyView.tsx` L15, L508 |
| **Test** | vitest: (1) `strategyToResinPolicy` maps all 6 values correctly; (2) `strategyToI18nKey` maps all 6 values to existing i18n keys; (3) `ipcPlatformUpdate` translates before forwarding; (4) invalid strategy value rejected at TS boundary |

### Bug 3: Subscription drag-to-reorder doesn't work

| Field | Value |
|-------|-------|
| **User evidence** | A5: "只有爬取的鼠标，具体表现拖动完全没反应" |
| **Root cause** | `SubscriptionsView.tsx:267` `setPointerCapture(e.pointerId)` locks ALL pointer events onto the source `<li>` element. After capture, `onPointerEnter` on OTHER rows **never fires** because the captured element intercepts all pointer events. The user sees the cursor move but nothing happens — the list doesn't reorder. |
| **Fix** | Delete the `setPointerCapture` line. Without capture, pointer events bubble naturally — `onPointerEnter` fires on every row the cursor passes over. The window-level `pointerup` listener (already implemented L256) handles release-outside-row. |
| **Location** | `src/views/SubscriptionsView.tsx` line 267 — delete the single line |
| **Test** | vitest: simulate pointer down on row 0, pointer enter on row 2, pointer up → assert list reordered (row 0 content now at index 2) |

### Bug 4: Port conflict error shows `[object Object]`

| Field | Value |
|-------|-------|
| **User evidence** | A6: "端口17111添加失败" + "报错的是[object Object]" |
| **Root cause** | **Two bugs**: (1) `map_resin_error` at `mod.rs:83` matches `CONFLICT` (from HTTP 409 Conflict status) BEFORE the `bind` match at L97 → port bind error gets `error.conflict` instead of `error.bindConflict:PORT`. (2) `PlatformsView.tsx:433` `.catch((err) => ... String(err))` — `String(IpcError)` returns `[object Object]` because IpcError is a complex object, not an Error instance. `translateError` exists (L5 import) but is NOT used at L433. |
| **Fix** | (1) Reorder `map_resin_error`: move `bind` match (L97-102) BEFORE `CONFLICT` match (L83-84). (2) Change `PlatformsView.tsx:433` catch to `translateError(e, t)`. |
| **Location** | `src-tauri/src/commands/mod.rs` L83/L97 reorder, `src/views/PlatformsView.tsx` L433 |
| **Test** | cargo: reorder test — `map_resin_error("409 Conflict: listen on port 17111: bind: Only one usage...")` → `error.bindConflict:17111`. vitest: L433 catch path uses translateError. |

### Bug 5: Port auth info username missing platform prefix

| Field | Value |
|-------|-------|
| **User evidence** | A6: "SOCKS5账号命名格式必须为<平台名称>.<账号名称>...正确账号为Default.port-1792...但GUI返回port-1792" |
| **Root cause** | `port_auth_info` at `mod.rs:1735-1741` returns `username = mapping.account` (or `port-{port}` fallback). But Resin SOCKS5 requires `<Platform>.<Account>` format. Without the platform prefix, SOCKS5 auth succeeds (password = proxy_token validates) but CONNECT returns General failure (error 1) because Resin doesn't know which platform the traffic belongs to. |
| **Fix** | Change username to `format!("{}.{}", mapping.platform_name, mapping.account)` (or `format!("{}.port-{}", mapping.platform_name, port)` for empty-account fallback). |
| **Location** | `src-tauri/src/commands/mod.rs` L1735-1741 |
| **Test** | cargo: mock DbPool with a `PortMapping { port: 1792, platform_name: "Default", account: "port-1792" }` → assert `username == "Default.port-1792"` |

### Bug 6: Two headless.exe binaries

| Field | Value |
|-------|-------|
| **User evidence** | "egressapikey_headless.exe和egressapikey-headless.exe，怎么有两个" |
| **Root cause** | `build-all.ps1` filename pattern mismatch — one from cargo `--bin` flag (`egressapikey_headless.exe` with underscore) and one from a rename step (`egressapikey-headless.exe` with hyphen) |
| **Fix** | Already partially addressed in P30-AUDIT commit (ec1f629). Verify only one exe is staged in `release/windows-gui/`. |
| **Test** | Verify `release/windows-gui/` has exactly one headless binary via build script assertion |

---

## Execution Phases (small-step commits, each with closed-loop test)

### Phase 1: Strategy unification (Bug 2 + Bug 4 strategy prefix)
1. Delete `src/lib/policy.ts`
2. Create `strategy.ts` with `StrategyId`, `STRATEGY_IDS`, `strategyToI18nKey()`, `strategyToResinPolicy()`
3. Update `src/lib/ipc.ts`: replace `ALLOCATION_POLICIES` with `STRATEGY_IDS`, add translation in `ipcPlatformUpdate`
4. Update `src/views/PlatformsView.tsx` L433-434: use `STRATEGY_IDS` + `strategyToI18nKey`
5. Update `src/views/TopologyView.tsx` L15/L508: replace `policyToI18nKey` import + call
6. **Test**: vitest — mapping + option round-trip + invalid rejection
7. **Commit**: `P31-T6: strategy unification — shell 6-option as sole UI source of truth`

### Phase 2: Error handling unification (Bug 4)
1. `src-tauri/src/commands/mod.rs`: reorder `map_resin_error` match arms (bind before CONFLICT)
2. `src/views/PlatformsView.tsx` L433: change `.catch` to use `translateError(e, t)`
3. Scan ALL views for remaining `String(err)` patterns → replace with `translateError`
4. **Test**: cargo — bind-conflict maps to `error.bindConflict:PORT`; vitest — translateError on IpcError
5. **Commit**: `P31-T6: error handling unification — bind conflict before CONFLICT + translateError everywhere`

### Phase 3: Drag fix (Bug 3)
1. `src/views/SubscriptionsView.tsx` L267: delete `setPointerCapture` line
2. Add `releasePointerCapture` in `onPointerUpRow` as safety net
3. **Test**: vitest — simulate drag reorder
4. **Commit**: `P31-T6: drag fix — remove setPointerCapture blocking onPointerEnter`

### Phase 4: Port health + auth info fix (Bug 1 + Bug 5)
1. `src-tauri/src/commands/mod.rs` L1820: change HTTP greeting from `CONNECT` to `GET / HTTP/1.1`
2. `src-tauri/src/commands/mod.rs` L1735-1741: change `port_auth_info` username to `{platform}.{account}`
3. **Test**: cargo — HTTP probe sends GET; port_auth_info returns platform-prefixed username
4. **Commit**: `P31-T6: port health HTTP GET + SOCKS5 Platform.Account username format`

### Phase 5: Build + stage + smoke (AGENTS §5)
1. `pnpm build` + `cargo build --release -p egressapikey-app --features custom-protocol`
2. Stage to `release/windows-gui/EgressAPIKEY.exe`
3. Verify Vite chunk hash embedded in exe bytes
4. Smoke launch: MainWindowTitle + resin.exe child alive
5. **Commit**: `P31-T6: build + stage release exe`

---

## ADRs

| ADR | Title | Status |
|-----|-------|--------|
| ADR-0030 | Strategy name unification — shell 6-option as sole UI source of truth | PROPOSED |
| ADR-0031 | Drag fix — remove setPointerCapture (Pointer Events capture blocks onPointerEnter) | PROPOSED |
| ADR-0032 | Port health probe — HTTP GET instead of CONNECT + port_auth_info Platform.Account username | PROPOSED |

---

## Constraints

- **Ponytail full**: shortest working diff, reuse existing helpers, no new deps
- **ctx_* first**: file edits via `ctx_execute_file` + Node `fs.writeFileSync`
- **Single-threaded**: no subagents
- **i18n**: 18 base locales must stay in sync (strategy.* keys already exist from T5)
- **Closed-loop tests**: every fix must have a test that would fail if the fix is reverted
- **AGENTS §5**: each commit touching src/crates must rebuild + stage release exe + verify Vite hash
