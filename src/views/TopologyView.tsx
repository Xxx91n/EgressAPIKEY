import { useTranslation } from "react-i18next";
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
  ipcPlatformListFull, ipcNodeList, ipcPlatformUpdate, ipcBackupCreate,
  ipcLeaseMap, ipcPortList, type LeaseEntry, type PortMapping,
  ipcStrategyConfigGet, ipcStrategyConfigPut, ipcStrategyApply,
  ipcPortBindPlatform, ipcGetConfigDir,
} from "../lib/ipc";
import { loadTopologyState, saveTopologyState, type TopologyState as T15TopologyState } from "../lib/settings";
import { strategyToI18nKey, mapResinToShell, type StrategyId, type AllocationPolicy } from "../lib/strategy";
import { listen } from "@tauri-apps/api/event";
import type { ColorMode } from "@xyflow/react";
import { AlertTriangle, Loader2, ZoomIn, ZoomOut, Maximize, Lock, Unlock, Home } from "lucide-react";
import { usePoll } from "../hooks/usePoll";

/// TopologyView T13 — Canvas V2: strategy-driven dagre layout + flash fix + zustand cache.
///   A (Entry proxy port) --> B (Platforms) --strategy match--> C (IP channels / nodes)
///
/// T13-1: C column only shows nodes targeted by at least one platform region_filters.
/// T13-2: dagre auto-layout replaces hardcoded positions.
/// T13-3: Handle fixed at card edge midpoint (not full-area overlay).
/// T13-4: Empty-state messages inside opacity gate; sync() resolves then setReady.
/// T13-5: topologyStore (zustand + shallow) caches sync data; 5s poll restored.

interface PlatformFull {
  id: string;
  name: string;
  regex_filters: string[] | null;
  region_filters: string[] | null;
  allocation_policy: string;
  routable_node_count: number;
  sticky_ttl: string;
  // T15-v3-3: strategyConfig read-side fields (ADR-0036 read-side, ADR-0039 SS2)
  aClass?: string;              // strategyConfig a_class: manual | region | quality | subscription
  bClass?: string;             // strategyConfig b_class (shell StrategyId snake_case)
  subscriptionNames?: string[]; // strategyConfig subscriptions
  topN?: number;               // strategyConfig top_n
}

interface NodeItem {
  name?: string;
  display_tag?: string;
  node_hash?: string;
  has_outbound?: boolean;
  failure_count?: number;
  region?: string;
  tags?: Array<{ subscriptionName?: string; tag?: string }>;
}

interface SubscriptionGroup {
  subscriptionName: string;
  nodes: NodeItem[];
  healthy: number;
  total: number;
  regions: string[];
}

// --- T13-5: topologyStore (zustand + shallow) ---
interface TopologyState {
  platforms: PlatformFull[];
  subGroups: SubscriptionGroup[];
  leases: LeaseEntry[];
  ports: PortMapping[];
  setPlatforms: (p: PlatformFull[]) => void;
  setSubGroups: (s: SubscriptionGroup[]) => void;
  setLeases: (l: LeaseEntry[]) => void;
  setPorts: (p: PortMapping[]) => void;
}

export const useTopologyStore = create<TopologyState>((set) => ({
  platforms: [],
  subGroups: [],
  leases: [],
  ports: [],
  setPlatforms: (p) => set((s) => (shallow(s.platforms, p) ? {} : { platforms: p })),
  setSubGroups: (sg) => set((s) => (shallow(s.subGroups, sg) ? {} : { subGroups: sg })),
  setLeases: (l) => set((s) => (shallow(s.leases, l) ? {} : { leases: l })),
  setPorts: (p) => set((s) => (shallow(s.ports, p) ? {} : { ports: p })),
}));

// --- T13-1: helper — get selected regions from all platforms ---
export function getSelectedRegions(platforms: PlatformFull[]): Set<string> {
  const set = new Set<string>();
  for (const p of platforms) {
    for (const r of p.region_filters ?? []) set.add(r.toLowerCase());
  }
  return set;
}

// --- T13-1: parse nodes grouped by subscription source ---
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
      const subTag = n.tags.find((t) => t.subscriptionName);
      if (subTag) subName = subTag.subscriptionName!;
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

// T14-audit: extract region from a node — deduplicated from 6 inline copies
function getNodeRegion(n: NodeItem): string {
  if (n.region) return n.region.toLowerCase();
  if (Array.isArray(n.tags)) {
    const rt = n.tags.find((t) => t.tag && t.tag.length <= 3);
    if (rt) return rt.tag!.toLowerCase();
  }
  return "other";
}

function parsePlatforms(raw: unknown): PlatformFull[] {
  if (!raw || typeof raw !== "object") return [];
  const v = raw as Record<string, unknown>;
  const items = Array.isArray(v.items) ? v.items : Array.isArray(v) ? v : [];
  return items.filter((x): x is Record<string, unknown> => !!x && typeof x === "object")
    .map((p) => ({
      id: String(p.id ?? ""),
      name: String(p.name ?? ""),
      regex_filters: Array.isArray(p.regex_filters) ? p.regex_filters as string[] : null,
      region_filters: Array.isArray(p.region_filters) ? p.region_filters as string[] : null,
      allocation_policy: String(p.allocation_policy ?? "BALANCED"),
      routable_node_count: Number(p.routable_node_count ?? 0),
      sticky_ttl: String(p.sticky_ttl ?? "0s"),
    }));
}

// --- T13-2: dagre auto-layout helper ---
export function layoutNodesViaDagre(nodes: Node[], edges: Edge[], nodeWidth = 200, nodeHeight = 100): Node[] {
  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir: "LR", ranksep: 80, nodesep: 40, marginx: 20, marginy: 20 });
  g.setDefaultEdgeLabel(() => ({}));
  for (const n of nodes) {
    g.setNode(n.id, { width: nodeWidth, height: nodeHeight });
  }
  for (const e of edges) {
    g.setEdge(e.source, e.target);
  }
  dagre.layout(g);
  // T15-v3-2: authoritative dagre → ReactFlow position formula (pos.x - nodeWidth/2 only).
  // The graphLabel.width/2 subtraction was a known offset-drift bug (ADR-0039 SS4).
  return nodes.map((n) => {
    const pos = g.node(n.id);
    if (pos) {
      return { ...n, position: { x: pos.x - nodeWidth / 2, y: pos.y - nodeHeight / 2 } };
    }
    return n;
  });
}

// --- T13-3: fixed-edge Handle style (replaces full-area overlay) ---
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
  return (
    <div className="relative rounded-lg border border-blue-400 dark:border-blue-600 bg-blue-50 dark:bg-blue-950/50 px-4 py-3 text-xs min-w-[140px] max-w-[200px]">
      <Handle type="source" position={Position.Right} style={fixedHandleStyle} />
      <div className="font-semibold text-blue-700 dark:text-blue-300">
        {typeof d.port === "number" && d.port > 0 ? String(d.port) : String(d.label)}
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

/// T13-1: Custom node: Subscription group (C column) — only shows selected nodes.
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
      {/* T14-3: region stats summary (collapsed view) */}
      {regionStats.length > 0 && !expanded && (
        <div className="mt-1.5 flex flex-wrap gap-1">
          {regionStats.map((rs, i) => (
            <span key={"rs-" + i} className="rounded bg-emerald-100 dark:bg-emerald-900/40 px-1 py-0.5 text-[9px] text-emerald-700 dark:text-emerald-300 font-mono">
              {rs.region}({rs.count})
            </span>
          ))}
        </div>
      )}
      {/* T14-3: expanded node list (max 10) */}
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


/// T14-4: Custom node: Region group (C column region view) — aggregates by region
/// T15-v3-1: Region group (C column region view) — fold contract mirroring SubscriptionGroupNode.
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
  platforms: { name: string; region_filters: string[] | null; allocation_policy?: string }[],
  nodeGroups: { region: string }[] | { subscriptionName: string; regions: string[] }[],
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
        const matched = regions.filter((r: string) => groupRegions.includes(r));
        if (matched.length > 0) {
          list.push({
            id: "e-" + p.name + "-" + g.subscriptionName,
            source: "platform-" + p.name,
            target: "subgroup-" + g.subscriptionName,
            label: "region:" + matched.join(","),
            deletable: false,
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

/// T13-5: Custom CanvasControls with i18n tooltips.
function CanvasControls({ viewMode, setViewMode, locked, setLocked }: { viewMode: "subscription" | "region"; setViewMode: (m: "subscription" | "region") => void; locked: boolean; setLocked: (l: boolean) => void; }) {
  const { t } = useTranslation();
  const reactFlow = useReactFlow();
  return (
    <div className="absolute bottom-2 left-2 z-10 flex flex-col gap-1">
      {/* T14-5: segmented toggle for C column view mode */}
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
      {/* T14-5: reset to world center {x:0, y:0, zoom:1} */}
      <button title={t("topology.resetCenter")} onClick={() => reactFlow.setViewport({ x: 0, y: 0, zoom: 1 })} className="rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 p-1.5 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700 w-fit self-center">
        <Home size={14} />
      </button>
      <button title={locked ? t("topology.unlock") : t("topology.lock")} onClick={() => setLocked(!locked)} className="rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 p-1.5 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700 w-fit self-center">
        {locked ? <Lock size={14} /> : <Unlock size={14} />}
      </button>
    </div>
  );
}

function TopologyCanvas() {
  const { t, i18n } = useTranslation();
  const reactFlow = useReactFlow();
  const theme = useAppStore((s) => s.theme);

  // T13-5: zustand store selectors with shallow
  const platforms = useTopologyStore((s) => s.platforms);
  const subGroups = useTopologyStore((s) => s.subGroups);
  const leases = useTopologyStore((s) => s.leases);
  const ports = useTopologyStore((s) => s.ports);
  const setPlatforms = useTopologyStore((s) => s.setPlatforms);
  const setSubGroups = useTopologyStore((s) => s.setSubGroups);
  const setLeases = useTopologyStore((s) => s.setLeases);
  const setPorts = useTopologyStore((s) => s.setPorts);

  const [sidecarStatus, setSidecarStatus] = useState<string>("");
  const [viewMode, setViewMode] = useState<"subscription" | "region">("subscription");
  const [locked, setLocked] = useState(false);
  const [ready, setReady] = useState(false);
  const patchingRef = useRef(false);

  const sync = useCallback(async () => {
    try {
      const [pRaw, nRaw, lRaw, ptRaw, cfgRaw] = await Promise.all([
        ipcPlatformListFull(),
        ipcNodeList(),
        ipcLeaseMap(),
        ipcPortList(),
        ipcStrategyConfigGet().catch(() => null),
      ]);
      let plats = parsePlatforms(pRaw);
      // T15-sp6 (ADR-0036): strategyConfig JSON is the single source of truth for region_filters.
      // Merge strategyConfig regions over Resin platform.region_filters so the canvas reflects
      // whitebox edits even before strategy_apply PATCHes Resin.
      // T15-v3-3 (ADR-0039 SS2): merge ALL strategyConfig fields, not just regions.
      if (cfgRaw && Array.isArray(cfgRaw.platforms)) {
        const map = new Map<string, Record<string, unknown>>();
        for (const ps of cfgRaw.platforms) {
          if (ps.platform_name) map.set(ps.platform_name, ps as unknown as Record<string, unknown>);
        }
        plats = plats.map((p) => {
          const ps = map.get(p.name);
          if (!ps) return p;
          return {
            ...p,
            region_filters: Array.isArray(ps.regions) ? ps.regions as string[] : p.region_filters,
            aClass: typeof ps.a_class === "string" ? ps.a_class : undefined,
            bClass: typeof ps.b_class === "string" ? ps.b_class : undefined,
            subscriptionNames: Array.isArray(ps.subscriptions) ? ps.subscriptions as string[] : undefined,
            topN: typeof ps.top_n === "number" ? ps.top_n : undefined,
          };
        });
      }
      setPlatforms(plats);
      setSubGroups(parseSubscriptionGroups(nRaw));
      setLeases(lRaw as LeaseEntry[]);
      setPorts(ptRaw as PortMapping[]);
    } catch { /* vitest or sidecar not ready */ }
    setReady(true); // T13-4: setReady after sync so data is present before show
  }, [setPlatforms, setSubGroups, setLeases, setPorts]);

  // T14-3: usePoll replaces setInterval + visibilitychange boilerplate
  usePoll(sync, { intervalMs: 5000, fireImmediately: true, pauseWhenHidden: true });

  // Keep the sidecar-status event listener (not covered by usePoll)
  useEffect(() => {
    let unsub: (() => void) | null = null;
    void listen("sidecar-status", (evt) => {
      setSidecarStatus(String((evt as { payload: unknown }).payload ?? ""));
    }).then((fn) => { unsub = fn as (() => void); }).catch((e) => console.warn("[TopologyView] listen failed", e));
    return () => { if (unsub) { try { unsub(); } catch { /* ignore */ } } };
  }, []);

  // T15-3: persist viewMode + locked when they change (but only after ready to avoid overriding onInit load)
  useEffect(() => {
    if (!ready) return;
    void (async () => {
      try {
        const ts = await loadTopologyState() as T15TopologyState | null;
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

  // T13-1: get selected regions for filtering
  const selectedRegions = useMemo(() => getSelectedRegions(platforms), [platforms]);

  // T13-2: build nodes — positions assigned by dagre later
  const rawNodes: Node[] = useMemo(() => {
    if (!i18n.isInitialized || !i18n.language) return [];
    const list: Node[] = [];
    ports.forEach((p) => {
      list.push({
        id: "entry-port-" + p.port,
        type: "entryPort",
        position: { x: 0, y: 0 },
        data: { port: p.port, protocol: p.protocol, label: p.label, boundPlatform: p.platform_name, account: p.account },
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
      // T15-v3-3: strategy badge reads strategyConfig first (ADR-0039 SS2).
      // B-class: prefer strategyConfig b_class; fall back to Resin allocation_policy mapping.
      const shellStrategy = p.bClass ?? mapResinToShell(p.allocation_policy ?? "BALANCED");
      const bClassLabel = t(strategyToI18nKey(shellStrategy as ReturnType<typeof mapResinToShell>));
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
    // T13-1: C column — only show nodes whose region is in selectedRegions
    subGroups.forEach((g) => {
      const healthLabel = g.healthy === g.total
        ? t("topology.healthy")
        : g.healthy + "/" + g.total + " " + t("topology.healthy");
      const regionsLabel = g.regions.length > 0
        ? g.regions.slice(0, 5).join(", ").toUpperCase() + (g.regions.length > 5 ? "+" : "")
        : "";
      const sub = [healthLabel, regionsLabel].filter(Boolean).join(" * ");
      // T13-1: filter node rows — only show nodes in selected regions
      const filteredNodes = g.nodes.filter((n) => selectedRegions.has(getNodeRegion(n)));
      // T15-1: hide subscription group entirely if no nodes are selected by any platform
      if (filteredNodes.length === 0) return;
      const unboundCount = g.nodes.length - filteredNodes.length;
      // T14-3: compute region stats for collapsed view
      const regionStatsMap = new Map<string, number>();
      for (const n of g.nodes) {
        const r = getNodeRegion(n);
        regionStatsMap.set(r, (regionStatsMap.get(r) ?? 0) + 1);
      }
      const regionStatsArr = [...regionStatsMap.entries()].sort((a, b) => b[1] - a[1]).map(([region, count]) => ({ region: region.toUpperCase(), count }));
      const nodeRows = filteredNodes.map((n) => {
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
    // T14-4: region view mode — build region group nodes
    if (viewMode === "region") {
      const regionMap = new Map<string, { total: number; healthy: number; subs: Set<string>; nodeRows: Array<{ display_tag: string; region: string; healthy: boolean; latencyColor: string }> }>();
      for (const g of subGroups) {
        for (const n of g.nodes) {
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
        // T16-1: only show region groups that at least one platform selects
        if (!selectedRegions.has(region)) continue;
        const filteredNodeRows = info.nodeRows;
        list.push({
          id: "regiongroup-" + region,
          type: "regionGroup",
          position: { x: 0, y: 0 },
          data: { label: region.toUpperCase(), total: info.total, healthy: info.healthy, subs: [...info.subs], nodes: filteredNodeRows },
        });
      }
    }
    return list;
  }, [platforms, subGroups, leases, ports, t, i18n.isInitialized, i18n.language, selectedRegions, viewMode]);

  // T13-2: build edges first, then dagre layout both
  const edges: Edge[] = useMemo(() => {
    if (viewMode === "region") {
      // T14-4: in region view, edges connect platforms to region groups
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
        for (const r of p.region_filters ?? []) {
          list.push({ id: "e-" + p.name + "-r-" + r, source: "platform-" + p.name, target: "regiongroup-" + r.toLowerCase(), label: "region:" + r, deletable: false });
        }
      }
      return list;
    }
    const adapted = subGroups.map((g) => ({ subscriptionName: g.subscriptionName, regions: g.regions }));
    return buildEdges(platforms, adapted, ports) as Edge[];
  }, [platforms, subGroups, ports, viewMode]);

  // T13-2: dagre auto-layout — compute positions
  const nodes: Node[] = useMemo(() => {
    return layoutNodesViaDagre(rawNodes, edges);
  }, [rawNodes, edges]);

  
  // T15-5: update platform region_filters via strategyConfig JSON pipeline (not direct Resin PATCH)
  const patchRegionViaStrategyConfig = useCallback(async (platName: string, nextRegions: string[]) => {
    const cfg = await ipcStrategyConfigGet();
    let entry = cfg.platforms.find((p) => p.platform_name === platName);
    if (!entry) {
      entry = { platform_name: platName, a_class: "region", b_class: "random", regions: nextRegions };
      cfg.platforms.push(entry);
    } else {
      entry.regions = nextRegions;
    }
    await ipcStrategyConfigPut(cfg);
    await ipcStrategyApply();
  }, []);

  const onConnect = useCallback(async (conn: Connection) => {
    // T16-2: entry-port → platform drag-bind
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
    let next: string[];
    if (isSub) {
      const subName = conn.target.slice("subgroup-".length);
      const sg = subGroups.find((g) => g.subscriptionName === subName);
      if (!sg) { patchingRef.current = false; return; }
      const newRegions = sg.regions.filter((r) => !(plat.region_filters ?? []).includes(r));
      next = [...(plat.region_filters ?? []), ...newRegions];
    } else {
      // region view: target is regiongroup-<region>
      const region = conn.target.slice("regiongroup-".length);
      if (region.startsWith("-")) { patchingRef.current = false; return; }
      next = addRegionFilter(plat.region_filters, region);
    }
    try {
      await backupBeforeEdit();
      await patchRegionViaStrategyConfig(platName, next);
      await sync();
    } catch { /* swallow */ }
    patchingRef.current = false;
  }, [platforms, subGroups, sync, patchRegionViaStrategyConfig]);

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
      let next: string[];
      if (isSub) {
        const subName = e.target.slice("subgroup-".length);
        const sg = subGroups.find((g) => g.subscriptionName === subName);
        if (!sg) { patchingRef.current = false; continue; }
        next = (plat.region_filters ?? []).filter((r) => !sg.regions.includes(r));
      } else {
        // region view: remove the single region
        const region = e.target.slice("regiongroup-".length);
        next = removeRegionFilter(plat.region_filters, region);
      }
      try {
        await backupBeforeEdit();
        await patchRegionViaStrategyConfig(platName, next);
        await sync();
      } catch { /* swallow */ }
      patchingRef.current = false;
    }
  }, [platforms, subGroups, sync, patchRegionViaStrategyConfig]);

  return (
    <section className="flex h-full flex-col p-3">
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
      {/* T13-4: empty-state messages inside opacity gate so they don't flash before sync */}
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
        <button
          className="absolute bottom-20 left-4 z-10 rounded-lg border border-slate-300 bg-white px-3 py-1.5 text-xs font-medium text-slate-700 shadow-sm hover:bg-slate-50 dark:border-slate-600 dark:bg-slate-800 dark:text-slate-200 dark:hover:bg-slate-700"
          title={t("topology.openStrategyConfig")}
          onClick={async () => {
            try {
              const dir = await ipcGetConfigDir();
              const { openPath } = await import("@tauri-apps/plugin-opener");
              await openPath(dir + "/egressapikey-strategy.json");
            } catch { /* swallow */ }
          }}
        >
          {t("topology.openStrategyConfig")}
        </button>
          <div title={t("topology.minimapHint")}>
          <MiniMap
            pannable
            zoomable
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
        </div>
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
