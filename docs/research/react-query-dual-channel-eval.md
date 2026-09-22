# React Query over the Tauri invoke dual channel — R12-01 evaluation

Date: 2026-09-22
Status: evaluation record — migration SUSPENDED (unblock conditions below)
Ticket: R12-01 (r12-wave-a D-003⑤). Time-boxed 0.5-day evaluation, as
scoped. This is not a migration commit.

## The three seams under evaluation

| Seam | Current shape |
|---|---|
| Polling | `usePoll` (`src/hooks/usePoll.ts`): 40 lines, zero deps — interval + AbortController cancel + `visibilitychange` pause/resume. Every data view polls through it (Topology/Subscriptions/Platforms/Nodes/Diagnostics). |
| Dispatch | `invoke()` in `src/lib/ipc.ts` is the single dual-channel point: Tauri `invoke()` on desktop, `invokeHeadless` fetch -> axum routes in headless (`headless-routes.ts` cmd->REST table). Both carry `__trace_id` / typed `IpcError` shape uniformly. |
| Writes | Direct `await ipcXxx()` calls; `SettingsView` lightweight config writes via a 500 ms debounce (`lightweightSaveRef`). |

## Question 1 — does a React Query `queryFn` compose with the dual channel?

Yes, trivially: `queryFn: () => ipcAuthoritativeSnapshot()` — the queryFn
never sees the transport; both desktop invoke and headless fetch are
already behind the same wrapper. **No transport obstacle exists.** The
dual channel is a solved problem below the query layer.

What RQ would add on top:

- Cross-view cache dedup: `authoritative_snapshot`, `node_list`,
  `lease_map` are fetched by multiple views independently today (each
  view's `usePoll` fires its own call at its own cadence). RQ's query
  cache with a shared key + `staleTime` would dedupe them.
- `refetchIntervalInBackground: false` + `focusManager` already encode
  the visibility-pause semantics `usePoll` hand-rolls.
- `useMutation` gives writes pending/error state (see Q2).

What it costs:

- A new runtime dep (~13 kB gzip) + `QueryClientProvider` wiring, against
  a codebase whose stated discipline is zero-dep where a 40-line hook
  suffices (Ponytail). Two overlapping fetch systems during migration.
- The real dedup win is narrow: the hot views (Topology) already
  batch-fetch via `Promise.all` once per interval; cross-view dedup only
  matters if two views poll the same key on overlapping cadences — today
  they largely don't.
- Cache invalidation gets subtle around `strategy_apply`/`sync()` —
  today views resync imperatively after writes; under RQ that becomes
  `queryClient.invalidateQueries` discipline — an extra coordination
  surface for the dual-channel headless path (headless responses are
  plain fetch; RQ caching keys them fine, but the imperative `sync()`
  contract is well-understood and tested — 504 vitest cases exercise it).

## Question 2 — is `SettingsView`'s debounced write a mutation?

Yes in semantics: `ipcLightweightSet` is a write (it must be a
`useMutation`, never a query). But the debounce itself survives a
migration — React Query has no debounce; `useMutation().mutate()` inside
the existing `setTimeout` is the only mapping. So RQ would change the
WRITE's error/pending surface, not the debounce.

The evaluation DID surface one real defect, independent of RQ:

```ts
// SettingsView.tsx ~L122: the debounced save fires with no .catch —
// a failed save is an unhandled rejection and silently loses the write.
lightweightSaveRef.current = setTimeout(() => {
  ipcLightweightSet(lightweightEnabled, lightweightDelay);
}, 500);
```

A one-line `.catch` fixes the silent-loss; RQ is not required for it.

## Verdict — migration stays suspended

No concrete correctness or performance win was found that justifies the
dependency + dual-system transition period. The identified defect is a
missing `.catch`, not a missing framework.

### Unblock conditions (pre-written per ticket)

Migration re-opens when EITHER holds:

- **(a) concrete benefit**: a follow-up finds a correctness/perf issue
  that RQ solves materially better than the incumbent pattern — e.g.
  measured duplicate-fetch load across views (same key, overlapping
  cadence), or a write path that needs retry/ordering semantics
  `useMutation` gives for free. The silent-save defect found here does
  NOT qualify (one-line fix).
- **(b) seam collision**: the next feature must touch all three seams
  anyway (polling cadence model + dispatch + write lifecycle) — then
  migrating is cheaper than maintaining both.

## If unblocked, the mapping is mechanical

- `usePoll(fn, {intervalMs})` -> `useQuery({ queryKey, queryFn, refetchInterval: intervalMs, refetchIntervalInBackground: false })`
- imperative post-write `sync()` -> `queryClient.invalidateQueries(...)`
- debounced writes -> `useMutation` inside the existing `setTimeout`
- test mocks unchanged (they mock `invoke`, one level below queryFn).
