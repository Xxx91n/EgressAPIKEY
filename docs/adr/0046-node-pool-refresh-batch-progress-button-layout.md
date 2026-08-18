# ADR-0046: Node Pool Refresh + Batch Progress + Button Layout

Date: 2026-08-19
Status: ACCEPTED
Supersedes: none (ADR-0044 S2 was already superseded by ADR-0047; this ADR layers UI/UX decisions on top of ADR-0044 S1/S3/S4 and ADR-0047)

## Context

The node pool tab (`src/views/NodesView.tsx`) has three user-visible bugs reported after the T20-v2 audit close (commit 1d23d74):

1. **Refresh + BatchProbe button position is off**: the T20-v2 nested-button fix (to resolve HTML spec 4.10.1 violation) moved the two action buttons out of the title `<button>` into a sibling `<div>` below the title row; the visual result is a separate below-the-title band instead of an inline toolbar. clash-verge-rev HeadState puts all group-level IconButtons inline in the title row.
2. **Batch probe progress stuck at 0**: `handleBatchProbe` tooltip `t("nodes.batchProbing", { done: 0, total: items.length })` hardcodes `done=0`; the `done++` counter inside the chunk loop never calls a setState, so React never re-renders the tooltip, so users see `0/N` frozen even while probes resolve.
3. **Refresh sub silent + top-right refresh mental model confused**: `handleRefreshSub` sets `refreshingSub` spinner but gives no toast; `await refresh()` immediately after `ipcSubscriptionRefresh` may read stale Resin memory (though, per source-level research of Resin v1.2.0 `Scheduler.UpdateSubscription`, this is actually sync and not stale). The top-right page-level refresh button tooltip `nodes.refresh` is the same string as the per-sub refresh button, conflating two distinct actions (re-read Resin memory vs trigger Resin remote-fetch).

## Decisions

### S1 - Inline button row via `<div + onClick>` (Q1-A)

Swap the outer `<button onClick={() => toggleSub(sub)}>` (L401) for `<div onClick={...} role="button" tabIndex={0}>` and bring the two IconButtons (RefreshCw + Activity) inline into the same `flex items-center gap-2` row as the chevron + name + node-count + health-rate. Title bar IS the per-group toolbar; no separate action band below. Keyboard access: add `onKeyDown` for Enter/Space.

**Rejected**: keep nested `<button>` (already a spec violation T20-v2 fixed). **Rejected**: separate sibling div with negative margins (visual cheating, does not fix mental model).

### S2 - Batch progress via per-probe `setProbeResults` + `batchProgress` Map (Q2-A)

Extract `applyProbeResult(prev, hash, kind, res)` pure helper from `handleProbeNode` and call it from both the single-node path and the batch path. Each probe completion writes `setProbeResults(prev => applyProbeResult(prev, hash, "latency", res))` so every row latency updates `-` to `ms` in real-time, matching clash-verge-rev `DelayManager.setListener` per-proxy model. Add `batchProgress: Map<string, {done, total}>` state; the batch tooltip interpolates `{{done}}/{{total}}` from this Map. Increment after each probe resolves; reset on `finally`.

**Rejected**: only display total progress text without per-row update (users cannot see WHICH node finished). **Rejected**: wait until all probes done then render once (no real-time visual feedback, same symptom as today bug). **Rejected**: full clash-rev `DelayManager` single-instance service (200+ lines of `useReducer` + listener lifecycle; overkill for our per-node need).

### S3 - Refresh sub sync toast feedback (Q5a-A, fact-driven)

Resin v1.2.0 `/actions/refresh` is `RefreshSubscription(id) -> Scheduler.UpdateSubscription(sub)` which for `source_type=remote` BLOCKS on `Fetcher(attemptURL)` - the new Clash YAML is already in Resin memory when the POST responds. Therefore the frontend needs no delayed/polling fallback. Toast flow: `refreshSent` fires immediately on click, `await ipcSubscriptionRefresh(name)` blocks on the sync backend, `await refresh()` re-reads now-fresh Resin memory, then compare `nodes.length` before/after to pick `refreshDone` (delta > 0) or `refreshNoChange` (delta = 0).

**Rejected**: 1.5s / 3s delayed poll (Q2-A provisional plan - based on wrong assumption that backend is async). The source-level research of `internal/topology/subscription_scheduler.go UpdateSubscription` confirms sync-block-on-Fetcher for remote source_type.

### S4 - Density refactor: 3 text cards to popover / footer / sticky toolbar (Q3-A + Q4-A)

Delete the three standalone info cards (`egressPolicyNote`, `protocolWeights`, `reputationTitle`): the two policy/protocol note bodies crumb into an `<Info size=12 />` icon at the top-right corner of the giveaway StatCard with a `z-50 whitespace-pre-line` hover popover showing both notes; reputation content moves into the `lastRefresh` footer row as a 1-line compact 2-column flex (reputation summary left, timestamp right). Promote the search toolbar (search input + 5 toggle chips + expand/collapse-all) to `sticky top-0 z-30` so it stays visible while the node list scrolls. Top-right page-level refresh button tooltip renames to `nodes.syncCache` ("Re-sync backend node snapshot") to separate from the per-sub refresh semantic.

**Rejected**: delete cards permanently (loses discoverability for new users). **Rejected**: collapse-on-expand panel (still occupies row outline space compared to popover). **Rejected**: move reputation/egressPolicyNote/protocolWeights to Settings page (i18n key migration from `nodes.*` to `settings.*` across 18 locales is scope creep).

## Consequences

- **Positive**: 3 user-reported bugs fixed in one surgical plan; per-row real-time latency feedback matches industry-standard (clash-verge) mental model; page-level vs group-level refresh semantic distinction becomes explicit; node pool page vertical density improves (3 dead cards to 1 popover + 1 footer line, sticky toolbar removes one scroll-to-top round-trip).
- **Negative**: the inline title row gets busier (6 elements in one flex row instead of 3+2 in two rows); at small viewports the row may wrap awkwardly. Mitigation: `flex-wrap` on the container with `gap-2` lets it wrap to second line gracefully without separate sibling div.
- **Neutral**: no new IPC commands, no new Rust surface, no backend event channel needed (T21 relies on Resin being sync - already true in v1.2.0).

## Open debt explicitly NOT addressed

- The double `ADR-0047` filenames (`0045-ipc-error-contract-hardening.md` and `0047-subscription-refresh-native-resin-actions.md`) - scope creep; defer to a future ADR-hygiene commit.
- `VirtualNodeList` virtualization for >50 nodes already works; clash-rev-style column reflow is bigger work and out of scope.
- `settings.json#nodeProbe` knobs surface in Settings panel (ADR-0044 S4 surface) still planned but unchanged by T21.

## References

- ADR-0044 (node-pool collapse + Resin-native probe + whitebox knobs) - S1/S3/S4 still hold, S2 superseded by ADR-0047.
- ADR-0047 (subscription refresh sink to Resin native `/actions/refresh` with `source_type=remote`) - T21 builds on top of this foundation.
- `docs/GRILL_T21_NODE_POOL_BUTTON_SYNC_PLAN.md` - the 5-phase execution plan implementing these decisions.
- clash-verge-rev `src/components/proxy/use-filter-sort.ts` + `proxy-item.tsx` + `use-render-list.ts` - research source for the per-proxy DelayManager listener pattern.
- Resin v1.2.0 `internal/topology/subscription_scheduler.go UpdateSubscription` - source-level evidence that `/actions/refresh` is sync-block-on-Fetcher for remote source_type.