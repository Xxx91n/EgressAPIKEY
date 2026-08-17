# ADR-0042: Canvas + Pipeline Sync — Port Health, Enable Switch, B-Strategy Params, Manual Mode, Config Entry, Whitebox Restore

> Status: ACCEPTED
> Date: 2026-08-17
> Supersedes: none (builds on ADR-0036 strategyConfig single source, ADR-0039 canvas fold, ADR-0041 canvas v4)
> Sources: atomcode research Q8 (sing-box urltest.go + Kiali cache + mihomo backoff), atomcode research Q16 (mihomo select vs filter + ReactFlow group no-handle + Kiali read-only canvas)

## S1: Port health chip — batch probe, not per-port poll

Decision: Rust side new `watch_port_health` Channel command. Single Tokio task: ticker interval = max(5s, k*ln(1+N)), batch probe with concurrency cap 10, atomic reentry guard, TTL skip, exponential backoff for dead ports (5 consecutive fails to Dead, backoff capped at 5m), pause when tab hidden. Front-end single subscription + Map update + React.memo.

Rejected: per-port 5s sync() poll (IPC flood when N>100), push-only WebSocket (overkill for discrete snapshots), per-port on-hover probe (not continuous enough for topology view).

Consequence: Port health is a background stream, not tied to the 5s sync() poll. EntryPortNode shows 4-state chip (alive/degraded/dead/restarting) + lock icon for auth_required. Disabled ports (enabled=false) are skipped by the probe.

Evidence: sing-box urltest.go (batch.New concurrency 10, atomic checking swap, history TTL skip), Kiali 3m health cache + 60s UI refresh, mihomo max-failed-times:5, Cilium CFP-32820 interval = k*ln(1+n), MechanicalRock Tauri IPC benchmark (Channel 50ms vs 891ms base64).

## S2: Port enable/disable — PortForwarder controls listen, whitebox is truth

Decision: New `port_toggle(port, enabled)` IPC command. Updates whitebox egressapikey-ports.json `enabled` field (atomic write). PortForwarder listens when enabled=true, stops listening when enabled=false. Resin endpoint untouched.

Rejected: DELETE+POST Resin endpoint on each toggle (state churn), fork Resin to add enabled field.

Consequence: Whitebox `enabled` is the truth source for port on/off. Resin keeps its endpoint config; PortForwarder controls the actual TCP listen. Disabled ports show grayed + lock on canvas.

## S3: B-strategy parameter display — i18n templates with interpolation

Decision: PlatformStrategy gains `b_class_params: BClassParams` (optional round_robin_n, latency_threshold_ms, quality_score). PlatformNode B badge uses per-strategy i18n template with param interpolation. Per-strategy param input in PlatformsView strategy panel.

Rejected: B badge text hardcoded English, params in tooltip only, no param input.

Consequence: B badge shows localized parameter text. Params stored in strategyConfig JSON, whitebox editable.

## S4: Manual mode — card checkbox, not canvas drag

Decision: Canvas drag stays group-level only (platform to subgroup/regiongroup, entry to platform). Manual mode selection via expanded region/subscription card node-row checkbox. Checkbox toggles node_hash in strategyConfig `manual_nodes`. strategy_engine maps manual_nodes to regions to PATCH Resin region_filters. No leaf-node handles on canvas.

Rejected: drag platform to leaf node (violates ReactFlow group-no-handle paradigm, no Resin per-node API), fork Resin for per-node binding.

Consequence: Two orthogonal A-strategy paths: region/subscription = canvas drag (group-level), manual = card checkbox (node-level). Both funnel into region_filters PATCH via strategy_engine. Canvas drag guard rejects leaf to platform connections.

Evidence: mihomo select group (manual = runtime member selection) vs filter group (region = name regex), sing-box selector (Clash API runtime), ReactFlow group type no handles attached, Kiali topology graph is not config editor, Cytoscape multi-select batch edges.

## S5: Config-file entry — right-top toolbar dropdown

Decision: Remove FileCog from left-bottom CanvasControls. Add right-top floating toolbar with FileCog + dropdown menu: open ports config opens egressapikey-ports.json, open strategy config opens egressapikey-strategy.json. Both via ipcGetConfigDir + openPath.

Rejected: left-bottom keep (vertical toolbar too long, FileCog buried), settings-page only (no canvas entry).

Consequence: Canvas has one config access point, top-right, not obstructing nodes.

## S6: Resin restart port restore from whitebox

Decision: After boot_resin success in main.rs setup(), call restore_ports_from_whitebox. For each whitebox entry_port: if enabled then POST /api/v1/endpoints to Resin (skip on 409 Conflict). This closes the whitebox is truth source loop.

Rejected: rely on GUI sync() to recreate ports (race condition if user opens app before sync), Resin self-persist (whitebox not authoritative).

Consequence: Resin restart + whitebox rebuild = deterministic state recovery. Boot log traces each port restore.
