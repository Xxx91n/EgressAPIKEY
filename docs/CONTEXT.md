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

## IPC and files

- strategy_config_get / strategy_config_put / strategy_apply - the three IPC commands that form the
  whitebox-JSON pipeline. Read, write, and propagate-to-Resin respectively.
- backup_create - called before any canvas drag-edit, so every whitebox-write traces to a restorable snapshot.
- The strategy JSON path is surfaced in Settings together with the network-whitebox path; both are read-only
  copy + reload after external edit surfaces, not in-app text editors. See ADR-0039 S5.
