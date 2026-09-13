# ADR-0029: Platform Page UX Fix — Drag, Strategy Display, Unbound Ports

Status: ACCEPTED (2026-08-13)

## Context

Five bugs on PlatformsView.tsx identified during user testing:
1. Dragging a no-auth port to a platform silently flips auth_required to true (bindPortToPlatform calls ipcPortUpsert without auth_required, Rust defaults to true)
2. Drag feedback is poor (only opacity-60 + ring-2) and triggers text selection (no preventDefault)
3. A-class strategy hidden behind chevron expand, not visible on card
4. A-class selectors (manual/region/subscription) are plain text boxes, no pickers
5. New ports auto-assigned to Default platform (hardcoded fallback)

PWM research (GPT-5.6 Terra, conversation ce3f718d) confirms mature proxy GUIs use: chip-based multi-select, inline badges, 3-layer drag feedback (preventDefault + userSelect:none + ghost/ring/scale).

## Decision

1. New IPC port_bind_platform(port, platform_name) that only PATCHes the Resin endpoint platform binding, never touches auth_required. bindPortToPlatform TS calls this instead of ipcPortUpsert.
2. Drag onPointerDown: e.preventDefault() + document.body.style.userSelect=none. Source card: opacity-50 + cursor-grabbing. Target card: ring-2 ring-offset-2 scale-[1.02] transition.
3. A-class strategy shown as inline badge chips on platform card (A:manual, A:region[HK,JP], etc). Delete expandedPlatform state + chevron + strategy-inline panel.
4. A-class selectors replaced with chip-based multi-select: manual= nodeList checkbox chips, region= distinct regions toggle chips, subscription= subscriptionList toggle chips, quality= top_n input + live preview.
5. handleAddPort: platform_name = newPlatformName.trim() || empty string (unbound). port_upsert Rust accepts empty platform_name (skip port_mappings row). UI shows Unbound badge.

## Consequences

- port_upsert no longer the only write path; port_bind_platform is the binding-only path
- Strategy panel simplified: no expand/collapse, all info visible inline
- Chip selectors require nodeList + subscriptionList data already available via existing IPC
- Unbound ports require UI badge to communicate state to user
