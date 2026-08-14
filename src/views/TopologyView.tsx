import { useTranslation } from "react-i18next";
import {
  ReactFlow, Background, BackgroundVariant, MiniMap,
  Handle, Position, type Node, type Edge, type Connection, type NodeProps,
  useReactFlow, ReactFlowProvider, type OnMoveEnd,
} from "@xyflow/react";
import { useEffect, useMemo, useState, useCallback, useRef } from "react";
import { create } from "zustand";
import { shallow } from "zustand/shallow";
import dagre from "dagre";
import "@xyflow/react/dist/style.css";
import { useAppStore } from "../store/appStore";
import {
  ipcPlatformListFull, ipcNodeList, ipcPlatformUpdate, ipcBackupCreate,
  ipcLeaseMap, ipcPortList, type LeaseEntry, type PortMapping,
} from "../lib/ipc";
import { loadTopologyViewport, saveTopologyViewport } from "../lib/settings";
import { strategyToI18nKey, mapResinToShell, type StrategyId, type AllocationPolicy } from "../lib/strategy";
import { listen } from "@tauri-apps/api/event";
import type { ColorMode } from "@xyflow/react";
import { AlertTriangle, Loader2, ZoomIn, ZoomOut, Maximize, Lock, Unlock } from "lucide-react";

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
function EntryPortNode({ data }: NodeProps) {
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

/// Custom node: Platform (B column) with dual A+B strategy badges.
function PlatformNode({ data }: NodeProps) {
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
      {bClassLabel && (
        <div className="mt-1.5 flex items-center gap-1">
          <span className="rounded bg-blue-100 dark:bg-blue-900/40 px-1.5 py-0.5 text-[9px] text-blue-600 dark:text-blue-400 font-medium">
            B: {bClassLabel}
          </span>
        </div>
      )}
      {aClassLabel && (
        <div className="mt-0.5 flex items-center gap-1">
          <span className="rounded bg-emerald-100 dark:bg-emerald-900/40 px-1.5 py-0.5 text-[9px] text-emerald-600 dark:text-emerald-400 font-medium">
            A: {aClassLabel}
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

/// T13-1: Custom node: Subscription group (C column) — only shows selected nodes.
function SubscriptionGroupNode({ data }: NodeProps) {
  const d = data as Record<string, unknown>;
  const nodes = (Array.isArray(d.nodes) ? d.nodes : []) as Array<{
    display_tag: string; region: string; healthy: boolean; latencyColor: string;
  }>;
  const unboundCount = typeof d.unboundCount === "number" ? d.unboundCount : 0;
  return (
    <div className="relative rounded-lg border border-emerald-400 dark:border-emerald-600 bg-emerald-50 dark:bg-emerald-950/50 px-4 py-3 text-xs min-w-[180px] max-w-[280px]">
      <Handle type="target" position={Position.Left} style={fixedHandleStyle} />
      <div className="font-semibold text-emerald-700 dark:text-emerald-300">{String(d.label)}</div>
      {typeof d.sub === "string" && d.sub && <div className="text-emerald-600/70 dark:text-emerald-400/70 mt-1 text-[10px]">{d.sub}</div>}
      {nodes.length > 0 && (
        <div className="mt-2 flex flex-col gap-0.5">
          {nodes.map((n, i) => (
            <div key={"node-" + i} className="flex items-center gap-1.5 text-[10px]">
              <span className={"inline-block h-1.5 w-1.5 rounded-full " + n.latencyColor} />
              <span className="text-zinc-600 dark:text-zinc-300 font-mono truncate">{n.display_tag}</span>
              <span className="text-zinc-400 dark:text-zinc-500 text-[9px]">{n.region}</span>
            </div>
          ))}
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

const nodeTypes = { entryPort: EntryPortNode, platform: PlatformNode, subscriptionGroup: SubscriptionGroupNode };

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
function CanvasControls() {
  const { t } = useTranslation();
  const reactFlow = useReactFlow();
  const [locked, setLocked] = useState(false);
  return (
    <div className="absolute bottom-2 left-2 z-10 flex flex-col gap-1">
      <button title={t("topology.zoomIn")} onClick={() => reactFlow.zoomIn()} className="rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 p-1.5 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700">
        <ZoomIn size={14} />
      </button>
      <button title={t("topology.zoomOut")} onClick={() => reactFlow.zoomOut()} className="rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 p-1.5 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700">
        <ZoomOut size={14} />
      </button>
      <button title={t("topology.fitView")} onClick={() => reactFlow.fitView({ maxZoom: 1 })} className="rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 p-1.5 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700">
        <Maximize size={14} />
      </button>
      <button title={locked ? t("topology.unlock") : t("topology.lock")} onClick={() => setLocked(!locked)} className="rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 p-1.5 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700">
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
  const [ready, setReady] = useState(false);
  const patchingRef = useRef(false);

  const sync = useCallback(async () => {
    try {
      const [pRaw, nRaw, lRaw, ptRaw] = await Promise.all([
        ipcPlatformListFull(),
        ipcNodeList(),
        ipcLeaseMap(),
        ipcPortList(),
      ]);
      setPlatforms(parsePlatforms(pRaw));
      setSubGroups(parseSubscriptionGroups(nRaw));
      setLeases(lRaw as LeaseEntry[]);
      setPorts(ptRaw as PortMapping[]);
    } catch { /* vitest or sidecar not ready */ }
    setReady(true); // T13-4: setReady after sync so data is present before show
  }, [setPlatforms, setSubGroups, setLeases, setPorts]);

  // T13-5: mount + 5s poll + visibilitychange
  useEffect(() => {
    void sync();
    const interval = setInterval(() => void sync(), 5000);
    let unsub: (() => void) | null = null;
    void listen("sidecar-status", (evt) => {
      setSidecarStatus(String((evt as { payload: unknown }).payload ?? ""));
    }).then((fn) => { unsub = fn as (() => void); }).catch((e) => console.warn("[TopologyView] listen failed", e));
    const onVis = () => { if (document.visibilityState === "visible") void sync(); };
    document.addEventListener("visibilitychange", onVis);
    return () => {
      clearInterval(interval);
      if (unsub) { try { unsub(); } catch { /* ignore */ } }
      document.removeEventListener("visibilitychange", onVis);
    };
  }, [sync]);

  const onMoveEnd: OnMoveEnd = useCallback((_evt, viewport) => {
    if (!ready) return;
    if (!viewport || typeof viewport.x !== "number" || typeof viewport.y !== "number" || typeof viewport.zoom !== "number") return;
    try { void saveTopologyViewport({ x: viewport.x, y: viewport.y, zoom: viewport.zoom }); } catch { /* vitest */ }
  }, [ready]);

  const onInit = useCallback((_instance: unknown) => {
    void (async () => {
      try {
        const vp = await loadTopologyViewport();
        if (vp && typeof vp.x === "number" && typeof vp.y === "number" && typeof vp.zoom === "number") {
          reactFlow.setViewport({ x: vp.x, y: vp.y, zoom: vp.zoom });
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
      const shellStrategy = mapResinToShell(p.allocation_policy ?? "BALANCED");
      const bClassLabel = t(strategyToI18nKey(shellStrategy));
      const aClassLabel = (p.region_filters?.length ?? 0) > 0 ? "region:" + p.region_filters!.join(",").toUpperCase() : "manual";
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
      const filteredNodes = g.nodes.filter((n) => {
        let region = "other";
        if (n.region) region = n.region.toLowerCase();
        else if (Array.isArray(n.tags)) {
          const rt = n.tags.find((t2) => t2.tag && t2.tag.length <= 3);
          if (rt) region = rt.tag!.toLowerCase();
        }
        return selectedRegions.has(region);
      });
      const unboundCount = g.nodes.length - filteredNodes.length;
      const nodeRows = filteredNodes.map((n) => {
        const isHealthy = (n.failure_count ?? 0) === 0 && n.has_outbound !== false;
        let region = "other";
        if (n.region) region = n.region.toLowerCase();
        else if (Array.isArray(n.tags)) {
          const rt = n.tags.find((t2) => t2.tag && t2.tag.length <= 3);
          if (rt) region = rt.tag!.toLowerCase();
        }
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
        data: { label: g.subscriptionName, sub, nodes: nodeRows, unboundCount },
      });
    });
    return list;
  }, [platforms, subGroups, leases, ports, t, i18n.isInitialized, i18n.language, selectedRegions]);

  // T13-2: build edges first, then dagre layout both
  const edges: Edge[] = useMemo(() => {
    const adapted = subGroups.map((g) => ({ subscriptionName: g.subscriptionName, regions: g.regions }));
    return buildEdges(platforms, adapted, ports) as Edge[];
  }, [platforms, subGroups, ports]);

  // T13-2: dagre auto-layout — compute positions
  const nodes: Node[] = useMemo(() => {
    return layoutNodesViaDagre(rawNodes, edges);
  }, [rawNodes, edges]);

  const onConnect = useCallback(async (conn: Connection) => {
    if (!conn.source.startsWith("platform-") || !conn.target.startsWith("subgroup-")) return;
    if (patchingRef.current) return;
    patchingRef.current = true;
    const platName = conn.source.slice("platform-".length);
    const subName = conn.target.slice("subgroup-".length);
    const plat = platforms.find((p) => p.name === platName);
    if (!plat) { patchingRef.current = false; return; }
    const sg = subGroups.find((g) => g.subscriptionName === subName);
    if (!sg) { patchingRef.current = false; return; }
    try {
      await backupBeforeEdit();
      const newRegions = sg.regions.filter((r) => !(plat.region_filters ?? []).includes(r));
      const next = [...(plat.region_filters ?? []), ...newRegions];
      await ipcPlatformUpdate(platName, undefined, undefined, next);
      await sync();
    } catch { /* swallow */ }
    patchingRef.current = false;
  }, [platforms, subGroups, sync]);

  const onEdgesDelete = useCallback(async (delEdges: Edge[]) => {
    for (const e of delEdges) {
      if (e.deletable === false) continue;
      if (!e.source.startsWith("platform-") || !e.target.startsWith("subgroup-")) continue;
      if (patchingRef.current) continue;
      patchingRef.current = true;
      const platName = e.source.slice("platform-".length);
      const subName = e.target.slice("subgroup-".length);
      const plat = platforms.find((p) => p.name === platName);
      const sg = subGroups.find((g) => g.subscriptionName === subName);
      if (!plat || !sg) { patchingRef.current = false; continue; }
      try {
        await backupBeforeEdit();
        const next = (plat.region_filters ?? []).filter((r) => !sg.regions.includes(r));
        await ipcPlatformUpdate(platName, undefined, undefined, next);
        await sync();
      } catch { /* swallow */ }
      patchingRef.current = false;
    }
  }, [platforms, subGroups, sync]);

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
          nodesConnectable
          nodesDraggable
          connectionRadius={40}
          defaultEdgeOptions={{ type: "smoothstep", animated: true, style: { fontSize: 10 } }}
        >
          <Background variant={BackgroundVariant.Dots} gap={18} size={1.4} />
          <CanvasControls />
          <MiniMap pannable zoomable />
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
