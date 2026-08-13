# GRILL T8 — Platform Page UX Fix (2026-08-13)

> Branch: codex/rust-pass | Supersedes: T7 | Depends on: T5 (trace_id + IpcError), T6 (network layer)

## Q&A Decision Summary

| Q | Topic | Decision | ADR |
|---|---|---|---|
| Q1 | Drag port to platform silently flips auth_required to true | B (new IPC port_bind_platform, not port_upsert) | ADR-0029 |
| Q2 | Drag feedback: text selection + poor visual | A (3-layer: preventDefault + user-select:none + ghost/ring/scale) | ADR-0029 |
| Q3 | A-class strategy hidden behind chevron expand | A (inline badge/chip on card, delete expand panel) | ADR-0029 |
| Q4 | A-class selectors: manual has no IP picker, region/sub are text boxes | A (chip-based multi-select, reuse ipcNodeList + ipcSubscriptionList) | ADR-0029 |
| Q5 | New port auto-assigned to Default platform | A (empty string = unbound, no default platform) | ADR-0029 |

## T8 Execution Plan

| Step | Task | Files | ADR | Test |
|------|------|------|-----|------|
| T8-1 | New IPC port_bind_platform(port, platform_name) — only PATCH endpoint platform binding, no auth_required touch | src-tauri/src/commands/mod.rs, crates/resin-core/src/resin_client.rs | 0029 | 2 cargo mockito (PATCH platform_name only), 1 vitest wrapper forward |
| T8-2 | bindPortToPlatform TS calls ipcPortBindPlatform instead of ipcPortUpsert | src/views/PlatformsView.tsx | 0029 | 1 vitest (bind calls port_bind_platform not port_upsert) |
| T8-3 | Drag 3-layer visual: (1) onPointerDown preventDefault + body userSelect:none (2) source opacity-50 + cursor-grabbing (3) target ring-2 ring-offset-2 scale-[1.02] transition | src/views/PlatformsView.tsx | 0029 | 2 vitest (preventDefault + userSelect toggled) |
| T8-4 | A-class inline badge row on platform card: colored chips showing A:manual / A:region[HK,JP] / A:subscription[alpha] / A:quality top-N | src/views/PlatformsView.tsx | 0029 | 2 vitest (badge renders per type + shows values) |
| T8-5 | Delete expandedPlatform state + chevron + strategy-inline panel; A-class edit via clicking badge popover | src/views/PlatformsView.tsx | 0029 | 1 vitest (no expand/collapse in DOM, badge click opens edit) |
| T8-6 | Chip-based multi-select for A-class: (a) manual= nodeList checkbox chips (b) region= distinct regions toggle chips (c) subscription= subscriptionList toggle chips (d) quality= top_n input + live preview top-3 | src/views/PlatformsView.tsx | 0029 | 4 vitest (one per a_class type, chips render + toggle) |
| T8-7 | handleAddPort: platform_name = newPlatformName.trim() || empty string (unbound); port_upsert Rust accepts empty (skip port_mappings row); UI shows Unbound badge | src/views/PlatformsView.tsx, src-tauri/src/commands/mod.rs | 0029 | 2 vitest (empty creates unbound + badge shows) |
| T8-8 | i18n keys for new UI strings across 18 locales | src/locales/*/platform.json | 0029 | i18n:check green |
| T8-9 | Build exe + stage + smoke + codegraph sync + push | scripts/build-all.ps1 | - | release exe smoke |

Dependency: T8-1 to T8-2, then (T8-3 || T8-4 || T8-6 || T8-7) parallel, then T8-5, then T8-8, then T8-9

## Root cause analysis (code-level)

### Bug 1 (auth flip on drag)
bindPortToPlatform L258 calls ipcPortUpsert without auth_required. Rust port_upsert L1572-1600 defaults auth_required = true. Fix: new port_bind_platform IPC only PATCHes platform_name, never touches auth.

### Bug 2 (text selection + poor feedback)
onPointerDown L360 only calls setDraggingPort, no e.preventDefault(). Browser triggers text selection. Only opacity-60 + ring-2 as feedback. Fix: preventDefault + userSelect:none + cursor-grabbing + target scale/ring/offset.

### Bug 3 (A-class hidden)
Strategy panel defaults to expandedPlatform = null (collapsed). User must click chevron to see A-class. Fix: inline badge row on card, delete expand panel.

### Bug 4 (selectors are text boxes)
manual: no IP picker. region: text input. subscription: text input. Fix: chip-based multi-select from live nodeList + subscriptionList.

### Bug 5 (default platform assignment)
handleAddPort L229: platform_name: newPlatformName.trim() || Default. Fix: empty string = unbound.

## PWM research reference

GPT-5.6 Terra research (conversation ce3f718d): mature proxy GUI patterns for drag-drop, inline badges, chip selectors. Key citations: nngroup.com drag-drop, marvelapp drag-drop design systems, logrocket UX patterns.
