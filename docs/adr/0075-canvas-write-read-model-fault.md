# ADR-0075: Canvas write/read model fault — subscription intent vs region bundle

Date: 2026-09-22
Status: Accepted
Supersedes: partial write-path of ADR-0048 (read side only)
Provenance: renumbered from branch `fix/zcode-GUI-Topology` ADR-0052
(recovered from commit `4d9fa39e`, reimplemented for the CanvasV4 /
deep-IPC seam on main; the branch version wrote `strategyConfig` via
`strategy_config_put`, main's sanctioned path is the
`strategy_platform_subscriptions_set` deep edit — semantics identical,
write seam different). Re-validated against main 2026-09-22: every defect
below was live on main.

## Context

The canvas had **one generation writing** and **another generation
reading** strategy data:

| Layer | Generation | Behavior |
|---|---|---|
| **Write path** (`onConnect`, `onEdgesDelete`) | Gen-1 | Treated a dragged subscription group as a **region bundle**: expanded `sg.regions` into `region_filters` via `strategy_platform_regions_set`. Never wrote the `subscriptions` field. |
| **Read path** (`buildEdges`, `buildRegionViewEdges`) | Gen-2 (ADR-0048) | Judged edges by `subscriptions`: empty -> unconditional `"subscription:all"` label. Since the write path never filled it, every bound platform displayed `subscription:all`. |
| **Backend** (`strategy_engine.rs`) | Gen-2 (correct) | `a_class_regions` Subscription branch derives regions FROM `subscriptions`. Empty = all. **No engine change needed.** |

Second symptom — **edge scramble**: edge ids were identical across view
modes, so ReactFlow `EdgeWrapper` reused stale geometry for 1-2 frames on
view switch (xyflow #2973 measurement timing; #693: unique ids are the
only working fix).

## Decision

### 1. Write subscription INTENT, not region projection

`onConnect` subscription branch writes the subscription list and stamps
`a_class=subscription` — via the new sanctioned deep write
`strategy_platform_subscriptions_set` -> `StrategyService::
set_platform_subscriptions` (mirrors `set_platform_regions`). It no
longer expands `sg.regions` into `region_filters`; region derivation
belongs to `compute_plan` at apply time.

`onEdgesDelete` subscription branch mirrors this: removes the
subscription from `subscriptions` (not `sg.regions` from
`region_filters`).

New seam added this ticket: service fn + `#[tauri::command]` +
headless `PATCH /api/v1/shell/strategy/subscriptions` + capability +
ipc.ts wrapper `ipcStrategyPlatformSubscriptionsSet`.

### 2. Unconditional edges carry NO label

Per the Kiali/Grafana/LangGraph convention (default/unrestricted flows
carry no label; only conditional/exception edges are labeled):

- Empty `subscriptions` -> edge exists with **no label** (was
  `"subscription:all"`)
- Specific `subscriptions` -> label `"subscription:<name>"`

All `"subscription:all"` literals removed from `buildEdges` and
`buildRegionViewEdges`.

### 3. Edge id viewMode prefix

- `buildEdges` (subscription viewMode): `e-sub-{plat}-{sub}`
- `buildRegionViewEdges` (region viewMode): `e-reg-{plat}-r-{region}`

This forces `EdgeWrapper` remount on viewMode switch, eliminating stale
geometry.

## Implementation notes

- Subscription/region/quality edges keep `deletable: false` (CONTEXT.md:
  only manual edges can be dragged/deleted). React Flow v12 filters
  `deletable:false` upstream, so the `isSub` arm inside `onEdgesDelete`
  is latent code — kept as documentation of intent-removal semantics if
  the domain rule ever changes.
- The three patch helpers intentionally stay non-abstracted: each writes
  a different field with its own default `a_class`; a generic helper
  would add parameters without behavioral payoff (Fowler "Duplicated
  Code" judgment call).

## Known leftover (decided: no auto-migration)

Platforms dragged with the old Gen-1 write path have `regions` populated
but `subscriptions` empty -> they show unconditional (unlabeled) edges
until the user re-drags the subscription group once. Auto-migration was
rejected: the recorded `regions` remain a valid region strategy;
converting would be lossy.

## Test coverage

Vitest `TopologyView.test.tsx` ADR-0075 describe block: specific subs ->
`subscription:<name>` label; empty subs -> unlabeled edges; `e-sub-` /
`e-reg-` id prefixes; cross-viewMode B->C ids never collide; no
`subscription:all` literal anywhere.

## References

- xyflow issue #2973 (ResizeObserver measurement timing), #693 (edge id
  uniqueness fix)
- Kiali/Grafana/LangGraph default-flow-unlabeled convention
- CONTEXT.md glossary: "Subscription Intent" + "Strategy-Labeled Edge"
- ADR-0048 (Gen-2 read path), ADR-0036 (strategyConfig single write
  authority), ADR-0076/0077 (companion recovered fixes)
