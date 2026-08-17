# Domain Glossary - EgressAPIKEY

> Pair with docs/adr/ for decision records. Terms here are the authoritative *ubiquitous language* of the shell.
> Established T15-v3 via grill; only amend via an ADR addendum when the boundary of a term shifts.

## Platforms and routing

- **Platform** - a named routing group in the Resin sidecar. The user-facing concept for where a set of inbound keys exits. A Platform owns:
  regex_filters (upstream Host allow-list), region_filters (geographic allow-list),
  allocation_policy (Resin-native exit-IP picker enum: BALANCED | PREFER_LOW_LATENCY | PREFER_IDLE_IP),
  and the coarse shell-layer identity used to display strategy badges.
- **StrategyConfig JSON** - the whitebox file egressapikey-strategy.json under the per-user config dir.
  Single source of truth for shell-side strategy *writes* (ADR-0036). The shell layer parses it into PlatformStrategy
  entries (one per Platform name) and calls strategy_apply to PATCH the matching Resin Platform fields.
- **A-class strategy** - which IPs go into this Platform (Manual / Region / Quality / Subscription).
  Lives on PlatformStrategy.a_class in the JSON, *not* derivable from a single Resin field.
  Canvas must read it from strategyConfig to label the badge correctly.
- **B-class strategy** - how the port picks an exit IP for this Platform (Random / Sequential / Latency / Quality / Bandwidth / Protocol-Weight).
  Six shell enum values map (3:1 or N:1) into Resin allocation_policy's three values;
  the JSON stores the shell value, the Resin field stores the mapped enum.
- **config_export / config_import** - the shell-level snapshot of the *Resin* backend state (Platforms + Subscriptions native fields).
  Does NOT contain strategyConfig fields (a_class/b_class/manual_nodes/subscriptions/top_n).
  Two-file boundary is intentional: backend-native config vs shell strategy layer. See ADR-0039 S1.

## Canvas and layout

- **Canvas viewMode** - subscription (default) or region. The toggle selects the *primary grouping dimension*
  of the C column (right side of the A->B->C canvas). Both modes display folded group cards; only the
  A-class badge on the Platform node (B column) tells the user which logical A strategy selected what.
  Switching views is preserved across remounts via topologyState.viewMode (ADR-0039 S2).
- **RegionGroupNode** - a C-column card representing one geographic region. In region viewMode it replaces
  SubscriptionGroupNode as the top-level fold; it must stay *folded* (summary + count) like SubscriptionGroupNode,
  never auto-expand node rows, so hundreds of nodes do not flood the canvas. See ADR-0039 S3.
- **dagre center** - the single correct top-left conversion is pos.x - nodeWidth/2, pos.y - nodeHeight/2
  (dagre returns node centers; ReactFlow uses top-left). No additional graphLabel.width / 2 subtraction -
  that offset drifts as the node count grows. The Home button (setViewport({x:0,y:0,zoom:1})) plus fitView
  on onInit are the two canonical framing primitives. See ADR-0039 S4.

### Canvas interactions

- **selectedRegions** - the set of regions that at least one platform has bound via
  region_filters. Computed by getSelectedRegions(platforms). Controls which region
  group nodes appear in region viewMode; unselected regions are not rendered.
- **port_bind_platform** - the IPC command that persists a port-to-platform binding
  to the SQLite port_mappings table. Called when the user drags a connection from
  an entry-port node to a platform node on the canvas.
- **MiniMap** - the ReactFlow overview component in the bottom-right of the canvas.
  Shows a scaled-down SVG of all nodes with a viewport rectangle. nodeColor
  distinguishes node types (blue=entryPort, purple=platform, green=subscriptionGroup,
  amber=regionGroup). The localized hover tooltip is delivered via the ariaLabel prop
  (SVG <title>), not a <div title> wrapper which has zero hit-test height over an
  absolutely positioned child. See ADR-0041 S3.
- **aggregationColumn** - the single C column in the canonical topology. Holds exactly
  one node type per viewMode: subscriptionGroup nodes when viewMode="subscription",
  regionGroup nodes when viewMode="region". The subGroups build loop is gated by
  viewMode === "subscription" and the regionMap loop by viewMode === "region",
  enforcing one-node-per-entity invariant (Kiali-style). See ADR-0041 S1.
- **nodePool** - the conceptual set of resolved exit nodes from /api/v1/nodes. The
  canvas renders an aggregation of the pool; the same underlying node_hash appears
  only once in any rendered row, regardless of how many subscriptions reference it.
  Dedup happens via Set<node_hash> at both the region path and subscription path.
  See ADR-0041 S2.

## IPC and files

- strategy_config_get / strategy_config_put / strategy_apply - the three IPC commands that form the
  whitebox-JSON pipeline. Read, write, and propagate-to-Resin respectively.
- port_bind_platform - persists a port-to-platform binding to the
  SQLite port_mappings table. Called from canvas drag (entry-port -> platform).
- backup_create - called before any canvas drag-edit, so every whitebox-write traces to a restorable snapshot.
- The strategy JSON path is surfaced in Settings together with the network-whitebox path; both are read-only
  copy + reload after external edit surfaces, not in-app text editors. See ADR-0039 S5.

## T18 — Canvas + Pipeline Sync (ADR-0042)

- **portHealthChip** - the 4-state visual indicator on EntryPortNode showing port liveness: alive (green), degraded (amber), dead (red + card grayed), restarting (blue pulse). Driven by the Rust-side watch_port_health Channel batch probe (sing-box urltest pattern: single ticker + concurrency cap 10 + atomic reentry guard + TTL skip + exponential backoff). Not tied to the 5s sync() poll; disabled ports are skipped. See ADR-0042 S1.
- **portEnabled** - the whitebox egressapikey-ports.json enabled field that controls whether PortForwarder listens on the port. Truth source for port on/off; Resin endpoint untouched. Toggled from PlatformsView switch; canvas EntryPortNode shows grayed + lock when disabled. See ADR-0042 S2.
- **bClassParams** - optional per-platform B-strategy parameters (round_robin_n, latency_threshold_ms, quality_score) stored in strategyConfig JSON. PlatformNode B badge renders them via i18n template interpolation. Whitebox editable. See ADR-0042 S3.
- **manualMode** - the A-class strategy where the user manually selects specific node hashes (not regions). Canvas drag stays group-level; manual selection is via expanded card node-row checkbox. Checkbox toggles node_hash in strategyConfig manual_nodes; strategy_engine maps manual_nodes to regions to PATCH Resin region_filters. No leaf-node handles on canvas (ReactFlow group-no-handle paradigm). See ADR-0042 S4.
- **configEntry** - the right-top floating toolbar on canvas with a FileCog dropdown to open egressapikey-ports.json or egressapikey-strategy.json in the OS default editor. Replaces the left-bottom FileCog. See ADR-0042 S5.
- **whiteboxRestorePorts** - the post-boot_resin logic that rebuilds Resin endpoints from whitebox egressapikey-ports.json on every Resin restart. For each enabled entry_port: POST /api/v1/endpoints (skip on 409 Conflict). Closes the whitebox-is-truth-source loop. See ADR-0042 S6.
