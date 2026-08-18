# GRILL T21 - Node Pool Button Sync + Batch Progress + Density Refactor Plan

> Canon: ADR-0046 (decisions) + CONTEXT.md deltas (1 new term `nodePoolToolbar` + `nodeProbe` note) + this file (5-phase execution).
> Source grill rounds: R1 (Q1-Q4 all A) + R2 (Q5a=A / Q5b=A) + R3 (Q6a=A / Q6b=A / Q6c=A).
> Authority: ADR-0044 S1/S3/S4 still hold; ADR-0045 sets the Resin-native /actions/refresh path this plan builds on (T20-v2 already implemented).
> Branch: T21 (parallel to any audit-clean branch; tree currently clean at HEAD 1d23d74).

## Decisions locked

| Q | Topic | Answer |
|---|-------|--------|
| Q1 | Refresh + BatchProbe button position after T20 nested-button fix + top-right refresh semantic | **A** - outer `<button>` to `<div + onClick>`; merge the two action buttons inline into the same `flex` row as the title/name/count/health - no nested `<button>`, clash-rev HeadState IconButton pattern. Top-right refresh button tooltip changes to `nodes.syncCache` ("Re-sync backend node snapshot"), separated from the sub-group refresh semantic. |
| Q2 | Batch probe progress feedback (currently tooltip pinned at `done=0`) + Refresh sub silent | **A** - batch probe merges with single-node path: shared `applyProbeResult(hash, kind, res)` writes `setProbeResults` after each probe finishes, so every row lights up `- to ms` in real-time; add `batchProgress: Map<sub, {done,total}>` state for the spinner tooltip; refresh sub shows toast `nodes.refreshSent` immediately then `await`s; on success checks node-count delta and shows `refreshDone` / `refreshNoChange`. No delayed polling (see Q5a). |
| Q3+Q4 | Density refactor + top-right refresh mental model expiration | **A** - delete the 3 info-text cards (`egressPolicyNote`, `protocolWeights` body content crammed into an `Info` popover on the StatCard corner; `reputationTitle` body moves to the footer row as one line sharing `lastRefresh`). Merge the search toolbar into one sticky-top single-line flex row (search flex-1 + 5 toggles). Top-right refresh button tooltip renamed `nodes.syncCache`. |
| Q5a | Wait strategy after Refresh sub click - async or sync? | **A** - Resin v1.2.0 `/actions/refresh` is `sync` (RefreshSubscription to Scheduler.UpdateSubscription to Fetcher(url) blocks); on return new nodes are already in memory. No delayed/polling fallback. Toast `refreshSent` immediate, `await` wraps `refresh()`, compare counts to pick `refreshDone` / `refreshNoChange`. Simpler than Q2-A provisional 1.5s/3s schedule. |
| Q5b | i18n key naming convention | **A** - flat under `nodes.*` namespace. 5 new keys: `syncCache` / `refreshSent` / `refreshDone` / `refreshNoChange` / `batchProgress`. All verified unused in current 37 `nodes.*` keys. |
| Q6a | Test closed-loop form | **A** - pure helpers (`applyProbeResult` + `nextBatchProgress`) plus component mock path; +5 vitest assertions; no e2e (follow existing NodesView test pattern); no new cargo (truth is already covered by T20 mockito `refresh_subscription_native`). |
| Q6b | ADR + Plan + AGENTS.md placement | **A** - Plan `docs/GRILL_T21_NODE_POOL_BUTTON_SYNC_PLAN.md`, ADR `docs/adr/0046-node-pool-refresh-batch-progress-button-layout.md`, CONTEXT.md delta one new term + note on `nodeProbe`; AGENTS.md append section 63. |
| Q6c | Build closed-loop form | **A** - `tsc --noEmit` + `pnpm i18n:check` (+5 keys x 18 locales = +90 entries, 388 to 393) + `pnpm test` (+5 vitest) + `cargo test -p resin-core --lib` (baseline unchanged) + `pnpm build` (new chunk hash) + touch `src-tauri/src/main.rs` + `cargo build --release -p egressapikey-app --features custom-protocol` + chunk-hash verify in staged exe + smoke + `codegraph sync .` + `git push`. |

## Context (prior grill history this layer builds on)

- **ADR-0044** S1 (default-collapse + hide-unhealthy + latency sort) - untouched. S2 (shell-side subscription refresh via local re-fetch) - superseded by ADR-0045. S3 (Resin-native per-node probe) - foundation. S4 (whitebox probe knobs) - foundation.
- **ADR-0045** (T20-v2): subscription refresh sinks to Resin-native `POST /api/v1/subscriptions/{id}/actions/refresh` with `source_type=remote` on add. Resin `Scheduler.UpdateSubscription` for remote source_type is **sync** and blocks on `Fetcher(url)` returning; new nodes already in Resin memory when the POST responds. Q5a-A realises this synchronously.
- **ADR-0045 filename conflict**: `docs/adr/0045-ipc-error-contract-hardening.md` and `0045-subscription-refresh-native-resin-actions.md` both exist. This conflict itself is project historical debt; T21 does not fix it (scope drift) but records it for a separate ADR hygiene commit later.

## Phase execution plan

### Phase 1 - Layout fix: inline button row + top-right refresh tooltip rename

**Files**: `src/views/NodesView.tsx` L400-427 (button to div + inline flex)

**Work**:
1. Change L401 `<button onClick={() => toggleSub(sub)} ...>` to `<div onClick={...} role="button" tabIndex={0} className="...">`. Keep the chevron + name + count + health-rate children layout unchanged. Add `onKeyDown` handler (Enter/Space) for keyboard access (replaces native `<button>` semantics).
2. Inline the two existing action buttons (L411-418 RefreshCw + L419-426 Activity) into the same row right of the health-rate span: delete L410 outer `<div className="flex items-center gap-1 px-4 pb-1 -mt-1">` and move the two buttons up into L401 row. The row becomes a single `flex items-center gap-2` containing chevron + name + node-count + health-rate + 2 action icons.
3. Delete L410-427 outer wrapper div since buttons now in-row.
4. Top-right refresh L300-306: change button title from `t("nodes.refresh")` to `t("nodes.syncCache")` (i18n key added Phase 5). Keep the RefreshCw icon. The semantic shift is tooltip-only; the click action still calls `refresh()` (re-read Resin memory).

**i18n keys** (added Phase 5, all 18 locales):
- `nodes.syncCache`: en="Re-sync backend node snapshot" / zh="重新同步后端节点快照"

**Loop test**: Phase 2 vitest assertion that the row has no nested `<button>` children and 2 IconButtons (RefreshCw + Activity) as siblings.

**Estimate**: ~25 lines TS changed (mostly mechanical move + `<button>` to `<div>`).

### Phase 2 - Batch probe progress (real-time per-probe light-up + tooltip done/total)

**Files**: `src/views/NodesView.tsx` L194-226 `handleBatchProbe` + L132 `probeResults` + new `batchProgress` state

**Work**:
1. Extract pure helper `applyProbeResult(prev: Map, hash: string, kind, res) -> Map` from the inline block in `handleProbeNode` L174-185. Both single-probe (L173 path) and batch-probe (L206 path) now call this helper via `setProbeResults(prev => applyProbeResult(prev, hash, kind, res))`. This unifies the write-path so batch actually posts to `probeResults` (the missing wire today).
2. Add `const [batchProgress, setBatchProgress] = useState<Map<string, {done:number, total:number}>>(new Map())` state.
3. In `handleBatchProbe`:
   - Before the `for` loop: `setBatchProgress(prev => new Map(prev).set(subName, {done:0, total: hashes.length}))`.
   - In the `chunk.map` body: after `await Promise.race([probe, timeout])` try-catch, do `setProbeResults(prev => applyProbeResult(prev, hash, "latency", res))` where `res` is the resolved probe; if `Promise.race` throws, `res` stays `undefined` so `applyProbeResult` short-circuits (no-op).
   - After each `done++`: `setBatchProgress(prev => { const m = new Map(prev); const cur = m.get(subName) ?? {done:0,total:hashes.length}; m.set(subName, {done: cur.done+1, total: cur.total}); return m; })`.
4. Tooltip L420 `t("nodes.batchProbing", { done: 0, total: items.length })` to `t("nodes.batchProbing", batchProgress.get(sub) ?? {done:0, total: items.length})`. `batchProbing` already parametric on `{{done}}` / `{{total}}` - verify before Phase 5 key add.
5. On `finally`: `setBatchProgress(prev => { const m = new Map(prev); m.delete(subName); return m; })` (clean reset).

**i18n**: `nodes.batchProgress` is the tooltip string for the progress spinner itself - already seconded by `batchProbing` interpolating `{{done}}/{{total}}`; if `batchProbing` already covers it we verify there is no key gap. Add `batchProgress` only if Phase 2 outcome needs it; otherwise drop to keep YAGNI.

**Loop test** (vitest, `src/views/NodesView.test.tsx` +3 assertions):
- `applyProbeResult` pure: idempotent on same hash; `kind=egress` adds `egress_ip+region` fields without clobbering existing `latency`; `kind=latency` only updates `latency`.
- `nextBatchProgress` pure (the increment helper): `{done:N+1,total:M}` from `{done:N,total:M}` on a fired probe; reset returns `undefined` to fall back to original `batchProbe` label.
- Component mock: render NodesView, click `batchProbe` button on a sub group with 2 fake nodes, assert `ipcNodeProbe` called twice via mocked IPC, and `applyProbeResult` to `probeResults` state observed on both rows (latency = 50ms, 100ms).

**Estimate**: ~50 lines TS changed + 2 new pure helpers + 3 vitest.

### Phase 3 - Refresh sub toast feedback (sync backend fact-driven)

**Files**: `src/views/NodesView.tsx` L152-167 `handleRefreshSub`

**Work**:
1. Track pre-click node count: `const before = nodes.length` captured at click entry.
2. Show toast-like inline state first: `setLocalToast({key: "refreshSent"})` - new state `const [localToast, setLocalToast] = useState<{key:string, opts?:object} | null>(null)`, auto-clear after 2s via `setTimeout`. Render the toast in the NodesView top area near the error banner slot.
3. `const nodeDelta = await ipcSubscriptionRefresh(subName); await refresh(); const after = nodes.length; const delta = after - before;`
4. If `delta > 0` to `setLocalToast({key:"refreshDone", opts:{count: after}})`. Else to `setLocalToast({key:"refreshNoChange"})`. Both auto-clear after 3s.
5. Keep the existing `refreshingSub` Set state for the spinner - no change to that skeleton.
6. On error path: keep existing `translateError(e, t) to setError` (already in place). Add `setLocalToast(null)` on catch.

**i18n keys** (Phase 5, all 18 locales):
- `nodes.refreshSent`: en="Refresh request sent to backend" / zh="刷新请求已发送到后端"
- `nodes.refreshDone`: en="Refreshed, {{count}} nodes in pool" / zh="已刷新，当前共 {{count}} 个节点"
- `nodes.refreshNoChange`: en="No node count change; retry might be needed" / zh="节点数未变化，可稍后重试"

**Loop test** (vitest, +2 assertions):
- `handleRefreshSub` mock path: click Refresh sub button to `ipcSubscriptionRefresh` called once to `setLocalToast({key:"refreshSent"})` shown, then after `refresh()` (mocked) returns to `setLocalToast({key:"refreshDone",opts:{count:42}})`.
- `handleRefreshSub` zero-delta path: mocked `nodes.length` unchanged after refresh to `setLocalToast({key:"refreshNoChange"})`.

**Estimate**: ~30 TS, 3 i18n keys x 18 = 54 entries.

### Phase 4 - Density refactor (3 text cards to popover + sticky toolbar merge)

**Files**: `src/views/NodesView.tsx` L309-322 (reputation card) + L338-347 (egressPolicyNote + protocolWeights) + L349-386 (search toolbar) + new `InfoPopover` component (co-located helper)

**Work**:
1. Delete L309-322 standalone reputation card. Move reputation content into the L449-453 `lastRefresh` footer row: 2-column flex `flex items-center gap-2` `<footer>` - left side `reputationSummary` 1-line compact, right side `lastRefresh` time. Reputation empty / unavailable state stays 1-line with - placeholder.
2. Delete L338-347 egressPolicyNote + protocolWeights standalone cards. Crumb their body content into a small `<Info size=12 />` icon at the top-right corner of the giveaway StatCard (L324-329); `hover` opens a popover (`z-50 whitespace-pre-line`) showing both notes. Ponytail: reuse an existing `<Info>` tooltip pattern from elsewhere in the repo if any; otherwise define inline (max ~30 lines). No new dependency.
3. Promote L349-386 search toolbar (search input + 5 toggle chips + expand/collapse-all) to `sticky top-0 z-30 bg-white dark:bg-zinc-950 pb-1` so it stays visible while the node list scrolls. Keep the same flex wrap behaviour for mobile.
4. No changes to `VIRTUAL_THRESHOLD = 50` or the column layout (Ponytail: column reflow is bigger change beyond this scope).

**i18n**: no new keys (cards deleted, content reused in popover/footer; helper labels like `nodes.protocolWeights` already exist).

**Loop test** (vitest, +3 assertions):
- Render nodes view with reputation data to assert `lastRefresh` row contains reputation summary AND timestamp.
- Render nodes view to assert no standalone `egressPolicyNote` card div exists in output; assert Info icon in StatCard corner is present.
- Sticky toolbar: jsdom does not compute sticky positioning but we can assert `sticky top-0` className on the toolbar wrapper.

**Estimate**: ~60 TS, 0 new Rust, 0 i18n keys.

### Phase 5 - i18n seed + AGENTS section 63 + full gate build

**Files**: `src/locales/*/common.json` (18 files, each +4 to +5 keys), `src/test/setup.ts` en-inline mirrors 4 keys, `AGENTS.md` + section 63, `codegraph sync .`

**Work**:
1. Add 5 new keys to all 18 `common.json` files under `nodes` namespace:
   - `nodes.syncCache` (Phase 1)
   - `nodes.batchProgress` (Phase 2 formal fallback label; check if Phase 2 actually reuses it - if not, drop it to keep YAGNI; final add based on Phase 2 outcome)
   - `nodes.refreshSent` (Phase 3)
   - `nodes.refreshDone` (Phase 3)
   - `nodes.refreshNoChange` (Phase 3)
2. Run `pnpm i18n:check` - expect 388 to 393 keys x 18 locales.
3. `pnpm exec tsc --noEmit` green.
4. `pnpm test` - expect baseline + 5 (Phase 2 has 3 pure + 2 component + Phase 3 has 2 component + Phase 4 has 3 component; final count per Phase outcome).
5. `cargo test -p resin-core --lib` - baseline unchanged (no new Rust).
6. `pnpm build` (tsc + vite) - new Vite chunk hash.
7. Touch `src-tauri/src/main.rs` then `cargo build --release -p egressapikey-app --features custom-protocol` - exe staged at `release/windows-gui/EgressAPIKEY.exe`.
8. Grep new Vite chunk hash inside staged exe bytes (AGENTS section 5 hard close-loop).
9. Smoke: `Start-Process` to `MainWindowTitle="EgressAPIKEY"`, resin child alive, log tail no `WARN resin_ipc`.
10. `codegraph sync .` re-index.
11. Append `AGENTS.md` section 63 document block: P26-T21 node-pool button sync + batch progress + density refactor - 3 bugs fixed + 4 i18n keys x 18 locales + 5 vitest + cargo baseline unchanged + exe chunk-hash verified.

**CONTEXT.md deltas**:
- New term `nodePoolToolbar` - the single inline flex row on each subscription-group title bar containing chevron + name + node-count + health-rate + RefreshCw + Activity IconButton. References ADR-0046. Length-by-design (Ponytail): no separate action band below the title bar; title bar IS the only surface.
- `nodeProbe` term existing note addendum: per-probe results propagate via `applyProbeResult` immediately (clash-rev DelayManager per-proxy listener model); in-batch each row updates - to ms the moment its probe resolves; batch progress number shown in spinner tooltip for at-a-glance state.
- `subscriptionRefresh` term existing note addendum (supersedes T20 partial description): the frontend pair (`handleRefreshSub` + `ipcSubscriptionRefresh`) runs synchronously with backend: toast `refreshSent` fires immediately, `ipcSubscriptionRefresh` blocks until Resin `Scheduler.UpdateSubscription` returns (remote fetch done), `refresh()` re-reads the now-fresh Resin memory, delta check to `refreshDone` / `refreshNoChange`.

**Estimate**: ~30 docs, 5 i18n keys x 18, gate verification.

## ADR-0046 (decisions locked)

1. **S1 - Inline button row + title bar as single per-group toolbar (Q1-A)**: move Refresh+BatchProbe inline into the title row by swapping the outer `<button>` for `<div + onClick>`, matching clash-rev HeadState pattern. Rejected: nested `<button>` (HTML spec violation; original bug T20-v2 just fixed). Rejected: separate sibling div with negative margins (visual cheating, does not address mental model).
2. **S2 - Batch progress = per-probe immediate `setProbeResults` + Map-based tooltip count (Q2-A)**: clash-rev DelayManager per-proxy listener model - each probe completion instantly updates its own row latency value via `applyProbeResult` (extracted helper); batch tooltip runs `batchProgress` Map for progress number. Rejected: only display total progress text without per-row update (users cannot see WHICH node finished). Rejected: wait until all probes done then render (no real-time visual feedback, same symptom as today bug).
3. **S3 - Refresh sub sync feedback via immediate toast (Q5a-A)**: Resin v1.2.0 `/actions/refresh` is `Scheduler.UpdateSubscription` which for remote source_type blocks on `Fetcher(url)`; when POST responds the new nodes are already in Resin memory. Toast `refreshSent` shows the click, backend blocks on the same promise, `refreshDone` / `refreshNoChange` based on node-count delta. No late-polling fallback. Rejected: 1.5s / 3s retry interval (Q2-A provisional plan dropped; adds UX wait for nothing because backend is sync).
4. **S4 - Density refactor: 3 text cards to popover / footer / sticky toolbar (Q3+Q4-A)**: delete standalone egressPolicyNote + protocolWeights cards, content into Info icon popover on StatCard corner; reputation card body into the `lastRefresh` footer row as 1-line compact; search toolbar promoted to sticky top. Top-right refresh tooltip renamed `nodes.syncCache` to separate from sub-group refresh semantic. Rejected: delete cards permanently (loses discoverability for new users); rejected: collapse-on-expand panel (still occupies row outline space compared to popover).

## Rejected alternatives (recorded for audit trail)

- Keep nested `<button>` and instead of div+onClick: HTML spec 4.10.1 forbids `<button>` as a descendant of another interactive element; the original bug T20-v2 just fixed this.
- Hybrid: `<button>` parent for title, `<div>` container for actions as sibling (current post-T20-v2 structure): leaves the ugly "below the title row" L410-427 wrapper stayed, which was bug #1.
- Full DelayManager single instance (Q2-C): full clash-rev DelayManager single instance takes 200+ lines of `useReducer` + `setListener` / `removeListener` lifecycle management; our per-node need is merely state-update-on-resolve, which `setProbeResults` + `applyProbeResult` helper already covers with Ponytail concision.
- 1.5s/3s delayed poll (Q5a-B): based on wrong assumption that Resin refresh is async; source-level research of `internal/topology/subscription_scheduler.go UpdateSubscription` shows it blocks on `Fetcher(url)` for remote source type. Q5a-A holds.
- Move reputation / egressPolicyNote / protocolWeights content to Settings page (Q3-C): would require i18n key migration from `nodes.*` to `settings.*` namespace across 18 locales + routing changes. Worth it only if the user later wants a separate `diagnostics` page; not in this bug-fix scope.
- Top-right refresh button delete (Q4-C): the auto-poll does the same work every 10s, but manual trigger has UX value during rapid testing; keeping it with a corrected semantic tooltip (`syncCache`) is the balance.

## Open debt explicitly NOT addressed here

- The double `ADR-0045` filenames (0045-ipc-error-contract-hardening.md and 0045-subscription-refresh-native-resin-actions.md) - scope creep; defer to a future ADR-hygiene commit.
- VirtualNodeList virtualization for >50 nodes - already works; column reflow like clash-rev is bigger work and not in this scope.
- `settings.json#nodeProbe` knobs surface in Settings panel (ADR-0044 S4 surface) - still planned but unchanged by T21.

## Summary

3 user-reported bugs (button position off / batch progress stuck at 0 / top-right refresh mental model confused + density cluttered) addressed with 4 surgical phases (layout, batch progress, refresh toast, density) + 5 i18n keys x 18 locales + 5+ vitest closed loops. No new IPC, no new Rust, no backend change. Total estimate ~165 TS lines changed + 5 i18n keys x 18 entries + 5 vitest + 0 cargo + docs.