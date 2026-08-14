# ADR-0032: Platform Page UX Polish

**Date**: 2026-08-14  
**Status**: ACCEPTED  
**Supersedes**: ADR-0029 (platform page UX fix, T8) partially — adds collapsible cards, per-platform sync, port form polish  

## Context

The platform page (PlatformsView.tsx) has accumulated UX issues across T8-T10 iterations:
1. Platform cards always render at full expansion — no way to scan many platforms quickly
2. Strategy chip selection has weak visual feedback (only color change)
3. Manual node selection shows all nodes with no search/filter — unusable with 50+ nodes
4. Global Apply button causes "partial not found" errors when strategyConfig contains stale deleted platforms
5. Port form flashes hardcoded defaults ("17990", "Default", true) before persisted values load
6. Port cards cannot be selected for keyboard operations
7. Port form uses 2-column grid wasting vertical space
8. Port cards always expanded showing all details

## Decision

### Q1 — Collapsible platform cards (shadcn Collapsible pattern)

**Collapsed state**: single summary row — platform name + A/B strategy badges (`A:manual[3]` or `B:latency`) + leases/nodes pill + chevron icon.  
**Expanded state**: full A/B split chip selectors with search for manual mode.  
Click the chevron or card header to toggle.

### Q2 — Ring feedback on selected chips (B)

Add `ring-2 ring-primary ring-offset-1` in addition to the existing color change. Provides clear visual boundary around selected state.

### Q3 — Manual node search with hybrid selected-first (A+hybrid)

Inline search input above the node chip list. Typing filters chips by display_tag prefix match. Selected nodes that match the search stay in their original position; selected nodes that DO NOT match the search filter are pulled to a "Selected (N)" section at the top so they remain visible and removable regardless of the filter.

### Q4 — Per-platform sync mode (C)

**Remove** the global Apply button. Each chip change immediately:  
1. Updates local `strategyConfig` state  
2. PUTs the updated config to `egressapikey-strategy.json`  
3. PATCHes the specific platform's `region_filters` on the live Resin sidecar  

This eliminates the stale-platform-batch problem. Additionally, auto-clean: before any strategy operation, filter `strategyConfig.platforms` to only those that exist in the live Resin `platform_list_full()` result.

**Fix `a_class_regions` Manual bug**: The `Manual` case in `strategy_engine.rs` must map `manual_nodes` (node hashes) to region codes by looking up each hash in the healthy nodes list and collecting the unique regions. The GUI writes `manual_nodes`, NOT `regions` — the current code returns `strategy.regions` which is empty for Manual mode.

### Q5 — Port form bootstrap gate (A)

Add `bootstrapped: boolean` state, initialized to `false`. Render a skeleton/div placeholder while `false`. The first `useEffect` loads persisted port form defaults (port, protocol, label, platform, auth) from settings.json, sets all state to the loaded values, then flips `bootstrapped=true`. First paint after bootstrap shows the real persisted values — no flash.

### Q6 — Port card toggle select (A)

`selectedPort: number | null` state. Click a port card toggles selection. Selected card shows `ring-2 ring-primary`. When a port is selected, pressing Delete/Backspace triggers `handleDeletePort`. Clicking the same card again deselects.

### Q7 — Port form compact inline (A)

Replace the 2-column grid with a single flex row: `w-20` (port) + `w-24` (protocol select) + `flex-1` (label input) + `shrink-0` (auth checkbox) + `shrink-0` (add button). One line, no wrapping.

### Q8 — Port card collapsible (A)

Matches Q1 pattern. Collapsed: single row — `port/proto` + health dot + auth icon + delete button + chevron. Expanded: auth credentials + health details + platform binding. Click row to toggle.

## Consequences

- **Positive**: cleaner UX, no stale-platform errors, no default flash, better space utilization, keyboard-friendly port management
- **Negative**: more state management in PlatformsView (bootstrapped, selectedPort, expanded card sets) — mitigated by keeping each concern in its own useState
- **Risk**: per-platform sync means more PATCH calls to Resin — but each is a single platform PATCH, lighter than the old batch apply

## Status: ACCEPTED
