# ADR-0052: Canvas write/read model fault — subscription intent vs region bundle

Date: 2026-08-24
Status: Accepted
Supersedes: partial write-path of ADR-0048 (read side only)

## Context

Two user-visible symptoms appeared on the Topology canvas:

1. **Edge scramble**: switching the subscription/region selector made platform→node edges visually scramble for 1-2 frames (lines pointing at stale positions).
2. **`subscription:all` mislabel**: every platform→node edge showed `subscription:all` even when the user had bound specific subscriptions by dragging subscription groups onto platforms.

### Root cause: two-generation model fault line

The canvas had **one generation writing** and **another generation reading** strategy data:

| Layer | Generation | Behavior |
|---|---|---|
| **Write path** (`onConnect`, `onEdgesDelete`) | Gen-1 (T16) | Treated a dragged subscription group as a **region bundle**: expanded `sg.regions` into `strategyConfig.regions`, stamped `a_class:"region"`. Never wrote the `subscriptions` field. |
| **Read path** (`buildEdges`, `buildRegionViewEdges`) | Gen-2 (ADR-0048) | Judged edges by `strategyConfig.subscriptions`: empty → `"subscription:all"` label. Since the write path never filled it, every bound platform still displayed `subscription:all`. |
| **Backend** (`strategy_engine.rs`) | Gen-2 (correct) | `a_class_regions` Subscription branch derives regions FROM `subscriptions` and PATCHes Resin `region_filters`. Empty = all regions. **No backend change needed.** |

The sync merge (`sync` callback L987-1006) correctly reads `ps.subscriptions` into `subscriptionNames` — but since the write path never wrote `subscriptions`, `subscriptionNames` was always `[]`.

### Edge scramble mechanism

Edge ids were identical across view modes (`e-{plat}-{sub}` for subscription view, `e-{plat}-r-{region}` for region view — but port/entry edges like `e-port-{port}-{name}` and `e-entry-{name}` were shared). When ReactFlow's `EdgeWrapper` receives the same edge id with a different target node after a viewMode switch, it reuses the internal edge instance. Combined with `ResizeObserver` measurement timing (xyflow issue #2973: "nodes render before their dimensions are measured"), the edge path geometry is stale for 1-2 frames.

**Industry-verified fix** (atomcode research 2026-08-24): make edge ids unique per view mode so `EdgeWrapper` remounts fresh. This is the recognized pattern from erda-ui production code (edge id = `${parent.id}-${rest.id}`) and xyflow issue #693 ("changing the nodes ids each time" was the only working fix).

## Decision

### 1. Write subscription INTENT, not region projection

`onConnect` subscription branch now writes `strategyConfig.subscriptions` (accumulate + dedupe) and stamps `a_class:"subscription"`. It no longer expands `sg.regions` into `strategyConfig.regions` — region derivation belongs to `compute_plan` in the strategy engine.

`onEdgesDelete` subscription branch mirrors this: removes the subscription from `subscriptions` instead of subtracting `sg.regions` from `region_filters`.

New helper: `patchSubscriptionsViaStrategyConfig(platName, nextSubs)` — mirrors `patchRegionViaStrategyConfig` but writes `subscriptions` + `a_class:"subscription"`.

### 2. Unconditional edges carry NO label

Per the Kiali/Grafana/LangGraph convention (default/unrestricted flows carry no label; only conditional/exception edges are labeled):

- Empty `subscriptions` → edge exists but has **no label** (was `"subscription:all"`)
- Specific `subscriptions` → label `"subscription:<name>"` (unchanged)

All `"subscription:all"` string literals removed from `buildEdges` and `buildRegionViewEdges`.

### 3. Edge id viewMode prefix

- `buildEdges` (subscription viewMode): `e-sub-{plat}-{sub}`
- `buildRegionViewEdges` (region viewMode): `e-reg-{plat}-r-{region}`

This forces ReactFlow `EdgeWrapper` to remount edges on viewMode switch, eliminating stale geometry.

## Known leftover (decided: no auto-migration)

Platforms previously dragged with the old Gen-1 write path have `regions` populated but `subscriptions` empty. After this fix, those platforms will show **no edge label** (unrestricted) until the user re-drags the subscription group once. This is a one-time manual re-drag taking seconds. Auto-migration was explicitly rejected (grill Q4=option 1) because:
- The old `regions` data is still valid as a region strategy; auto-converting it to subscriptions would be lossy.
- The user can see the strategy badge on the platform card and re-bind deliberately.

## Test coverage

Vitest `TopologyView.test.tsx` ADR-0052 describe block:
- `buildEdges`: specific subscriptions → label `subscription:<name>`
- `buildEdges`: empty subscriptions → edges exist with no label
- `buildEdges`: edge id has `e-sub-` prefix
- `buildRegionViewEdges`: edge id has `e-reg-` prefix
- Cross-viewMode B→C edge ids never collide
- No remaining `subscription:all` label in either builder

## References

- xyflow issue #2973 (ResizeObserver measurement timing)
- xyflow issue #693 (edge id uniqueness fix)
- erda-ui production code (edge id = `${parent.id}-${rest.id}`)
- Kiali v2.8 edge label convention (protocol + metrics, default unlabeled)
- CONTEXT.md glossary: "Subscription Intent" + "Strategy-Labeled Edge" (commit 7e80dce)
- ADR-0048 (Gen-2 read path, subscription a_class)
- ADR-0036 (strategyConfig JSON single source of truth)
