# ADR-0039: Canvas v3 - fold consistency, dagre center correction, strategy badge sync, two-file whitebox boundary

Status: ACCEPTED (T15-v3 grill, 2026-08-16)
Supersedes parts of: ADR-0034 (canvas v2 dagre-strategy-layout) - S4 corrects the center-offset formula introduced there;
ADR-0036 (strategy single-source-of-truth) - S2/S3 extends the sync contract to *reading* strategy fields, not just writing.

## S1 Two-file whitebox boundary

Keep config_export/config_import and egressapikey-strategy.json as two separate files.
The first contains Resin-native Platform + Subscription fields (regex_filters, region_filters, allocation_policy, sticky_ttl, subscription url).
The second contains shell-layer strategy identity (a_class, b_class, manual_nodes, subscriptions, top_n).
Settings surfaces BOTH paths with reload buttons; no in-app text editor (Ponytail: users who whitebox-edit use an external editor; a JSON textarea adds validation surface without removing the risk).
Rejected: merging into one JSON. The backend vs strategy-layer boundary is the whole point of the split; merging hides it.

## S2 Canvas reads strategy fields (ADR-0036 read-side extension)

ADR-0036 pinned the *write* pipeline (drag -> strategyConfig JSON -> apply -> Resin PATCH). The *read* side was left implicit and rotted:
TopologyView computed aClassLabel from p.region_filters?.length alone - i.e. only the Region variant was ever displayed; Manual / Quality / Subscription all rendered as Manual.
Fix: PlatformFull gains aClass, bClass, manualNodes, subscriptionNames, topN; sync() merges these from strategyConfig the same way it already merges regions.
aClassLabel and bClassLabel are then computed from the merged fields via strategyToI18nKey-equivalent mappings, not inferred from region_filters.
Closed loop: change a_class/b_class in the JSON -> reload -> canvas badge updates. Change in PlatformsView -> save -> same path.

## S3 Region viewMode fold consistency

The region/subscription toggle is preserved (Q1 = A) because the two viewModes serve different tasks: subscription mode traces which subscription feeds which nodes, region mode traces which geographic areas the Platform covers.
Current bug: region mode auto-expands every region group into per-node rows, flooding the canvas with hundreds of cards and causing the lag + clutter symptom.
Fix: RegionGroupNode keeps the same fold contract as SubscriptionGroupNode - collapsed summary (region label + healthy/total count), expandable on click only. No default expansion. Auto-expand when there is only one region is rejected: it breaks the stable-layout guarantee.

## S4 dagre center correction

Remove the offsetX = graphLabel.width / 2 line in layoutNodesViaDagre (T15-4 introduced it as a center to origin hack).
The authoritative formula per kimi_k26 research (cross-checked against the official ReactFlow + dagre example) is:
position = { x: pos.x - nodeWidth/2, y: pos.y - nodeHeight/2 } - pure center-to-top-left conversion.
dagre internal marginx/marginy: 20 already reserves canvas breathing room; no extra graph-width subtraction.
Effect: setViewport({x:0,y:0,zoom:1}) now shows the graph top-left margin corner (~ origin), and fitView({maxZoom:1}) still auto-frames on init.
The previous drift (more nodes = wider graph = bigger offset = visible shift away from Home) disappears.

## S5 Settings whitebox reload surfaces

Settings already has the network-whitebox reload + path display. Add the same pattern for the strategy JSON:
- show egressapikey-strategy.json absolute path (read-only label)
- Reload from disk button that re-reads the file into the in-memory state and calls strategy_apply so the GUI reflects external edits
- no text editor widget, no save button on this surface (edits go through external editor or through PlatformsView GUI)
This makes the whitebox-everything-in-files mental model visible from inside the GUI without embedding a code editor.

## Consequences

- ~40 lines changed in TopologyView.tsx (PlatformFull widening + sync merge extension + aClass/bClass label computation + dagre offset deletion)
- ~15 lines in RegionGroupNode (fold contract alignment)
- ~10 lines in SettingsView.tsx (strategy JSON path + reload, mirroring the existing network-whitebox card)
- New i18n keys for A-class label variants (topology.aClassManual / topology.aClassQuality / topology.aClassSubscription) across all 18 locales
- Tests: existing T15-3/T15-5 plus new assertions that a non-region a_class renders the correct badge, and that Home centers the graph regardless of node count.
