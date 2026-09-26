import { useTranslation } from "react-i18next";
import i18n from "i18next";
import {
  ReactFlow, Background, BackgroundVariant, MiniMap,
  Handle, Position, type Node, type Edge, type Connection, type NodeProps,
  useReactFlow, ReactFlowProvider, type OnMoveEnd,
} from "@xyflow/react";
import { useEffect, useMemo, useState, useCallback, useRef, memo } from "react";
import { create } from "zustand";
import { shallow } from "zustand/shallow";
import dagre from "dagre";
import "@xyflow/react/dist/style.css";
import { useAppStore } from "../store/appStore";
import {
  ipcNodeList, ipcPlatformUpdate, ipcBackupCreate,
  ipcLeaseMap, type LeaseEntry, type PortMapping,
  ipcStrategyApply, ipcStrategyPlatformRegionsSet,
  ipcStrategyPlatformSubscriptionsSet, ipcStrategyPlatformManualNodesSet,
  ipcPortBindPlatform, ipcGetConfigDir,
  ipcWatchPortHealth, type PortHealthEntry,
  ipcAuthoritativeSnapshot, type AuthoritativeSnapshot,
} from "../lib/ipc";
import { loadTopologyState, saveTopologyState, type TopologyState as PersistedTopologyState } from "../lib/settings";
import { mapResinToShell, bClassLabel as bClassLabelFn, type StrategyId, type AllocationPolicy } from "../lib/strategy";
import { listen } from "@tauri-apps/api/event";
import type { ColorMode } from "@xyflow/react";
import { AlertTriangle, Loader2, ZoomIn, ZoomOut, Maximize, Lock, Unlock, Home, FileCog, ChevronDown } from "lucide-react";
import { usePoll } from "../hooks/usePoll";
import { HeadlessCapabilityNotice } from "../components/HeadlessCapabilityNotice";

/// translate helper for use inside memoized nodes (no React context).
const tFn = (k: string) => i18n.t(k);

/// TopologyView — Canvas V2: strategy-driven dagre layout + flash fix + zustand cache.
///   A (Entry proxy port) --> B (Platforms) --strategy match--> C (IP channels / nodes)
///
/// C column only shows nodes targeted by at least one platform region_filters.
/// dagre auto-layout replaces hardcoded positions.
/// Handle fixed at card edge midpoint (not full-area overlay).
/// Empty-state messages inside opacity gate; sync() resolves then setReady.
/// topologyStore (zustand + shallow) caches sync data; 5s poll restored.

interface PlatformFull {
  id: string;
  name: string;
  regex_filters: string[] | null;
  region_filters: string[] | null;
  allocation_policy: string;
  routable_node_count: number;
  sticky_ttl: string;
  // strategyConfig read-side fields (ADR-0036 read-side, ADR-0039 SS2)
  aClass?: string;              // strategyConfig a_class: manual | region | quality | subscription
  bClass?: string;             // strategyConfig b_class == the Resin allocation_policy value
  subscriptionNames?: string[]; // strategyConfig subscriptions
  topN?: number;               // strategyConfig top_n
  manualNodes?: string[];        // strategyConfig manual_nodes
}

interface NodeItem {
  name?: string;
  display_tag?: string;
  node_hash?: string;
  has_outbound?: boolean;
  failure_count?: number;
  region?: string;
  tags?: Array<{ subscriptionName?: string; subscription_name?: string; tag?: string }>;
}

interface SubscriptionGroup {
  subscriptionName: string;
  nodes: NodeItem[];
  healthy: number;
  total: number;
  regions: string[];
}

// --- topologyStore (zustand + shallow) ---
interface TopologyState {
  platforms: PlatformFull[];
  subGroups: SubscriptionGroup[];
  leases: LeaseEntry[];
  ports: PortMapping[];
  portHealth: Record<number, PortHealthEntry>;
  setPlatforms: (p: PlatformFull[]) => void;
  setSubGroups: (s: SubscriptionGroup[]) => void;
  setLeases: (l: LeaseEntry[]) => void;
  setPorts: (p: PortMapping[]) => void;
  setPortHealth: (m: Record<number, PortHealthEntry>) => void;
}

export const useTopologyStore = create<TopologyState>((set) => ({
  platforms: [],
  subGroups: [],
  leases: [],
  ports: [],
  portHealth: {},
  setPlatforms: (p) => set((s) => (shallow(s.platforms, p) ? s : { platforms: p })),
  setSubGroups: (sg) => set((s) => (shallow(s.subGroups, sg) ? s : { subGroups: sg })),
  setLeases: (l) => set((s) => (shallow(s.leases, l) ? s : { leases: l })),
  setPorts: (p) => set((s) => (shallow(s.ports, p) ? s : { ports: p })),
  setPortHealth: (m) => set((s) => (shallow(s.portHealth, m) ? s : { portHealth: m })),
}));

/// (ADR-0048 S1+S3+S4): a_class-semantic node filter helper.
/// Exported for vitest.
/// @param node       NodeItem from a subscription group
/// @param platforms  PlatformFull[] - the live canvas platforms
/// @param groupKey   subscriptionName (subscription viewMode) or region string (region viewMode)
/// @returns true if ANY platform a_class semantics select this node
export function isNodeSelectedByAnyPlatform(
  node: NodeItem,
  platforms: PlatformFull[],
  groupKey: string,
): boolean {
  const nodeRegion = getNodeRegion(node);
  const nodeHash = typeof node.node_hash === "string" ? node.node_hash : "";
  for (const p of platforms) {
    const aClass = p.aClass ?? "manual";
    switch (aClass) {
      case "subscription": {
        const subs = p.subscriptionNames ?? [];
        if (subs.length === 0) return true;
        if (subs.includes(groupKey)) return true;
        break;
      }
      case "quality": {
        if ((node.failure_count ?? 0) === 0 && node.has_outbound !== false) return true;
        break;
      }
      case "region": {
        const regs = (p.region_filters ?? []).map((r) => r.toLowerCase());
        if (regs.includes(nodeRegion)) return true;
        break;
      }
      case "manual":
      default: {
        const manual = p.manualNodes ?? [];
        if (nodeHash && manual.includes(nodeHash)) return true;
        break;
      }
    }
  }
  return false;
}

// --- parse nodes grouped by subscription source ---
function parseSubscriptionGroups(raw: unknown): SubscriptionGroup[] {
  if (!raw || typeof raw !== "object") return [];
  const v = raw as Record<string, unknown>;
  const items: NodeItem[] = Array.isArray(v.items)
    ? v.items.filter((x): x is NodeItem => !!x && typeof x === "object")
    : Array.isArray(v)
      ? v.filter((x): x is NodeItem => !!x && typeof x === "object")
      : [];
  const bySub = new Map<string, NodeItem[]>();
  for (const n of items) {
    let subName = "unknown";
    if (Array.isArray(n.tags)) {
      // (ADR-0077, recovered ADR-0051 defect A): Resin /api/v1/nodes returns
      // snake_case subscription_name; a camel-only lookup made every group
      // fall back to the per-node protocol tag, so strategyConfig
      // subscriptions never matched any group (no B->C edges).
      const subTag = n.tags.find((t) => t.subscriptionName ?? t.subscription_name);
      if (subTag) subName = (subTag.subscriptionName ?? subTag.subscription_name)!;
      else if (n.tags.length > 0 && n.tags[0].tag && n.tags[0].tag.length > 3) subName = n.tags[0].tag;
    }
    const arr = bySub.get(subName) ?? [];
    arr.push(n);
    bySub.set(subName, arr);
  }
  const groups: SubscriptionGroup[] = [];
  for (const [subscriptionName, nodes] of bySub) {
    const healthy = nodes.filter((n) => (n.failure_count ?? 0) === 0 && n.has_outbound !== false).length;
    const regions = [...new Set(nodes.map((n) => {
      if (n.region) return n.region.toLowerCase();
      if (Array.isArray(n.tags)) {
        const rt = n.tags.find((t) => t.tag && t.tag.length <= 3);
        if (rt) return rt.tag!.toLowerCase();
      }
      return "other";
    }))].sort();
    groups.push({ subscriptionName, nodes, healthy, total: nodes.length, regions });
  }
  return groups.sort((a, b) => a.subscriptionName.localeCompare(b.subscriptionName));
}

/// (ADR-0041 S2): dedup a list of nodes by node_hash.
/// - Empty/missing node_hash means "best-effort keep" (no dedup).
/// - Returns a stable-order array with duplicates removed; first occurrence wins.
/// Exported so a vitest can assert the contract without driving the DOM.
export function dedupNodesByHash<T extends { node_hash?: string }>(nodes: readonly T[]): T[] {
  const seen = new Set<string>();
  const out: T[] = [];
  for (const n of nodes) {
    const h = typeof n.node_hash === "string" ? n.node_hash : "";
    if (!h) { out.push(n); continue; }
    if (seen.has(h)) continue;
    seen.add(h);
    out.push(n);
  }
  return out;
}

// extract region from a node — deduplicated from 6 inline copies
function getNodeRegion(n: NodeItem): string {
  if (n.region) return n.region.toLowerCase();
  if (Array.isArray(n.tags)) {
    const rt = n.tags.find((t) => t.tag && t.tag.length <= 3);
    if (rt) return rt.tag!.toLowerCase();
  }
  return "other";
}
/// helper: build C-column subscriptionGroup/regionGroup nodes.
/// (ADR-0041 S1): C-column builder extracted from rawNodes useMemo so a vitest
/// can directly assert the viewMode guard without driving jsdom-rendered ReactFlow nodes
/// (jsdom does not stamp subscriptionGroup/regionGroup DOM nodes; textContent assertions
/// are unreliable because RegionGroupNode renders subscription name sub chips). The helper
/// is pure: same inputs -> same outputs, no React, no i18n re-init side effects.
/// @param t  i18next t() from the calling closure (already initialised in useMemo deps).
export function buildCColumnGroups(
  viewMode: "subscription" | "region",
  subGroups: SubscriptionGroup[],
  platforms: PlatformFull[],
  t: (key: string, opts?: Record<string, unknown>) => string,
): Node[] {
  const list: Node[] = [];
  // (ADR-0041 S1): viewMode guard — only push subscriptionGroup nodes when
  // viewMode === "subscription". In region viewMode they would be edgeless and
  // dagre would scatter them near the entry-port column (the "port column stray
  // nodes" bug). Region viewMode builds its own regionGroup nodes below.
  subGroups.forEach((g) => {
    if (viewMode !== "subscription") return;
    const healthLabel = g.healthy === g.total
      ? t("topology.healthy")
      : g.healthy + "/" + g.total + " " + t("topology.healthy");
    const regionsLabel = g.regions.length > 0
      ? g.regions.slice(0, 5).join(", ").toUpperCase() + (g.regions.length > 5 ? "+" : "")
      : "";
    const sub = [healthLabel, regionsLabel].filter(Boolean).join(" * ");
    // filter node rows — only show nodes in selected regions
    const filteredNodes = g.nodes.filter((n) => isNodeSelectedByAnyPlatform(n, platforms, g.subscriptionName));
    // hide subscription group entirely if no nodes are selected by any platform
    if (filteredNodes.length === 0) return;
    const unboundCount = g.nodes.length - filteredNodes.length;
    // compute region stats for collapsed view
    const regionStatsMap = new Map<string, number>();
    for (const n of g.nodes) {
      const r = getNodeRegion(n);
      regionStatsMap.set(r, (regionStatsMap.get(r) ?? 0) + 1);
    }
    const regionStatsArr = [...regionStatsMap.entries()].sort((a, b) => b[1] - a[1]).map(([region, count]) => ({ region: region.toUpperCase(), count }));
    // (ADR-0041 S2): dedup by node_hash within this subscription.
    const dedupNodes = dedupNodesByHash(filteredNodes);
    const nodeRows = dedupNodes.map((n) => {
      const isHealthy = (n.failure_count ?? 0) === 0 && n.has_outbound !== false;
      const region = getNodeRegion(n).toUpperCase();
      const latencyColor = isHealthy ? "bg-emerald-500" : "bg-red-500";
      return {
        display_tag: n.display_tag || n.name || "node",
        region: region.toUpperCase(),
        healthy: isHealthy,
        latencyColor,
      };
    });
    list.push({
      id: "subgroup-" + g.subscriptionName,
      type: "subscriptionGroup",
      position: { x: 0, y: 0 },
      data: { label: g.subscriptionName, sub, nodes: nodeRows, unboundCount, regionStats: regionStatsArr },
    });
  });
  // ponytail: S2 helper `dedupNodesByHash` (defined near the top of this file) is only called from
  // the subscription route above + its unit test, not from this region route. Region loop simultaneously
  // builds regionMap (total/healthy/subs/nodeRows) while deduping by node_hash via the seenGlobal Set below.
  // Fusing these into the helper would couple aggregation into a dedup-only function and grow the regression
  // surface. Reuse the helper here only when the two concerns can be cleanly separated — tracked in
  // docs/PONYTAIL_DEBT_LEDGER.md. S2 contract honored, different shape.
  // region view mode — build region group nodes
  if (viewMode === "region") {
    const regionMap = new Map<string, { total: number; healthy: number; subs: Set<string>; nodeRows: Array<{ display_tag: string; region: string; healthy: boolean; latencyColor: string }> }>();
    // (ADR-0041 S2): dedup across subscriptions by node_hash.
    // The same node_hash can appear under multiple subscription names (Resin
    // echoes proxies), which previously produced duplicate rows in one
    // regionGroup card.
    const seenGlobal = new Set<string>();
    for (const g of subGroups) {
      for (const n of g.nodes) {
        const h = typeof n.node_hash === "string" ? n.node_hash : "";
        if (h && seenGlobal.has(h)) continue;
        if (h) seenGlobal.add(h);
        const r = getNodeRegion(n);
        const entry = regionMap.get(r) ?? { total: 0, healthy: 0, subs: new Set<string>(), nodeRows: [] };
        entry.total++;
        if ((n.failure_count ?? 0) === 0 && n.has_outbound !== false) entry.healthy++;
        entry.subs.add(g.subscriptionName);
        const isHealthy = (n.failure_count ?? 0) === 0 && n.has_outbound !== false;
        const latencyColor = isHealthy ? "bg-emerald-500" : "bg-red-500";
        entry.nodeRows.push({
          display_tag: n.display_tag || n.name || "node",
          region: r.toUpperCase(),
          healthy: isHealthy,
          latencyColor,
        });
        regionMap.set(r, entry);
      }
    }
    for (const [region, info] of regionMap) {
      // (ADR-0048 S1): a_class-semantic region filter
      const anySelected = platforms.some((p) => {
        const aClass = p.aClass ?? "manual";
        if (aClass === "quality") return true;
        if (aClass === "subscription") {
          const subs = p.subscriptionNames ?? [];
          if (subs.length === 0) return true;
          return subGroups.some((sg) => subs.includes(sg.subscriptionName) && sg.regions.includes(region));
        }
        if (aClass === "region") {
          return (p.region_filters ?? []).map((r) => r.toLowerCase()).includes(region);
        }
        return false;
      });
      if (!anySelected) continue;
      list.push({
        id: "regiongroup-" + region,
        type: "regionGroup",
        position: { x: 0, y: 0 },
        data: { label: region.toUpperCase(), total: info.total, healthy: info.healthy, subs: [...info.subs], nodes: info.nodeRows },
      });
    }
  }
  return list;
}



/// (Authoritative Snapshot): map the pre-merged snapshot entries to
/// the canvas PlatformFull shape. Strategy intent fields now come from the
/// snapshot (merged in resin-core at the sanctioned merge point); a divergent
/// entry carries BOTH values and the canvas shows the whitebox intent while
/// the divergence is reported by the snapshot consumer, never reconciled here.
export function snapshotToPlatformFulls(snap: AuthoritativeSnapshot): PlatformFull[] {
  return snap.platforms
    .filter((ps) => typeof ps.platform_name === "string" && ps.platform_name.length > 0)
    .map((ps): PlatformFull => {
      // ADR-0039 SS2: ALL strategy fields travel with the snapshot; the canvas
      // never consults strategyConfig itself anymore.
      const common = {
        id: ps.platform_id || ps.platform_name,
        name: ps.platform_name,
        regex_filters: null,
        aClass: ps.a_class || undefined,
        bClass: ps.b_class || undefined,
        subscriptionNames: ps.subscriptions.length > 0 ? ps.subscriptions : undefined,
        manualNodes: ps.manual_nodes.length > 0 ? ps.manual_nodes : undefined,
      };
      if (ps.state === "consistent") {
        return {
          ...common,
          region_filters: ps.regions,
          allocation_policy: ps.resin_allocation_policy,
          routable_node_count: 0,
          sticky_ttl: "0s",
        };
      }
      if (ps.state === "divergent") {
        // Whitebox intent wins for canvas display (it is what the next
        // strategy_apply would enforce); both values live in the snapshot.
        return {
          ...common,
          region_filters: ps.whitebox_regions.length > 0 ? ps.whitebox_regions : ps.resin_regions,
          allocation_policy: ps.resin_allocation_policy,
          routable_node_count: 0,
          sticky_ttl: "0s",
        };
      }
      return {
        ...common,
        region_filters: ps.regions,
        allocation_policy: "BALANCED",
        routable_node_count: 0,
        sticky_ttl: "0s",
      };
    });
}
/// map the snapshot's port half into the store's PortMapping[] so
/// the entry-port column keeps its existing rendering contract. Resin runtime
/// agreement is already reflected in each state tag; the canvas only needs
/// identity + enabled to render.
export function snapshotToPortMappings(snap: AuthoritativeSnapshot): PortMapping[] {
  return snap.ports.map((ps) => ({
    port: ps.port,
    protocol: ps.protocol || "mixed",
    platform_name: ps.platform_name,
    account: ps.account || "port-" + ps.port,
    label: ps.label,
    enabled: ps.state === "consistent" ? ps.enabled : true,
    auth_required: ps.auth_required,
  }));
}



// --- dagre auto-layout helper ---
export function layoutNodesViaDagre(nodes: Node[], edges: Edge[], nodeWidth = 200, nodeHeight = 100): Node[] {
  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir: "LR", ranksep: 80, nodesep: 40, marginx: 20, marginy: 20 });
  g.setDefaultEdgeLabel(() => ({}));
  // (ADR-0077, recovered ADR-0051 defect B): invisible column anchors pin the
  // ADR-0002 three-column contract (entryPort=col0, platform=col1,
  // C-group=col2) even when real edges are missing - a port-less platform or
  // an unmatched subscription otherwise leaves nodes disconnected and dagre
  // stacks them at rank 0 below the platform column. Anchors exist only in
  // this dagre graph (minlen:0 = same rank); never returned as ReactFlow nodes.
  const colOf = (n: Node) => (n.type === "entryPort" ? 0 : n.type === "platform" ? 1 : 2);
  const anchors = ["__col0", "__col1", "__col2"];
  for (const a of anchors) g.setNode(a, { width: 1, height: 1 });
  g.setEdge(anchors[0], anchors[1]);
  g.setEdge(anchors[1], anchors[2]);
  for (const n of nodes) {
    g.setNode(n.id, { width: nodeWidth, height: nodeHeight });
    g.setEdge(anchors[colOf(n)], n.id, { minlen: 0 });
  }
  for (const e of edges) {
    g.setEdge(e.source, e.target);
  }
  dagre.layout(g);
  // authoritative dagre → ReactFlow position formula (pos.x - nodeWidth/2 only).
  // The graphLabel.width/2 subtraction was a known offset-drift bug (ADR-0039 SS4).
  return nodes.map((n) => {
    const pos = g.node(n.id);
    if (pos) {
      return { ...n, position: { x: pos.x - nodeWidth / 2, y: pos.y - nodeHeight / 2 } };
    }
    return n;
  });
}

// --- fixed-edge Handle style (replaces full-area overlay) ---
export const fixedHandleStyle: React.CSSProperties = {
  width: 12,
  height: 12,
  background: "#3b82f699",
  border: "1px solid #3b82f6",
  zIndex: 5,
};

/// Custom node: Entry port (A column).
const EntryPortNode = memo(function EntryPortNode({ data }: NodeProps) {
  const d = data as Record<string, unknown>;
  const port = typeof d.port === "number" ? d.port : 0;
  const healthState = typeof d.healthState === "string" ? (d.healthState as "alive" | "degraded" | "dead" | "restarting") : "alive";
  const authRequired = typeof d.authRequired === "boolean" ? d.authRequired : false;
  const enabled = typeof d.enabled === "boolean" ? d.enabled : true;
  // 4-state chip — alive=green, degraded=amber, dead=red+grayed, restarting=blue pulse
  const dot: Record<string, string> = {
    alive: "bg-emerald-500",
    degraded: "bg-amber-500",
    dead: "bg-red-500",
    restarting: "bg-blue-500 animate-pulse",
  };
  const healthLabelKey: Record<string, string> = {
    alive: "topology.portAlive",
    degraded: "topology.portDegraded",
    dead: "topology.portDead",
    restarting: "topology.portRestarting",
  };
  const OpacityClass = enabled ? "" : "opacity-50";
  return (
    <div className={`relative rounded-lg border border-blue-400 dark:border-blue-600 bg-blue-50 dark:bg-blue-950/50 px-4 py-3 text-xs min-w-[140px] max-w-[200px] ${OpacityClass}`}>
      <Handle type="source" position={Position.Right} style={fixedHandleStyle} />
      <div className="flex items-center gap-1.5">
        <span title={tFn(healthLabelKey[healthState])} className={`inline-block w-2 h-2 rounded-full ${dot[healthState]}`} />
        <span className="font-semibold text-blue-700 dark:text-blue-300">
          {port > 0 ? String(port) : String(d.label)}
        </span>
        {authRequired ? (
          <Lock className="w-3 h-3 text-blue-500/70 dark:text-blue-400/70" aria-label={tFn("topology.portAuthRequired")} />
        ) : (
          <Unlock className="w-3 h-3 text-emerald-500/70 dark:text-emerald-400/70" aria-label={tFn("topology.portAuthNotRequired")} />
        )}
      </div>
      <div className="text-blue-600/70 dark:text-blue-400/70 mt-0.5 text-[10px]">
        {typeof d.protocol === "string" ? d.protocol : ""}
      </div>
      {typeof d.boundPlatform === "string" && d.boundPlatform && (
        <div className="mt-1 text-[10px] text-blue-500/60 dark:text-blue-400/60 font-mono">
          {"->"} {d.boundPlatform}
          {typeof d.account === "string" && d.account ? " * " + d.account : ""}
        </div>
      )}
      {!enabled && (
        <div className="mt-1 text-[10px] text-zinc-500 dark:text-zinc-400 font-medium">
          {tFn("topology.portDisabled")}
        </div>
      )}
    </div>
  );
}
);

/// Custom node: Platform (B column) with dual A+B strategy badges.
const PlatformNode = memo(function PlatformNode({ data }: NodeProps) {
  const d = data as Record<string, unknown>;
  const leases = (Array.isArray(d.leases) ? d.leases : []) as Array<{
    account: string; egress_ip: string; target_domain: string;
  }>;
  const portChips = Array.isArray(d.ports) ? d.ports as number[] : [];
  const bClassLabel = typeof d.bClassLabel === "string" ? d.bClassLabel : "";
  const aClassLabel = typeof d.aClassLabel === "string" ? d.aClassLabel : "";
  return (
    <div className="relative rounded-lg border border-zinc-400 dark:border-zinc-600 bg-white dark:bg-zinc-900 px-4 py-3 text-xs min-w-[160px] max-w-[240px]">
      <Handle type="target" position={Position.Left} style={fixedHandleStyle} />
      <Handle type="source" position={Position.Right} style={fixedHandleStyle} />
      <div className="font-semibold text-zinc-800 dark:text-zinc-100">{String(d.label)}</div>
      {typeof d.sub === "string" && d.sub && <div className="text-zinc-500 dark:text-zinc-400 mt-1 text-[10px] whitespace-pre-line">{d.sub}</div>}
      {aClassLabel && (
        <div className="mt-1.5 flex items-center gap-1">
          <span className="rounded bg-emerald-100 dark:bg-emerald-900/40 px-1.5 py-0.5 text-[9px] text-emerald-600 dark:text-emerald-400 font-medium">
            A: {aClassLabel}
          </span>
        </div>
      )}
      {bClassLabel && (
        <div className="mt-0.5 flex items-center gap-1">
          <span className="rounded bg-blue-100 dark:bg-blue-900/40 px-1.5 py-0.5 text-[9px] text-blue-600 dark:text-blue-400 font-medium">
            B: {bClassLabel}
          </span>
        </div>
      )}
      {portChips.length > 0 && (
        <div className="mt-1 flex flex-wrap gap-1">
          {portChips.map((p) => (
            <span key={p} className="rounded bg-zinc-100 dark:bg-zinc-800 px-1 py-0.5 text-[9px] font-mono text-zinc-600 dark:text-zinc-400">{p}</span>
          ))}
        </div>
      )}
      {leases.length > 0 && (
        <div className="mt-1.5 flex flex-col gap-0.5">
          {leases.slice(0, 3).map((l, i) => (
            <div key={"lease-" + i} className="text-[9px] text-zinc-500 dark:text-zinc-400 truncate">
              <span className="font-mono">{(l.account || "").slice(0, 14)}</span> {"->"} <span className="font-mono" title={l.target_domain}>{(l.egress_ip || "").slice(0, 22) || "---"}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
);

/// Custom node: Subscription group (C column) — only shows selected nodes.
const SubscriptionGroupNode = memo(function SubscriptionGroupNode({ data }: NodeProps) {
  const d = data as Record<string, unknown>;
  const nodes = (Array.isArray(d.nodes) ? d.nodes : []) as Array<{
    display_tag: string; region: string; healthy: boolean; latencyColor: string;
  }>;
  const unboundCount = typeof d.unboundCount === "number" ? d.unboundCount : 0;
  const regionStats = (Array.isArray(d.regionStats) ? d.regionStats : []) as Array<{
    region: string; count: number;
  }>;
  const [expanded, setExpanded] = useState(false);
  const MAX_VISIBLE = 10;
  const visibleNodes = expanded ? nodes.slice(0, MAX_VISIBLE) : [];
  const moreCount = nodes.length - MAX_VISIBLE;
  return (
    <div className="relative rounded-lg border border-emerald-400 dark:border-emerald-600 bg-emerald-50 dark:bg-emerald-950/50 px-4 py-3 text-xs min-w-[180px] max-w-[280px]">
      <Handle type="target" position={Position.Left} style={fixedHandleStyle} />
      <div className="flex items-center justify-between">
        <div className="font-semibold text-emerald-700 dark:text-emerald-300">{String(d.label)}</div>
        {nodes.length > 0 && (
          <button
            onClick={(e) => { e.stopPropagation(); setExpanded(!expanded); }}
            className="text-[9px] text-emerald-500 dark:text-emerald-400 hover:text-emerald-700 dark:hover:text-emerald-200 cursor-pointer"
          >
            {expanded ? "▼" : "▶"}
          </button>
        )}
      </div>
      {typeof d.sub === "string" && d.sub && <div className="text-emerald-600/70 dark:text-emerald-400/70 mt-1 text-[10px]">{d.sub}</div>}
      {/* region stats summary (collapsed view) */}
      {regionStats.length > 0 && !expanded && (
        <div className="mt-1.5 flex flex-wrap gap-1">
          {regionStats.map((rs, i) => (
            <span key={"rs-" + i} className="rounded bg-emerald-100 dark:bg-emerald-900/40 px-1 py-0.5 text-[9px] text-emerald-700 dark:text-emerald-300 font-mono">
              {rs.region}({rs.count})
            </span>
          ))}
        </div>
      )}
      {/* expanded node list (max 10) */}
      {expanded && visibleNodes.length > 0 && (
        <div className="mt-2 flex flex-col gap-0.5">
          {visibleNodes.map((n, i) => (
            <div key={"node-" + i} className="flex items-center gap-1.5 text-[10px]">
              <span className={"inline-block h-1.5 w-1.5 rounded-full " + n.latencyColor} />
              <span className="text-zinc-600 dark:text-zinc-300 font-mono truncate">{n.display_tag}</span>
              <span className="text-zinc-400 dark:text-zinc-500 text-[9px]">{n.region}</span>
            </div>
          ))}
          {moreCount > 0 && (
            <div className="mt-0.5 text-[9px] text-zinc-400 dark:text-zinc-500">+{moreCount} more</div>
          )}
        </div>
      )}
      {unboundCount > 0 && (
        <div className="mt-1 text-[10px] text-zinc-400 dark:text-zinc-500">
          +{unboundCount} unbound
        </div>
      )}
    </div>
  );
}
);


/// Custom node: Region group (C column region view) — aggregates by region
/// Region group (C column region view) — fold contract mirroring SubscriptionGroupNode.
/// Collapsed by default: summary card with region label + healthy/total badge + sub chip.
/// Expandable on click: per-node rows (max 10 visible + "+N more").
const RegionGroupNode = memo(function RegionGroupNode({ data }: NodeProps) {
  const { t } = useTranslation();
  const d = data as Record<string, unknown>;
  const total = typeof d.total === "number" ? d.total : 0;
  const healthy = typeof d.healthy === "number" ? d.healthy : 0;
  const subs = (Array.isArray(d.subs) ? d.subs : []) as string[];
  const nodes = (Array.isArray(d.nodes) ? d.nodes : []) as Array<{
    display_tag: string; region: string; healthy: boolean; latencyColor: string;
  }>;
  const [expanded, setExpanded] = useState(false);
  const MAX_VISIBLE = 10;
  const visibleNodes = expanded ? nodes.slice(0, MAX_VISIBLE) : [];
  const moreCount = nodes.length - MAX_VISIBLE;
  return (
    <div className="relative rounded-lg border border-amber-400 dark:border-amber-600 bg-amber-50 dark:bg-amber-950/50 px-4 py-3 text-xs min-w-[160px] max-w-[240px]">
      <Handle type="target" position={Position.Left} style={fixedHandleStyle} />
      <div className="flex items-center justify-between">
        <div className="font-semibold text-amber-700 dark:text-amber-300">{String(d.label)}</div>
        {nodes.length > 0 && (
          <button
            onClick={(e) => { e.stopPropagation(); setExpanded(!expanded); }}
            className="text-[9px] text-amber-500 dark:text-amber-400 hover:text-amber-700 dark:hover:text-amber-200 cursor-pointer"
          >
            {expanded ? t("topology.collapseNodes") : t("topology.expandNodes")}
          </button>
        )}
      </div>
      <div className="mt-1 text-[10px] text-amber-600/70 dark:text-amber-400/70">
        {healthy}/{total} {t("topology.healthy")}
      </div>
      {!expanded && subs.length > 0 && (
        <div className="mt-1 flex flex-wrap gap-0.5">
          {subs.slice(0, 3).map((s, i) => (
            <span key={"sub-" + i} className="text-[9px] text-zinc-500 dark:text-zinc-400 truncate">{s}</span>
          ))}
          {subs.length > 3 && <span className="text-[9px] text-zinc-400">+{subs.length - 3}</span>}
        </div>
      )}
      {expanded && visibleNodes.length > 0 && (
        <div className="mt-2 flex flex-col gap-0.5">
          {visibleNodes.map((n, i) => (
            <div key={"node-" + i} className="flex items-center gap-1.5 text-[10px]">
              <span className={"inline-block h-1.5 w-1.5 rounded-full " + n.latencyColor} />
              <span className="text-zinc-600 dark:text-zinc-300 font-mono truncate">{n.display_tag}</span>
            </div>
          ))}
          {moreCount > 0 && (
            <div className="mt-0.5 text-[9px] text-zinc-400 dark:text-zinc-500">+{moreCount} more</div>
          )}
        </div>
      )}
    </div>
  );
}
);

const nodeTypes = { entryPort: EntryPortNode, platform: PlatformNode, subscriptionGroup: SubscriptionGroupNode, regionGroup: RegionGroupNode };

async function backupBeforeEdit(): Promise<void> {
  try { await ipcBackupCreate(); } catch { /* best-effort */ }
}

/// Region filter helpers.
export function addRegionFilter(current: string[] | null, region: string): string[] {
  if (current && current.includes(region)) return current;
  return [...(current ?? []), region];
}
export function removeRegionFilter(current: string[] | null, region: string): string[] {
  return (current ?? []).filter((r) => r !== region);
}

/// (ADR-0076, recovered ADR-0050): compute a platform's next manual_nodes
/// after deleting a manual group edge - forward-projection: deleting one
/// group-level edge removes the node_hashes belonging to that group from the
/// platform's manual_nodes. Returns null when the edge is not a manual group
/// edge (caller falls back to the region/subscription paths).
export function manualNodesAfterGroupEdgeDelete(
  edge: { source: string; target: string; label?: unknown },
  platforms: PlatformFull[],
  subGroups: SubscriptionGroup[],
): string[] | null {
  if (typeof edge.label !== "string" || !edge.label.startsWith("manual")) return null;
  if (!edge.source.startsWith("platform-")) return null;
  const platName = edge.source.slice("platform-".length);
  const plat = platforms.find((p) => p.name === platName);
  if (!plat) return null;
  const manualNodes = plat.manualNodes ?? [];
  let removeHashes: string[] = [];
  if (edge.target.startsWith("subgroup-")) {
    const subName = edge.target.slice("subgroup-".length);
    const sg = subGroups.find((g) => g.subscriptionName === subName);
    if (!sg) return null;
    removeHashes = sg.nodes
      .map((n) => (typeof n.node_hash === "string" ? n.node_hash : ""))
      .filter((h) => h);
  } else if (edge.target.startsWith("regiongroup-")) {
    const region = edge.target.slice("regiongroup-".length);
    removeHashes = subGroups
      .flatMap((g) => g.nodes)
      .filter((n) => getNodeRegion(n) === region)
      .map((n) => (typeof n.node_hash === "string" ? n.node_hash : ""))
      .filter((h) => h);
  } else {
    return null;
  }
  return manualNodes.filter((h) => !removeHashes.includes(h));
}

export async function patchAndSyncOnce(args: {
  platName: string; current: string[] | null; region: string; mode: "add" | "remove";
  sync: () => Promise<void>; backup?: () => Promise<void>;
  ipcUpdate?: (platName: string, policy: StrategyId | AllocationPolicy | undefined, regex: string[] | undefined, regions: string[]) => Promise<void>;
}): Promise<{ patched: boolean; next: string[] }> {
  const { platName, current, region, mode, sync, backup, ipcUpdate } = args;
  let next: string[];
  if (mode === "add") next = addRegionFilter(current, region);
  else next = removeRegionFilter(current, region);
  // idempotent skip for add: region already bound means no PATCH needed
  const curArr = current ?? [];
  if (mode === "add" && curArr.includes(region)) return { patched: false, next: curArr };
  if (backup) { try { await backup(); } catch { /* swallow */ } }
  const updater = ipcUpdate ?? ((n: string, _p: StrategyId | AllocationPolicy | undefined, _r: string[] | undefined, regs: string[]) => ipcPlatformUpdate(n, _p, _r, regs));
  await updater(platName, undefined, undefined, next);
  await sync();
  return { patched: true, next };
}

type EdgeWithLabel = { id: string; source: string; target: string; animated?: boolean; label?: string; deletable?: boolean };
export function buildEdges(
  platforms: { name: string; region_filters: string[] | null; allocation_policy?: string; aClass?: string; subscriptionNames?: string[]; manualNodes?: string[]; topN?: number }[],
  nodeGroups: { region: string }[] | { subscriptionName: string; regions: string[]; nodeHashes?: string[] }[],
  ports: { port: number; platform_name: string }[] = [],
): EdgeWithLabel[] {
  const list: EdgeWithLabel[] = [];
  for (const p of ports) {
    const plat = platforms.find((x) => x.name === p.platform_name);
    if (plat) {
      list.push({ id: "e-port-" + p.port + "-" + p.platform_name, source: "entry-port-" + p.port, target: "platform-" + p.platform_name, animated: true });
    }
  }
  if (ports.length === 0) {
    for (const p of platforms) {
      list.push({ id: "e-entry-" + p.name, source: "entry-port", target: "platform-" + p.name, animated: true });
    }
  }
  const isNewShape = nodeGroups.length > 0 && "subscriptionName" in (nodeGroups[0] as Record<string, unknown>);
  for (const p of platforms) {
    const regions = p.region_filters ?? [];
    for (const g of nodeGroups as any[]) {
      if (isNewShape) {
        const groupRegions: string[] = g.regions ?? [];
        // (ADR-0048 S3): a_class-semantic B->C edge
        const aClass = (p as any).aClass ?? "manual";
        let edgeMatched = false;
        let edgeLabel = "";
        switch (aClass) {
          case "subscription": {
            const subs = (p as any).subscriptionNames ?? [];
            if (subs.length === 0) {
              // (ADR-0075, recovered ADR-0052): unconditional edge carries NO
              // label (Kiali/Grafana/LangGraph default-flow-unlabeled convention).
              edgeMatched = true;
            } else if (subs.includes(g.subscriptionName)) {
              edgeMatched = true;
              edgeLabel = "subscription:" + g.subscriptionName;
            }
            break;
          }
          case "quality": {
            // Unconditional selection carries no label (same convention as
            // empty-subscription: default flows stay unlabeled).
            edgeMatched = true;
            break;
          }
          case "region": {
            const matched = regions.filter((r: string) => groupRegions.includes(r));
            if (matched.length > 0) {
              edgeMatched = true;
              edgeLabel = "region:" + matched.join(",");
            }
            break;
          }
          case "manual":
          default: {
            // (ADR-0076, recovered ADR-0050): manual draws a visible edge to
            // groups containing selected node_hashes - an invisible intent
            // previously left manual platforms looking disconnected.
            const manualNodes = p.manualNodes ?? [];
            if (manualNodes.length === 0) break;
            const groupHashes: string[] = g.nodeHashes ?? [];
            if (groupHashes.some((h: string) => manualNodes.includes(h))) {
              edgeMatched = true;
              edgeLabel = "manual:" + manualNodes.length;
            }
            break;
          }
        }
        if (edgeMatched) {
          list.push({
            // (ADR-0075): viewMode-prefixed edge id forces ReactFlow
            // EdgeWrapper remount on view switch (xyflow #2973/#693 stale
            // geometry).
            id: "e-sub-" + p.name + "-" + g.subscriptionName,
            source: "platform-" + p.name,
            target: "subgroup-" + g.subscriptionName,
            label: edgeLabel,
            deletable: aClass === "manual",
          });
        }
      } else {
        if (regions.includes(g.region)) {
          list.push({
            id: "e-" + p.name + "-" + g.region,
            source: "platform-" + p.name,
            target: "nodegroup-" + g.region,
            label: "region:" + g.region,
            deletable: false,
          });
        }
      }
    }
  }
  return list;
}

/// (ADR-0077, recovered ADR-0051 defect C): region-viewMode edges as an
/// exported pure helper (extracted from the edges useMemo). Adds the
/// previously missing subscription-with-specific-subs branch: a platform with
/// subscriptions=["subA"] draws edges to the regions of subA's nodes
/// (mirroring buildEdges' subscription semantics) instead of drawing nothing.
/// (ADR-0075): e-reg- prefixed ids force EdgeWrapper remount; unconditional
/// edges carry no label.
export function buildRegionViewEdges(
  platforms: PlatformFull[],
  subGroups: SubscriptionGroup[],
  ports: { port: number; platform_name: string }[],
): Edge[] {
  const list: Edge[] = [];
  for (const p of platforms) {
    for (const port of ports) {
      if (port.platform_name === p.name) {
        list.push({ id: "e-port-" + port.port + "-" + p.name, source: "entry-port-" + port.port, target: "platform-" + p.name, animated: true });
      }
    }
    if (ports.length === 0) {
      list.push({ id: "e-entry-" + p.name, source: "entry-port", target: "platform-" + p.name, animated: true });
    }
    // (ADR-0048 S3): a_class-semantic region-viewMode edges
    const rAclass = p.aClass ?? "manual";
    if (rAclass === "quality") {
      // edge to all actual regionGroup nodes (not a phantom "regiongroup-all")
      const allRegions = new Set<string>();
      for (const g of subGroups) for (const r of g.regions) allRegions.add(r.toLowerCase());
      for (const r of allRegions) {
        list.push({ id: "e-reg-" + p.name + "-r-" + r, source: "platform-" + p.name, target: "regiongroup-" + r, deletable: false });
      }
    } else if (rAclass === "subscription") {
      const subs = p.subscriptionNames ?? [];
      if (subs.length === 0) {
        for (const g of subGroups) {
          for (const r of g.regions) {
            list.push({ id: "e-reg-" + p.name + "-r-" + r, source: "platform-" + p.name, target: "regiongroup-" + r.toLowerCase(), deletable: false });
          }
        }
      } else {
        // (ADR-0077 defect C): specific subscriptions -> edges to those
        // subscriptions' regions.
        const hit = new Set<string>();
        for (const sg of subGroups) {
          if (!subs.includes(sg.subscriptionName)) continue;
          for (const r of sg.regions) hit.add(r.toLowerCase());
        }
        for (const r of hit) {
          list.push({ id: "e-reg-" + p.name + "-r-" + r, source: "platform-" + p.name, target: "regiongroup-" + r, label: "subscription:" + subs.join(","), deletable: false });
        }
      }
    } else if (rAclass === "region") {
      for (const r of p.region_filters ?? []) {
        list.push({ id: "e-reg-" + p.name + "-r-" + r, source: "platform-" + p.name, target: "regiongroup-" + r.toLowerCase(), label: "region:" + r, deletable: false });
      }
    } else if (rAclass === "manual") {
      // (ADR-0076): manual draws visible edges to regions containing selected
      // node_hashes.
      const manualNodes = p.manualNodes ?? [];
      if (manualNodes.length > 0) {
        const hitRegions = new Set<string>();
        for (const g of subGroups) {
          for (const n of g.nodes) {
            const h = typeof n.node_hash === "string" ? n.node_hash : "";
            if (h && manualNodes.includes(h)) hitRegions.add(getNodeRegion(n));
          }
        }
        for (const r of hitRegions) {
          list.push({ id: "e-reg-" + p.name + "-r-" + r, source: "platform-" + p.name, target: "regiongroup-" + r, label: "manual:" + manualNodes.length, deletable: true });
        }
      }
    }
  }
  return list;
}

/// Custom CanvasControls with i18n tooltips.
function CanvasControls({ viewMode, setViewMode, locked, setLocked }: { viewMode: "subscription" | "region"; setViewMode: (m: "subscription" | "region") => void; locked: boolean; setLocked: (l: boolean) => void; }) {
  const { t } = useTranslation();
  const reactFlow = useReactFlow();
  return (
    <div className="absolute bottom-2 left-2 z-10 flex flex-col gap-1">
      {/* segmented toggle for C column view mode */}
      <div className="flex gap-0.5 rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 p-0.5">
        <button
          title={t("topology.viewSubscription")}
          onClick={() => setViewMode("subscription")}
          className={"rounded px-1.5 py-1 text-[9px] font-medium " + (viewMode === "subscription" ? "bg-emerald-100 dark:bg-emerald-900/40 text-emerald-700 dark:text-emerald-300" : "text-zinc-500 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-700")}
        >
          {t("topology.viewSubscription")}
        </button>
        <button
          title={t("topology.viewRegion")}
          onClick={() => setViewMode("region")}
          className={"rounded px-1.5 py-1 text-[9px] font-medium " + (viewMode === "region" ? "bg-amber-100 dark:bg-amber-900/40 text-amber-700 dark:text-amber-300" : "text-zinc-500 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-700")}
        >
          {t("topology.viewRegion")}
        </button>
      </div>
      <button title={t("topology.zoomIn")} onClick={() => reactFlow.zoomIn()} className="rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 p-1.5 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700 w-fit self-center">
        <ZoomIn size={14} />
      </button>
      <button title={t("topology.zoomOut")} onClick={() => reactFlow.zoomOut()} className="rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 p-1.5 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700 w-fit self-center">
        <ZoomOut size={14} />
      </button>
      <button title={t("topology.fitView")} onClick={() => reactFlow.fitView({ maxZoom: 1 })} className="rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 p-1.5 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700 w-fit self-center">
        <Maximize size={14} />
      </button>
      {/* reset to world center {x:0, y:0, zoom:1} */}
      <button title={t("topology.resetCenter")} onClick={() => reactFlow.setViewport({ x: 0, y: 0, zoom: 1 })} className="rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 p-1.5 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700 w-fit self-center">
        <Home size={14} />
      </button>
      <button title={locked ? t("topology.unlock") : t("topology.lock")} onClick={() => setLocked(!locked)} className="rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 p-1.5 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700 w-fit self-center">
        {locked ? <Lock size={14} /> : <Unlock size={14} />}
      </button>
    </div>
  );
}


function ConfigToolbar() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => { if (ref.current && !ref.current.contains(e.target as unknown as globalThis.Node)) setOpen(false); };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);
  const openCfg = async (file: string) => {
    setOpen(false);
    try {
      const dir = await ipcGetConfigDir();
      const { openPath } = await import("@tauri-apps/plugin-opener");
      await openPath(dir + "/" + file);
    } catch { /* swallow */ }
  };
  return (
    <div ref={ref} className="absolute top-2 right-2 z-10 flex flex-col items-end gap-1">
      <button
        title={t("topology.openConfig")}
        onClick={() => setOpen(!open)}
        className="rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 p-1.5 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700 flex items-center gap-1 text-[10px] font-medium w-fit self-center"
      >
        <FileCog size={14} />
        <ChevronDown size={10} />
      </button>
      {open && (
        <div className="absolute top-9 right-0 w-44 rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 shadow-lg z-20">
          <button
            onClick={() => openCfg("egressapikey-ports.json")}
            className="block w-full text-left px-3 py-1.5 text-[10px] text-zinc-700 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700"
          >
            {t("topology.openPortsConfig")}
          </button>
          <button
            onClick={() => openCfg("egressapikey-strategy.json")}
            className="block w-full text-left px-3 py-1.5 text-[10px] text-zinc-700 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700 border-t border-zinc-200 dark:border-zinc-700"
          >
            {t("topology.openStrategyConfig")}
          </button>
        </div>
      )}
    </div>
  );
}
function TopologyCanvas() {
  const { t, i18n } = useTranslation();
  const reactFlow = useReactFlow();
  const theme = useAppStore((s) => s.theme);

  // zustand store selectors with shallow
  const platforms = useTopologyStore((s) => s.platforms);
  const subGroups = useTopologyStore((s) => s.subGroups);
  const leases = useTopologyStore((s) => s.leases);
  const ports = useTopologyStore((s) => s.ports);
  const setPlatforms = useTopologyStore((s) => s.setPlatforms);
  const setSubGroups = useTopologyStore((s) => s.setSubGroups);
  const setLeases = useTopologyStore((s) => s.setLeases);
  const setPorts = useTopologyStore((s) => s.setPorts);
  const portHealth = useTopologyStore((s) => s.portHealth);
  const setPortHealth = useTopologyStore((s) => s.setPortHealth);

  const [sidecarStatus, setSidecarStatus] = useState<string>("");
  const [viewMode, setViewMode] = useState<"subscription" | "region">("subscription");
  const [locked, setLocked] = useState(false);
  const [ready, setReady] = useState(false);
  const patchingRef = useRef(false);

  const sync = useCallback(async () => {
    try {
      // (Authoritative Snapshot): the canvas consumes the ONE
      // pre-merged snapshot (whitebox strategy + whitebox ports + Resin
      // runtime merged in resin-core). View-layer cross-store merging is
      // deleted per ARCHITECTURE.md §Config Authority; divergence is now
      // reported by the snapshot itself, never silently reconciled here.
      const [snap, nRaw, lRaw] = await Promise.all([
        ipcAuthoritativeSnapshot(),
        ipcNodeList(),
        ipcLeaseMap(),
      ]);
      const plats = snapshotToPlatformFulls(snap);
      setPlatforms(plats);
      setSubGroups(parseSubscriptionGroups(nRaw));
      setLeases(lRaw as LeaseEntry[]);
      setPorts(snapshotToPortMappings(snap));
    } catch { /* vitest or sidecar not ready */ }
    setReady(true); // setReady after sync so data is present before show
  }, [setPlatforms, setSubGroups, setLeases, setPorts]);

  // usePoll replaces setInterval + visibilitychange boilerplate
  usePoll(sync, { intervalMs: 5000, fireImmediately: true, pauseWhenHidden: true });

  // Keep the sidecar-status event listener (not covered by usePoll)
  useEffect(() => {
    let unsub: (() => void) | null = null;
    void listen("sidecar-status", (evt) => {
      setSidecarStatus(String((evt as { payload: unknown }).payload ?? ""));
    }).then((fn) => { unsub = fn as (() => void); }).catch((e) => console.warn("[TopologyView] listen failed", e));
    return () => { if (unsub) { try { unsub(); } catch { /* ignore */ } } };
  }, []);

  // subscribe to port health batch snapshots; update per-port health map.
  useEffect(() => {
    const unsub = ipcWatchPortHealth((snap) => {
      const next: Record<number, PortHealthEntry> = {};
      for (const e of snap.entries) next[e.port] = e;
      setPortHealth(next);
    }, (err) => console.warn("[TopologyView] port health stream error", err));
    return () => { try { unsub(); } catch { /* ignore */ } };
  }, [setPortHealth]);

  // persist viewMode + locked when they change (but only after ready to avoid overriding onInit load)
  useEffect(() => {
    if (!ready) return;
    void (async () => {
      try {
        const ts = await loadTopologyState() as PersistedTopologyState | null;
        await saveTopologyState({ x: ts?.x ?? 0, y: ts?.y ?? 0, zoom: ts?.zoom ?? 1, viewMode, locked });
      } catch { /* vitest */ }
    })();
  }, [viewMode, locked, ready]);

  const onMoveEnd: OnMoveEnd = useCallback((_evt, viewport) => {
    if (!ready) return;
    if (!viewport || typeof viewport.x !== "number" || typeof viewport.y !== "number" || typeof viewport.zoom !== "number") return;
    try { void saveTopologyState({ x: viewport.x, y: viewport.y, zoom: viewport.zoom, viewMode, locked }); } catch { /* vitest */ }
  }, [ready, viewMode, locked]);

  const onInit = useCallback((_instance: unknown) => {
    void (async () => {
      try {
        const ts = await loadTopologyState();
        if (ts) {
          reactFlow.setViewport({ x: ts.x, y: ts.y, zoom: ts.zoom });
          setViewMode(ts.viewMode);
          setLocked(ts.locked);
        } else {
          requestAnimationFrame(() => { try { reactFlow.fitView({ maxZoom: 1 }); } catch { /* vitest */ } });
        }
      } catch { /* vitest, no reactflow */ }
    })();
  }, [reactFlow]);

  const colorMode: ColorMode = theme;


  // build nodes — positions assigned by dagre later
  const rawNodes: Node[] = useMemo(() => {
    if (!i18n.isInitialized || !i18n.language) return [];
    const list: Node[] = [];
    ports.forEach((p) => {
      const he = portHealth[p.port];
      list.push({
        id: "entry-port-" + p.port,
        type: "entryPort",
        position: { x: 0, y: 0 },
        data: {
          port: p.port, protocol: p.protocol, label: p.label,
          boundPlatform: p.platform_name, account: p.account,
          healthState: he?.state ?? "alive",
          authRequired: p.auth_required,
          enabled: p.enabled,
        },
      });
    });
    if (ports.length === 0) {
      list.push({
        id: "entry-port",
        type: "entryPort",
        position: { x: 0, y: 0 },
        data: { label: t("topology.entryPort"), port: 0, protocol: "", boundPlatform: "", account: "" },
      });
    }
    const leasesByPid = new Map<string, typeof leases>();
    for (const l of leases) {
      const pid = (l.platform_id || "").trim();
      const arr = leasesByPid.get(pid) ?? [];
      arr.push(l);
      leasesByPid.set(pid, arr);
    }
    platforms.forEach((p) => {
      const filters = p.regex_filters?.length ? t("topology.filters", { filters: p.regex_filters.join(", ") }) : "";
      const routable = t("topology.routable", { count: p.routable_node_count });
      const sub = [filters, routable].filter(Boolean).join("\n");
      // strategy badge reads strategyConfig first (ADR-0039 SS2).
      // B-class: prefer strategyConfig b_class; fall back to Resin allocation_policy mapping.
      const shellStrategy = p.bClass ?? mapResinToShell(p.allocation_policy ?? "BALANCED");
      // no parameter interpolation left — the withdrawn
      // display-only BClassParams are gone, so the badge states the policy.
      const bClassLabel = bClassLabelFn(shellStrategy, t);
      // A-class: switch on strategyConfig a_class (not just region_filters length).
      const aClassLabel = (() => {
        switch (p.aClass ?? "manual") {
          case "region":
            return (p.region_filters?.length ?? 0) > 0 ? t("topology.aClassRegion", { regions: p.region_filters!.join(",").toUpperCase() }) : t("topology.aClassManual");
          case "quality":
            return t("topology.aClassQuality", { n: p.topN ?? 10 });
          case "subscription":
            return t("topology.aClassSubscription", { subs: (p.subscriptionNames ?? []).join(",") });
          case "manual":
          default:
            // Show manual_nodes count when available.
            if (Array.isArray(p.manualNodes) && p.manualNodes.length > 0) {
              return t("topology.aClassManualCount", { n: p.manualNodes.length });
            }
            // Fallback: if region_filters exist but aClass is unset, show Region badge.
            if ((p.region_filters?.length ?? 0) > 0) {
              return t("topology.aClassRegion", { regions: p.region_filters!.join(",").toUpperCase() });
            }
            return t("topology.aClassManual");
        }
      })();
      const pidLeases = (leasesByPid.get(p.id) ?? []).slice(0, 3);
      list.push({
        id: "platform-" + p.name,
        type: "platform",
        position: { x: 0, y: 0 },
        data: {
          label: p.name, sub,
          leases: pidLeases,
          ports: ports.filter((x) => x.platform_name === p.name).map((x) => x.port),
          bClassLabel, aClassLabel,
        },
      });
    });
    // (ADR-0041 S1): C-column subscriptionGroup/regionGroup nodes are built by the
    // extracted `buildCColumnGroups` helper so a vitest can assert the viewMode guard directly
    // without driving jsdom-rendered ReactFlow custom-node DOM (which does not stamp C-column
    // nodes reliably). See docs/adr/0041-canvas-v4-node-pool-toolbars-merge.md §S1.
    const cNodes = buildCColumnGroups(viewMode, subGroups, platforms, t);
    list.push(...cNodes);
    return list;
  }, [platforms, subGroups, leases, ports, t, i18n.isInitialized, i18n.language, viewMode, portHealth]);

  // build edges first, then dagre layout both
  const edges: Edge[] = useMemo(() => {
    if (viewMode === "region") {
      // in region view, edges connect platforms to region groups
      return buildRegionViewEdges(platforms, subGroups, ports);
    }
    const adapted = subGroups.map((g) => ({
      subscriptionName: g.subscriptionName,
      regions: g.regions,
      nodeHashes: g.nodes
        .map((n) => (typeof n.node_hash === "string" ? n.node_hash : ""))
        .filter((h) => h),
    }));
    return buildEdges(platforms, adapted, ports) as Edge[];
  }, [platforms, subGroups, ports, viewMode]);

  // dagre auto-layout — compute positions
  const nodes: Node[] = useMemo(() => {
    return layoutNodesViaDagre(rawNodes, edges);
  }, [rawNodes, edges]);


  // (ADR-0052): update platform region_filters through the
  // StrategyService deep IPC (strategy_platform_regions_set). The view no
  // longer reads/edits/derives the strategyConfig JSON shape itself — one
  // call sets regions whitebox-side, then strategy_apply enforces it on Resin.
  const patchRegionViaStrategyConfig = useCallback(async (platName: string, nextRegions: string[]) => {
    await ipcStrategyPlatformRegionsSet(platName, nextRegions);
    await ipcStrategyApply();
  }, []);

  // (ADR-0075): subscription deep write - records subscription INTENT +
  // a_class=subscription server-side; the engine derives the region
  // projection via a_class_regions at apply time. The Gen-1 fault expanded
  // sg.regions into region_filters on write while the Gen-2 read path judged
  // by subscriptions - write/read model mismatch (recovered ADR-0052).
  const patchSubscriptionsViaStrategyConfig = useCallback(async (platName: string, nextSubs: string[]) => {
    await ipcStrategyPlatformSubscriptionsSet(platName, nextSubs);
    await ipcStrategyApply();
  }, []);

  // (ADR-0076): manual_nodes deep write for the manual group-edge delete
  // projection (mirrors patchRegionViaStrategyConfig).
  const patchManualNodesViaStrategyConfig = useCallback(async (platName: string, nextManual: string[]) => {
    await ipcStrategyPlatformManualNodesSet(platName, nextManual);
    await ipcStrategyApply();
  }, []);

  const onConnect = useCallback(async (conn: Connection) => {
    // entry-port → platform drag-bind
    if (conn.source.startsWith("entry-port-") && conn.target.startsWith("platform-")) {
      const portNum = parseInt(conn.source.slice("entry-port-".length), 10);
      const platName = conn.target.slice("platform-".length);
      if (!Number.isFinite(portNum) || portNum <= 0) return;
      if (patchingRef.current) return;
      patchingRef.current = true;
      try {
        await ipcPortBindPlatform(portNum, platName);
        await sync();
      } catch { /* swallow */ }
      patchingRef.current = false;
      return;
    }
    if (!conn.source.startsWith("platform-")) return;
    const isSub = conn.target.startsWith("subgroup-");
    const isRegion = conn.target.startsWith("regiongroup-");
    if (!isSub && !isRegion) return;
    if (patchingRef.current) return;
    patchingRef.current = true;
    const platName = conn.source.slice("platform-".length);
    const plat = platforms.find((p) => p.name === platName);
    if (!plat) { patchingRef.current = false; return; }
    if (isSub) {
      // (ADR-0075): dragging a subscription-group edge writes subscription
      // INTENT (not a region bundle) - a_class=subscription lands
      // server-side, and compute_plan derives regions from subscriptions.
      const subName = conn.target.slice("subgroup-".length);
      if (!subName) { patchingRef.current = false; return; }
      const cur = plat.subscriptionNames ?? [];
      if (cur.includes(subName)) { patchingRef.current = false; return; }
      const nextSubs = [...cur, subName];
      try {
        await backupBeforeEdit();
        await patchSubscriptionsViaStrategyConfig(platName, nextSubs);
        await sync();
      } catch { /* swallow */ }
      patchingRef.current = false;
      return;
    }
    // region view: target is regiongroup-<region>
    const region = conn.target.slice("regiongroup-".length);
    if (region.startsWith("-")) { patchingRef.current = false; return; }
    const next = addRegionFilter(plat.region_filters, region);
    try {
      await backupBeforeEdit();
      await patchRegionViaStrategyConfig(platName, next);
      await sync();
    } catch { /* swallow */ }
    patchingRef.current = false;
  }, [platforms, subGroups, sync, patchRegionViaStrategyConfig, patchSubscriptionsViaStrategyConfig]);

  const onEdgesDelete = useCallback(async (delEdges: Edge[]) => {
    for (const e of delEdges) {
      if (e.deletable === false) continue;
      if (!e.source.startsWith("platform-")) continue;
      const isSub = e.target.startsWith("subgroup-");
      const isRegion = e.target.startsWith("regiongroup-");
      if (!isSub && !isRegion) continue;
      if (patchingRef.current) continue;
      patchingRef.current = true;
      const platName = e.source.slice("platform-".length);
      const plat = platforms.find((p) => p.name === platName);
      if (!plat) { patchingRef.current = false; continue; }
      // (ADR-0076): a manual group edge deletes from manual_nodes, not
      // region_filters.
      const nextManual = manualNodesAfterGroupEdgeDelete(e, platforms, subGroups);
      if (nextManual !== null) {
        try {
          await backupBeforeEdit();
          await patchManualNodesViaStrategyConfig(platName, nextManual);
          await sync();
        } catch { /* swallow */ }
        patchingRef.current = false;
        continue;
      }
      if (isSub) {
        // (ADR-0075): mirror of the connect path - deleting a subscription
        // edge removes the subscription INTENT, not a region projection.
        const subName = e.target.slice("subgroup-".length);
        const nextSubs = (plat.subscriptionNames ?? []).filter((sn) => sn !== subName);
        try {
          await backupBeforeEdit();
          await patchSubscriptionsViaStrategyConfig(platName, nextSubs);
          await sync();
        } catch { /* swallow */ }
        patchingRef.current = false;
        continue;
      }
      // region view: remove the single region
      const region = e.target.slice("regiongroup-".length);
      const next = removeRegionFilter(plat.region_filters, region);
      try {
        await backupBeforeEdit();
        await patchRegionViaStrategyConfig(platName, next);
        await sync();
      } catch { /* swallow */ }
      patchingRef.current = false;
    }
  }, [platforms, subGroups, sync, patchRegionViaStrategyConfig, patchSubscriptionsViaStrategyConfig, patchManualNodesViaStrategyConfig]);

  return (
    <section className="flex h-full flex-col p-3">
      <HeadlessCapabilityNotice commands={["backup_create", "strategy_apply", "strategy_platform_regions_set", "get_config_dir", "authoritative_snapshot"]} />
      {sidecarStatus === "unhealthy" && (
        <div className="mb-2 flex items-center gap-2 rounded-md border border-red-400 dark:border-red-700 bg-red-100 dark:bg-red-950/60 px-3 py-2 text-xs text-red-800 dark:text-red-200">
          <AlertTriangle size={14} className="shrink-0" />
          <span>{t("topology.sidecarUnhealthy")}</span>
        </div>
      )}
      {sidecarStatus === "restarting" && (
        <div className="mb-2 flex items-center gap-2 rounded-md border border-yellow-300 dark:border-yellow-800 bg-yellow-50 dark:bg-yellow-950/40 px-3 py-2 text-xs text-yellow-700 dark:text-yellow-300">
          <Loader2 size={14} className="shrink-0 animate-spin" />
          <span>{t("topology.sidecarRestarting")}</span>
        </div>
      )}
      {sidecarStatus === "terminated" && (
        <div className="mb-2 flex items-center gap-2 rounded-md border border-red-400 dark:border-red-700 bg-red-100 dark:bg-red-950/60 px-3 py-2 text-xs text-red-800 dark:text-red-200">
          <AlertTriangle size={14} className="shrink-0" />
          <span>{t("topology.sidecarTerminated")}</span>
        </div>
      )}
      <div className="mb-2 px-1">
        <span className="text-xs text-zinc-500 dark:text-zinc-400">{t("topology.dragHint")}</span>
      </div>
      {/* empty-state messages inside opacity gate so they don't flash before sync */}
      <div className={"flex-1 min-h-[400px] transition-opacity duration-200 " + (ready ? "opacity-100" : "opacity-0")}>
        {subGroups.length === 0 && (
          <div className="px-1 pb-2 text-xs text-zinc-400">{t("topology.noNodes")}</div>
        )}
        {platforms.length === 0 && (
          <div className="px-1 pb-2 text-xs text-zinc-400">{t("topology.noPlatforms")}</div>
        )}
        {ports.length === 0 && (
          <div className="px-1 pb-2 text-xs text-zinc-400">{t("topology.noPorts")}</div>
        )}
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          onConnect={onConnect}
          onEdgesDelete={onEdgesDelete}
          onMoveEnd={onMoveEnd}
          onInit={onInit}
          colorMode={colorMode}
          nodesConnectable={!locked}
          nodesDraggable={!locked}
          connectionRadius={40}
          defaultEdgeOptions={{ type: "smoothstep", animated: true, style: { fontSize: 10 } }}
        >
          <Background variant={BackgroundVariant.Dots} gap={18} size={1.4} />
          <CanvasControls viewMode={viewMode} setViewMode={setViewMode} locked={locked} setLocked={setLocked} />
          <ConfigToolbar />
          <MiniMap
            pannable
            zoomable
            ariaLabel={t("topology.minimapHint")}
            nodeColor={(n: Node) => {
              switch (n.type) {
                case "entryPort": return "#3b82f6";
                case "platform": return "#a855f7";
                case "subscriptionGroup": return "#22c55e";
                case "regionGroup": return "#f59e0b";
                default: return "#94a3b8";
              }
            }}
            maskColor="rgba(15, 23, 42, 0.7)"
          />
        </ReactFlow>
      </div>
    </section>
  );
}

export function TopologyView() {
  return (
    <ReactFlowProvider>
      <TopologyCanvas />
    </ReactFlowProvider>
  );
}
