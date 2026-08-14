# GRILL T11 — Platform Page UX Polish

> Supersedes T10 platform GUI refactor for the platform page surface.  
> ADR-0032 records the architectural decisions.  
> All file edits via ctx_* per AGENTS context-mode routing.

---

## Q1-Q8 Decision Summary

| Q | Topic | Decision | Key Change |
|---|-------|----------|-------------|
| Q1 | Platform card expand/collapse | A — Collapsible | collapsed=summary row (name + A/B badges + leases/nodes pill + chevron); expanded=full A/B chip selectors. shadcn Collapsible pattern. |
| Q2 | Strategy selected visual feedback | B — Ring feedback | Extra ring-2 ring-primary ring-offset-1 on selected chips, in addition to Q1 badge. |
| Q3 | Manual node search + prefix filter | A+hybrid | Inline search filters chip list; selected nodes stay visible at top in "Selected (N)" grouped section even when filtered out. |
| Q4 | Subscription "partial not found" + stale platforms | C — Per-platform sync | Delete global Apply button; each chip change directly PATCHes the specific platform. Fix Manual bug in strategy_engine.rs where a_class_regions returns strategy.regions instead of mapping manual_nodes to regions. Auto-clean stale platform entries from strategyConfig. |
| Q5 | Port form flash on mount | A — Bootstrap gate | Skeleton until persisted settings loaded, then render form. No default value flash. |
| Q6 | Port card click toggle select | A — Toggle select | selectedPort: number|null state, click toggles, selected=ring-2 ring-primary, enables keyboard Delete. |
| Q7 | Port form compact layout | A — Single inline row | port(w-20) + protocol(w-24) + label(flex-1) + auth checkbox + add button inline. Delete grid-cols-2. |
| Q8 | Port card compact collapsing | A — Collapsible | collapsed=single row (port/protocol + health dot + auth icon + delete); expanded=details. Matches Q1 pattern. |

---

## Root Cause Analysis

### Bug 4 (subscription "partial not found")

**Location**: `src-tauri/src/commands/mod.rs:2586-2615` `strategy_apply`

**Root cause**: The command iterates `strategyConfig.platforms` and calls `platform_id_for_name()` for each. If the config contains a `platform_name` that was deleted from Resin (stale entry), `platform_id_for_name` returns `None` and the result shows `patched: false, reason: "platform not found"`.

**Secondary bug**: `crates/resin-core/src/strategy_engine.rs:165` `a_class_regions` for `AClassStrategy::Manual` returns `strategy.regions.clone()`. But the GUI Manual mode writes node hashes to `manual_nodes`, NOT region codes to `regions`. So `compute_plan` never uses `manual_nodes` and Manual strategy produces empty or stale regions.

**Fix**: 
1. Fix `a_class_regions` Manual case to map `manual_nodes` (node hashes) to regions via the healthy nodes list.
2. Auto-clean stale entries: before iteration, filter `strategyConfig.platforms` to only those that exist in the live Resin platform list.
3. Switch to per-platform sync mode (Q4-C): each chip change PATCHes the individual platform directly, no global Apply button.

### Bug 5 (port form flash on mount)

**Location**: `src/views/PlatformsView.tsx:50-54`

**Root cause**: `useState("17990")`, `useState("Default")`, `useState(true)` initialize with hardcoded defaults. The `useEffect` that loads persisted values runs AFTER the first paint, so users see the defaults flash before the real values load.

**Fix**: Add a `bootstrapped` gate state. While `false`, render a skeleton placeholder. After the first `useEffect` loads persisted values and sets `bootstrapped=true`, render the real form. Same pattern as P15 App.tsx bootstrap gate.

---

## Execution Plan (10 Steps)

| Step | Task | Files | Test | Deps |
|------|------|-------|------|------|
| T11-4a | Fix a_class_regions Manual bug | strategy_engine.rs | 2 cargo | none |
| T11-4b | Per-platform sync + auto-clean stale | PlatformsView.tsx, mod.rs | 3 vitest + 2 cargo | T11-4a |
| T11-1 | Platform card collapsible | PlatformsView.tsx | 2 vitest | T11-4b |
| T11-2 | Chip ring feedback | PlatformsView.tsx | 1 vitest | T11-1 |
| T11-3 | Manual node search + hybrid | PlatformsView.tsx | 2 vitest | T11-1 |
| T11-5 | Port form bootstrap gate | PlatformsView.tsx | 1 vitest | none |
| ~~T11-6~~ | ~~Port card toggle select~~ (REMOVED in T13: blue ring selection deleted) | PlatformsView.tsx | N/A | N/A |
| T11-7 | Port form compact inline | PlatformsView.tsx | 1 vitest | T11-5 |
| T11-8 | Port card collapsible | PlatformsView.tsx | 2 vitest | (T11-6 removed) |
| T11-9 | i18n 18 locales | src/locales/*/common.json | i18n:check | all UI |
| T11-10 | Build + smoke + chunk hash | build-all.ps1 | hard close-loop | all |

Dependencies: T11-4a first (root cause fix). T11-4b after 4a. T11-1 after 4b. T11-2/T11-3 after 1. T11-5/6/7/8 can parallel with 1-3. T11-9 after all UI. T11-10 last.

---

## Acceptance Criteria

- [ ] `cargo test -p resin-core --lib` passes with new Manual mapping tests
- [ ] `pnpm test` passes with all new vitest closed-loop assertions
- [ ] No global Apply button in PlatformsView
- [ ] Platform cards collapsible with A/B badges in collapsed state
- [ ] Selected chips show ring-2 ring-primary ring-offset-1
- [ ] Manual node search filters chips + selected nodes stay visible at top
- [ ] No port form default value flash on mount
- [ ] Port card click toggles selection (ring + keyboard Delete)
- [ ] Port form is a single inline row
- [ ] Port cards collapsible (collapsed=single row)
- [ ] `pnpm i18n:check` green (all 18 locales match)
- [ ] Release exe embeds latest Vite chunk hash (grep verified)
- [ ] Smoke launch: MainWindowTitle=EgressAPIKEY, no panic

---

## Status: PLANNING COMPLETE, AWAITING EXECUTION
