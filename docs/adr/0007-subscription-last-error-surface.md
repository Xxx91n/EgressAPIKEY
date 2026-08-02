# ADR-0007: ADR-0006 item 2 closed-loop audit-then-fix (subscription last_error surface)

Date: 2026-08-02
Status: ACCEPTED
Decision Type: IPC projection + GUI observability

## Context

ADR-0006 item 2 audit-then-fix on the subscription import closed-loop.
Two facts emerged from live-sidecar probes:

1. The old user test URL `https://link123.52pokemon66.cc/...token=79e12...` now
   returns 403 to EVERY tested UA (clash-verge/v2.0.0, clash/v1.18.0,
   ClashForWindows/0.20.39, mihomo/1.18.0, plus Resin own default UA).
   Resin itself reports `last_error: downloader: unexpected status 403`.
   So the URL is source-side dead for this exit IP; the P13 fix pattern
   (shell-side clash-family UA + flow->block convert + local POST) is still
   correct and was re-verified to parse 3 local fixture proxies > 3 nodes.

2. The new URL `https://jinxi2410.qzz.io/cfnew2/sub?target=clash` 200s +
   73276 bytes under clash-verge/v2.0.0; raw local-POST to Resin yields
   `node_count=102` within 12s. End-to-end real loop verified.

The actual hollow point is NOT the import path (it works); it is the IPC
projection. `SubscriptionSnapshotEntry { name, node_count }` dropped
Resin `last_error` / `last_checked` / `healthy_node_count`, so
when a user imports a URL whose source returned 403 the GUI showed a mute
"0 nodes" with no reason. That is the toy UX ADR-0006 item 2 targets.

## Decision

Extend `SubscriptionSnapshotEntry` with three fields:
 - `healthy_node_count: u64` (default 0)
 - `last_error: string` (default empty; Resin uses empty string not null, but the helper handles null too)
 - `last_checked: string` (RFC3339 timestamp)

The GUI renders a secondary line below each subscription row showing
healthy count, a trimmed last_checked timestamp, and (when non-empty) the
last_error text in a rose-tinted span. last_error is technical telemetry and
is rendered verbatim > no translation. healthy count uses the i18n label
`subscription.healthy` as a tag, but the count itself is the literal
number so a fetch 403 cannot masquerade as a mute 0 in any locale.

## Consequences

- A future source 403 / DNS fail / parse error is visible at the row level
  without restarting the GUI or inspecting the log file.
- The `SubscriptionSnapshotEntry` struct widened; the TS interface
  mirrors it. Existing tests that asserted only name+node_count still pass
  (the new fields default to 0/empty when Resin omits them).
- AGENTS S31 scope remaining after item 2 closed: items 3 (A4-3 live-sidecar
  e2e) and 4 (tray i18n half-beat).

## Closed-loop tests

- `subscription_snapshot_reads_resin_items_wrapper_with_node_count`:
  extended to assert defaults when Resin omits the three new fields, plus
  the non-empty last_error path.
- `subscription_snapshot_handles_null_last_error_and_missing_healthy`:
  Resin (or any Go encoder) might emit `null` for an unset string
  pointer; `.as_str().unwrap_or("")` collapses null to empty.
- `SubscriptionsView item 2 row hint`: `shows last_error in red`,
  `shows healthy count when >0 and trims last_checked sub-seconds`.
  Vitest jsdom does not eager-load the i18n resources-to-backend chunks, so
  the matcher allows the raw-key fallback AND classList-based assertions
  (the CSS-selector `.text-rose-600` matches the `dark:text-rose-400`
  modifier incorrectly in jsdom).
- `cargo test -p resin-core --lib` = 79 passed; `pnpm test` =
  76 pass / 8 files (item 2 added 2 new tests; was 74).

## Build

`cargo build --release -p ai-api-route-app --features custom-protocol` >>
11.46MB exe staged at `release/windows-gui/ai-api-route.exe` + 
`resin.exe` sidecar (37.8MB gitignored). Embedded latest Vite chunk
`index-CfSStSwB`. Smoke: MainWindowTitle=ai-api-route, WS 36.4MB,
resin child alive.
