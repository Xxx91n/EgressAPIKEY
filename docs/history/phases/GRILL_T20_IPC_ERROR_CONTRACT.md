# GRILL_T20_IPC_ERROR_CONTRACT.md — IPC Error Contract Hardening

> Grill T20: 5-question decision chain. Q1-Q5 all answered. Plan finalized.

## Decisions

| Q | Decision | Description |
|---|----------|-------------|
| Q1 | A — Full refactor | All 64 `#[tauri::command]` return `Result<T, IpcError>`. Residual 3 `Result<_, String>` (L172/L1523/L1934) + 1 helper (L1968) to convert. |
| Q2 | A — Preventive lint + fault-tolerant | (b) fault-tolerant already done: `translateError` in `i18n-error.ts` handles Error vs IpcErr vs string. (a) ESLint custom rule `no-raw-error-in-toast` to prevent future regression. |
| Q3 | A — Toast action button full variant | `showToast` extends to 3rd param `action: { label, onClick }`. BindConflict → "换个端口" button calls `ipcPortSuggest()`. InvalidStrategy → "查看策略文档" link. ResinUpstream → "重试" button. Internal → no action. |
| Q4 | A — Full translateError + lint guard | **Finding: all 13 "raw catches" already use translateError** — SettingsView 7/7, NodesView 2/3 (1 is console.warn), DiagnosticsView 3/3 (zeroed-result pattern, not user-facing). Actual work = ESLint lint rule only. |
| Q5 | A — Full test coverage | cargo: `map_resin_error` 13+ branch tests + `extract_port_from_residual` boundary tests + IpcError 4 variant serde round-trip. vitest: view-level translateError closed-loop tests. ~14 cargo + ~6 vitest = 20 tests. |

## Execution Plan (5 phases)

### Phase 1 — Residual String → IpcError (Q1)
**Scope**: convert 3 `#[tauri::command]` + 1 helper from `Result<_, String>` to `Result<_, IpcError>`.
- L172: `port_upsert` (already has IpcError on 2021 variant but L172 is a different signature — verify)
- L1523: `strategy_apply` or similar command
- L1934: another command
- L1968: `validate_port_segments` helper
**IpcError enum** (already defined in resin-core): BindConflict / InvalidStrategy / ResinUpstream / Internal, each with `i18n_key` field. serde externally-tagged.
**map_resin_error** already handles 13+ Resin error patterns.
**Actions**: read each residual site, convert String → IpcError using `IpcError::internal("error.xxx")` or specific variant.
**Verification**: `cargo build -p egressapikey-app --features custom-protocol` green. `cargo test -p resin-core --lib` green.
**Estimated**: ~40 lines mechanical.

### Phase 2 — ESLint no-raw-error-in-toast rule (Q2a + Q4)
**Scope**: custom ESLint rule that flags any `catch (e)` block where `e` is passed to a function that is NOT `translateError` (i.e. raw `String(e)`, `e.message`, or bare `e` in a JSX expression).
**File**: `eslint.config.cjs` (exists, currently `rules: {}`).
**Rule**: custom processor scans catch blocks in .tsx/.ts for:
  - `showToast("err", e)` or `showToast("err", String(e))` → error
  - `showToast("err", e.message)` → error
  - Any `setXxx(e)` or `setXxx(String(e))` where the setter is not `setXxx(translateError(e, t))` → warn
**Allow**: `console.warn(e)` / `console.log(e)` (debug not user-facing).
**vitest AST guard**: a test that parses all .tsx/.ts files and asserts no catch block passes raw `e` to a user-facing display function.
**Estimated**: ~30 lines rule + ~20 lines vitest guard = ~50 lines.

### Phase 3 — Toast action button (Q3)
**Scope**: extend `showToast` signature from `(kind, msg)` to `(kind, msg, action?)` where `action: { label: string, onClick: () => void }`.
**PlatformsView.tsx L205**: modify `showToast` definition.
**BindConflict path**: on port-bind error, toast shows "换个端口" button → `ipcPortSuggest()` → fill new port in form.
**InvalidStrategy path**: toast shows "查看策略文档" → link to docs/STRATEGY.md (or tooltip).
**ResinUpstream path**: toast shows "重试" → re-invoke last failed IPC.
**Internal path**: no action button.
**i18n keys**: `error.action.changePort`, `error.action.viewDocs`, `error.action.retry` across 18 locales.
**Estimated**: ~60 lines TSX + ~10 lines i18n = ~70 lines.

### Phase 4 — Full test coverage (Q5)
**Rust cargo tests** (`src-tauri/src/commands/mod.rs` `#[cfg(test)]` module):
  - `map_resin_error` 13+ branch tests: cannot_delete_default, auth_required, auth_failed, url_parse, invalid_protocol, upstream_connect, upstream_request, bind_conflict_with_port, bind_conflict_no_port, conflict, not_found, bad_request, unauthorized, subscription_fetch_525, subscription_fetch_403, unknown_error_passes_through
  - `extract_port_from_residual` boundary tests: valid_port_in_message, no_port_returns_none, multiple_numbers_picks_first_valid, port_at_boundary (0, 65535, 65536)
  - `IpcError` 4 variant serde round-trip: BindConflict { port, i18n_key }, InvalidStrategy { value, accepted, i18n_key }, ResinUpstream { status, excerpt, i18n_key }, Internal { msg, i18n_key } — serialize → deserialize → assert field equality
**vitest tests**:
  - SettingsView: 1 test asserting `setConfigMsg(translateError(e, t))` is called in catch
  - DiagnosticsView: 1 test asserting zeroed-result pattern (not user-facing error)
  - NodesView: 1 test asserting `setError(translateError(e, t))` in catch
  - PlatformsView: 1 test asserting toast action button renders for BindConflict
**Estimated**: ~200 lines test code.

### Phase 5 — Build + smoke + commit
- `pnpm build` → new chunk hash
- `cargo build --release -p egressapikey-app --features custom-protocol` → ~5min
- Chunk hash verification in exe bytes
- Stage to `release/windows-gui/`
- Smoke: MainWindowTitle + sidecar boot + port operations
- `pnpm test` all green
- `cargo test -p resin-core --lib` all green
- `pnpm i18n:check` all 18 locales match
- `pnpm exec tsc --noEmit` green
- Commit + push

## Acceptance Criteria

1. **Zero residual `Result<_, String>` in `#[tauri::command]` signatures** (Q1)
2. **ESLint `no-raw-error-in-toast` rule active** — `pnpm exec eslint src/` passes; any new catch with raw `e` in toast/display triggers lint error (Q2a/Q4)
3. **Toast action buttons wired** for BindConflict (change port), InvalidStrategy (view docs), ResinUpstream (retry) (Q3)
4. **`map_resin_error` has >=13 cargo branch tests** + **`extract_port_from_residual` has >=4 boundary tests** + **IpcError 4 variant serde round-trip has 4 cargo tests** (Q5)
5. **>=3 vitest view-level tests** asserting translateError in catch blocks (Q5)
6. **Build + smoke + chunk hash verified** (Phase 5)
7. **Zero `[object Object]` possible** in any user-facing error path (Q2b already done, lint guards future)

## ADR
See `docs/adr/0045-ipc-error-contract-hardening.md`.

## Industry references
- Tauri v2 IPC error pattern: `tauri::ipc::InvokeError` serializes `Result<T, E: Serialize>` as externally-tagged enum
- ESLint custom rule pattern: `@typescript-eslint/utils` AST visitor on `CatchClause` body
- Toast action button: shadcn/ui `sonner` toast pattern (action prop), clash-verge-rev error toast with retry button
