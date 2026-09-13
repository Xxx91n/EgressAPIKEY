# ADR-0040: Canvas region filter + port drag-bind + MiniMap i18n

Status: ACCEPTED
> Date: 2026-08-16
> Depends on: ADR-0039 (canvas v3 fold + strategy sync)

## Context

Three bugs surfaced after T15-v3 canvas commit (db257ad):

1. Region viewMode generates region group cards for ALL regions that have nodes,
   regardless of whether any platform selected them via region_filters. This floods
   the canvas with hundreds of unbound region cards and causes lag.

2. Canvas drag from entry-port to platform does nothing. onConnect only handles
   platform -> subgroup/regiongroup connections. Port-to-platform binding exists as
   an IPC command (port_bind_platform) but is not wired into canvas drag.

3. MiniMap component has no i18n tooltip; hover over the minimap area shows no
   localized guidance. MiniMap also uses default grey nodeColor, making nodes
   indistinguishable in the overview.

## S1 Region viewMode: only show platform-bound regions

Region group nodes are only generated when at least one platform has the region in
its region_filters. Implementation: add a selectedRegions.has(region) guard
before pushing a regiongroup node into the node list. This mirrors the existing
subscription viewMode filter at L705.

Rejected: showing all regions with reduced opacity (Kiali idle/active pattern).
Rejected because the user explicitly chose A: "only show regions the platform
strategy actually selects."

## S2 Port drag-bind: wire entry-port -> platform into onConnect

Add a new branch in onConnect: when conn.source.startsWith("entry-port-") and
conn.target.startsWith("platform-"), extract the port number from the source
id and the platform name from the target id, then call ipcPortBindPlatform(port,
platformName) followed by sync().

This reuses the existing Rust port_bind_platform IPC command (L1951 in
commands/mod.rs) and the TS wrapper ipcPortBindPlatform (L425 in ipc.ts). No
new IPC needed.

Rejected: auto-creating edges without IPC. The port-platform binding must persist
to SQLite port_mappings; without the IPC call the edge would vanish on next sync.

## S3 MiniMap: i18n tooltip + nodeColor customization

Wrap <MiniMap> in a <div title={t("topology.minimapHint")}> to provide a
localized hover tooltip. Add 
odeColor function that colors nodes by type:
entryPort = blue, platform = purple, subscriptionGroup = green, regionGroup =
amber. Add maskColor for a dark viewport overlay. No new dependency; uses
ReactFlow 12 built-in MiniMap props.

Rejected: replacing MiniMap with a custom overview panel. Ponytail: MiniMap
already works; custom replacement adds ~200 lines for zero new capability.

## Consequences

- TopologyView.tsx: ~15 lines (region filter guard + onConnect port branch +
  MiniMap wrapper + nodeColor function)
- onConnect: new entry-port -> platform branch (~10 lines)
- MiniMap: wrap in div + nodeColor prop (~5 lines)
- New i18n keys: topology.minimapHint x18 locales
- Canvas entry: "open strategy config" button (~10 lines, reuses ipcGetConfigDir +
  openPath)
- Tests: 3 new vitest assertions (region filter guard, port drag-bind forward,
  MiniMap tooltip renders i18n text)