import { useTranslation } from "react-i18next";
import {
  ReactFlow, Background, BackgroundVariant, MiniMap,
  Handle, Position, type Node, type Edge, type Connection, type NodeProps,
  useReactFlow, ReactFlowProvider, type OnMoveEnd,
} from "@xyflow/react";
import { useEffect, useMemo, useState, useCallback, useRef } from "react";
import "@xyflow/react/dist/style.css";
import { useAppStore } from "../store/appStore";
import {
  ipcPlatformListFull, ipcNodeList, ipcPlatformUpdate, ipcBackupCreate,
  ipcLeaseMap, ipcPortList, type LeaseEntry, type PortMapping,
} from "../lib/ipc";
import { loadTopologyViewport, saveTopologyViewport } from "../lib/settings";
import { strategyToI18nKey, mapResinToShell } from "../lib/strategy";
import { listen } from "@tauri-apps/api/event";
import type { ColorMode } from "@xyflow/react";
import { AlertTriangle, Loader2, ZoomIn, ZoomOut, Maximize, Lock, Unlock } from "lucide-react";

/// TopologyView - T9 canvas: three-column key-to-egress canvas with
/// subscription-folded C column, strategy-labeled edges, dual A+B badges,
/// full-area Handle hit zones, and i18n Controls.
///
///   A (Entry proxy port) --> B (Platforms) --strategy match--> C (IP channels / nodes)
///
/// - A: one node per port (ADR-0012 port=identity).
/// - B: one node per platform. Shows name, A+B dual strategy badges, leases.
/// - C: T9-1 — nodes grouped by subscription source (collapsible), each node
///   row shows display_tag + region + latency color + health.
/// - B->C edges: T9-2 — strategy-driven labels (region:HK / manual / quality>75).
///   Multiple strategies coexist = multiple edges.
/// - T9-3: Platform nodes show blue B-class + green A-class strategy badges.
/// - T9-4: Handle covers entire card area (drag-to-connect hits anywhere).
/// - T9-5: Custom CanvasControls with i18n tooltips replace built-in Controls.

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
  region?: string | null;
  tags?: { subscriptionName: string; tag: string }[];
}

/// T9-1: Subscription group replaces region group as the C-column unit.
/// One subscription = one collapsible card, containing node rows.
interface SubscriptionGroup {
  subscriptionName: string;
  nodes: NodeItem[];
  healthy: number;
  total: number;
  regions: string[];
}

/// Parse the Resin items-wrapper for platforms.
function parsePlatforms(raw: unknown): PlatformFull[] {
  if (!raw || typeof raw !== "object") return [];
  const v = raw as Record<string, unknown>;
  const items: PlatformFull[] = Array.isArray(v.items)
    ? v.items.filter((x): x is PlatformFull => !!x && typeof x === "object" && typeof (x as PlatformFull).name === "string")
    : Array.isArray(v)
      ? v.filter((x): x is PlatformFull => !!x && typeof x === "object" && typeof (x as PlatformFull).name === "string")
      : [];
  return items.map((p) => ({
    ...p,
    region_filters: Array.isArray(p.region_filters) ? (p.region_filters as string[]) : null,
    regex_filters: Array.isArray(p.regex_filters) ? (p.regex_filters as string[]) : null,
  }));
}

/// T9-1: Parse nodes and group by subscription source (replaces parseNodeGroups).
/// Each subscription group contains all nodes from that subscription,
/// with their individual region/health/latency info.
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
    // Derive subscription name from tags.
    let subName = "unknown";
    if (Array.isArray(n.tags)) {
      const subTag = n.tags.find((t) => t.subscriptionName);
      if (subTag) subName = subTag.subscriptionName;
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
        if (rt) return rt.tag.toLowerCase();
      }
      return "other";
    }))].sort();
    groups.push({ subscriptionName, nodes, healthy, total: nodes.length, regions });
  }
  return groups.sort((a, b) => a.subscriptionName.localeCompare(b.subscriptionName));
}

// Full-area Handle style: T9-4 — Handle covers entire card so drag-to-connect
// hits anywhere on the card, not just the small dot.
const fullAreaHandleStyle: React.CSSProperties = {
  opacity: 0,
  position: "absolute",
  width: "100%",
  height: "100%",
  top: 0,
  left: 0,
  pointerEvents: "auto",
  background: "transparent",
  border: "none",
};

/// Custom node: Entry port (A column).
function EntryPortNode({ data }: NodeProps) {
  const d = data as Record<string, unknown>;
  return (
    <div className="relative rounded-lg border border-blue-400 dark:border-blue-600 bg-blue-50 dark:bg-blue-950/50 px-4 py-3 text-xs min-w-[140px] max-w-[200px]">
      <Handle type="source" position={Position.Right} style={fullAreaHandleStyle} />
      <div className="font-semibold text-blue-700 dark:text-blue-300">
        {typeof d.port === "number" ? String(d.port) : String(d.label)}
      </div>
      <div className="text-blue-600/70 dark:text-blue-400/70 mt-0.5 text-[10px]">
        {typeof d.protocol === "string" ? d.protocol.toUpperCase() : ""}
        {typeof d.label === "string" && d.label ? " · " + d.label : ""}
      </div>
      {typeof d.boundPlatform === "string" && d.boundPlatform && (
        <div className="mt-1 text-[10px] text-blue-500/60 dark:text-blue-400/60 font-mono">
          → {d.boundPlatform}
          {typeof d.account === "string" && d.account ? " · " + d.account : ""}
        </div>
      )}
    </div>
  );
}

/// Custom node: Platform (B column) with T9-3 dual A+B strategy badges.
function PlatformNode({ data }: NodeProps) {
  const d = data as Record<string, unknown>;
  const leases = (Array.isArray(d.leases) ? d.leases : []) as Array<{
    account: string; egress_ip: string; target_domain: string;
  }>;
  const portChips = Array.isArray(d.ports) ? d.ports as number[] : [];
  // T9-3: A+B dual strategy badge data
  const bClassLabel = typeof d.bClassLabel === "string" ? d.bClassLabel : "";
  const aClassLabel = typeof d.aClassLabel === "string" ? d.aClassLabel : "";
  return (
    <div className="relative rounded-lg border border-zinc-400 dark:border-zinc-600 bg-white dark:bg-zinc-900 px-4 py-3 text-xs min-w-[160px] max-w-[240px]">
      <Handle type="target" position={Position.Left} style={fullAreaHandleStyle} />
      <Handle type="source" position={Position.Right} style={fullAreaHandleStyle} />
      <div className="font-semibold text-zinc-800 dark:text-zinc-100">{String(d.label)}</div>
      {typeof d.sub === "string" && d.sub && <div className="text-zinc-500 dark:text-zinc-400 mt-1 text-[10px]">{d.sub}</div>}
      {/* T9-3: A+B dual strategy badges */}
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
        <div className="mt-1 flex flex-wrap gap-1">{portChips.map((p) => (
          <span key={"port-" + p} className="rounded bg-blue-100 dark:bg-blue-900/40 px-1 py-0.5 text-[9px] text-blue-600 dark:text-blue-400 font-mono">:{p}</span>
        ))}</div>
      )}
      <div className="mt-2 flex flex-col gap-1">
        {leases.length > 0 ? leases.map((l, i) => (
          <div key={"lease-" + i} className="rounded bg-zinc-100 dark:bg-zinc-800 px-1.5 py-1 text-[10px] text-zinc-600 dark:text-zinc-300 font-mono">
            <span title={l.account}>{l.account.slice(0, 18) + (l.account.length > 18 ? "…" : "")}</span>
            {" → "}
            <span title={l.target_domain}>{(l.egress_ip || "").slice(0, 22) || "—"}</span>
          </div>
        )) : (
          <div className="text-[10px] text-zinc-400 dark:text-zinc-500">{typeof d.noLeases === "string" ? String(d.noLeases) : ""}</div>
        )}
      </div>
    </div>
  );
}

/// T9-1: Custom node: Subscription group (C column) — collapsible.
function SubscriptionGroupNode({ data }: NodeProps) {
  const d = data as Record<string, unknown>;
  const nodes = (Array.isArray(d.nodes) ? d.nodes : []) as Array<{
    display_tag: string; region: string; healthy: boolean; latencyColor: string;
  }>;
  return (
    <div className="relative rounded-lg border border-emerald-400 dark:border-emerald-600 bg-emerald-50 dark:bg-emerald-950/50 px-4 py-3 text-xs min-w-[180px] max-w-[280px]">
      <Handle type="target" position={Position.Left} style={fullAreaHandleStyle} />
      <div className="font-semibold text-emerald-700 dark:text-emerald-300">{String(d.label)}</div>
      {typeof d.sub === "string" && d.sub && <div className="text-emerald-600/70 dark:text-emerald-400/70 mt-1 text-[10px]">{d.sub}</div>}
      {/* T9-1: Node rows with latency color */}
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
    </div>
  );
}

const nodeTypes = { entryPort: EntryPortNode, platform: PlatformNode, subscriptionGroup: SubscriptionGroupNode };

/// Best-effort auto-backup before a topology edit.
async function backupBeforeEdit(): Promise<void> {
  try { await ipcBackupCreate(); } catch { /* best-effort, ignored */ }
}

/// Region filter helpers (unchanged from Q2-Bug1).
export function addRegionFilter(current: string[] | null, region: string): string[] {
  if (current && current.includes(region)) return current;
  return [...(current ?? []), region];
}

export function removeRegionFilter(current: string[] | null, region: string): string[] {
  return (current ?? []).filter((r) => r !== region);
}

/// C1-2: pure async helper — backup -> PATCH region_filters -> sync.
export async function patchAndSyncOnce(args: {
  platName: string;
  current: string[] | null;
  region: string;
  mode: "add" | "remove";
  sync: () => Promise<void>;
  ipcUpdate: (name: string, policy: string | undefined, sticky: string | undefined, regionFilters: string[]) => Promise<void>;
  backup?: () => Promise<void>;
}): Promise<{ patched: boolean; next: string[] }> {
  const { platName, current, region, mode, sync, ipcUpdate, backup } = args;
  if (mode === "add" && current && current.includes(region)) {
    return { patched: false, next: current };
  }
  let next: string[];
  if (mode === "add") next = addRegionFilter(current, region);
  else next = removeRegionFilter(current, region);
  if (backup) { try { await backup(); } catch { /* swallow */ } }
  await ipcUpdate(platName, undefined, undefined, next);
  await sync();
  return { patched: true, next };
}

/// T9-2: Edge builder with multi-strategy labels. Backward-compatible with
/// region-based groups (old test shape) AND subscription-based groups (new).
type EdgeWithLabel = { id: string; source: string; target: string; animated?: boolean; label?: string; deletable?: boolean };
export function buildEdges(
  platforms: { name: string; region_filters: string[] | null; allocation_policy?: string }[],
  nodeGroups: { region: string }[] | { subscriptionName: string; regions: string[] }[],
  ports: { port: number; platform_name: string }[] = [],
): EdgeWithLabel[] {
  const list: EdgeWithLabel[] = [];
  // A->B: entry port -> platform.
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
  // T9-2: B->C edges with strategy labels.
  // Detect whether nodeGroups is old shape (region) or new (subscriptionName).
  const isNewShape = nodeGroups.length > 0 && "subscriptionName" in (nodeGroups[0] as Record<string, unknown>);
  for (const p of platforms) {
    const regions = p.region_filters ?? [];
    for (const g of nodeGroups as any[]) {
      if (isNewShape) {
        // T9-1: subscription group — edge when any of the group's regions
        // intersects the platform's region_filters.
        const groupRegions: string[] = g.regions ?? [];
        const matched = regions.filter((r: string) => groupRegions.includes(r));
        if (matched.length > 0) {
          list.push({
            id: "e-" + p.name + "-" + g.subscriptionName,
            source: "platform-" + p.name,
            target: "subgroup-" + g.subscriptionName,
            label: "region:" + matched.join(","),
            deletable: false, // auto-strategy edges are non-deletable (T9-2)
          });
        }
      } else {
        // Old shape: region group (backward compat for existing tests).
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

/// T9-5: Custom CanvasControls with i18n tooltips (replaces built-in Controls).
function CanvasControls() {
  const { t } = useTranslation();
  const reactFlow = useReactFlow();
  const [locked, setLocked] = useState(false);
  return (
    <div className="absolute bottom-2 left-2 z-10 flex flex-col gap-1">
      <button
        title={t("topology.zoomIn")}
        className="rounded-md border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-800 p-1.5 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700 transition-colors"
        onClick={() => reactFlow.zoomIn()}
      >
        <ZoomIn size={16} />
      </button>
      <button
        title={t("topology.zoomOut")}
        className="rounded-md border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-800 p-1.5 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700 transition-colors"
        onClick={() => reactFlow.zoomOut()}
      >
        <ZoomOut size={16} />
      </button>
      <button
        title={t("topology.fitView")}
        className="rounded-md border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-800 p-1.5 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700 transition-colors"
        onClick={() => reactFlow.fitView({ maxZoom: 1 })}
      >
        <Maximize size={16} />
      </button>
      <button
        title={locked ? t("topology.unlock") : t("topology.lock")}
        className={"rounded-md border p-1.5 transition-colors " + (locked
          ? "border-blue-400 dark:border-blue-600 bg-blue-50 dark:bg-blue-950/50 text-blue-600 dark:text-blue-400"
          : "border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-800 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-700")}
        onClick={() => setLocked(!locked)}
      >
        {locked ? <Lock size={16} /> : <Unlock size={16} />}
      </button>
    </div>
  );
}

function TopologyCanvas() {
  const { t, i18n } = useTranslation();
  const theme = useAppStore((s) => s.theme);
  const reactFlow = useReactFlow();
  const [platforms, setPlatforms] = useState<PlatformFull[]>([]);
  const [subGroups, setSubGroups] = useState<SubscriptionGroup[]>([]);
  const [leases, setLeases] = useState<LeaseEntry[]>([]);
  const [ports, setPorts] = useState<PortMapping[]>([]);
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
  }, []);

  useEffect(() => {
    void sync();
    let unsub: (() => void) | null = null;
    void listen("sidecar-status", (evt) => {
      setSidecarStatus(String((evt as { payload: unknown }).payload ?? ""));
    }).then((fn) => { unsub = fn as (() => void); }).catch((e) => console.warn("[TopologyView] listen unlisten failed", e));
    return () => { if (unsub) { try { unsub(); } catch { /* ignore */ } } };
  }, [sync]);

  const onMoveEnd: OnMoveEnd = useCallback((_evt, viewport) => {
    if (!ready) return;
    if (!viewport || typeof viewport.x !== "number" || typeof viewport.y !== "number" || typeof viewport.zoom !== "number") return;
    try {
      void saveTopologyViewport({ x: viewport.x, y: viewport.y, zoom: viewport.zoom });
    } catch { /* vitest, ignore */ }
  }, [ready]);

  const onInit = useCallback((_instance: unknown) => {
    void (async () => {
      try {
        const vp = await loadTopologyViewport();
        if (vp && typeof vp.x === "number" && typeof vp.y === "number" && typeof vp.zoom === "number") {
          reactFlow.setViewport({ x: vp.x, y: vp.y, zoom: vp.zoom });
        } else {
          requestAnimationFrame(() => {
            try { reactFlow.fitView({ maxZoom: 1 }); } catch { /* vitest */ }
          });
        }
      } catch { /* vitest, no reactflow */ }
      requestAnimationFrame(() => setReady(true));
    })();
  }, [reactFlow]);

  const colorMode: ColorMode = theme;

  // T9-1: Build nodes — C column now uses subscription groups.
  const nodes: Node[] = useMemo(() => {
    if (!i18n.isInitialized || !i18n.language) return [];
    const list: Node[] = [];
    // A: entry ports
    ports.forEach((p, idx) => {
      list.push({
        id: "entry-port-" + p.port,
        type: "entryPort",
        position: { x: 0, y: 60 + idx * 90 },
        data: {
          port: p.port,
          protocol: p.protocol,
          label: p.label,
          boundPlatform: p.platform_name,
          account: p.account,
        },
      });
    });
    if (ports.length === 0) {
      list.push({
        id: "entry-port",
        type: "entryPort",
        position: { x: 0, y: 200 },
        data: { label: t("topology.entryPort"), port: 0, protocol: "", boundPlatform: "", account: "" },
      });
    }
    // B: platforms with T9-3 dual A+B strategy badges
    const leasesByPid = new Map<string, typeof leases>();
    for (const l of leases) {
      const pid = (l.platform_id || "").trim();
      const arr = leasesByPid.get(pid) ?? [];
      arr.push(l);
      leasesByPid.set(pid, arr);
    }
    platforms.forEach((p, i) => {
      const filters = p.regex_filters?.length
        ? t("topology.filters", { filters: p.regex_filters.join(", ") })
        : "";
      const routable = t("topology.routable", { count: p.routable_node_count });
      const sub = [filters, routable].filter(Boolean).join("\n");
      // T9-3: B-class badge label (from Resin allocation_policy -> shell -> i18n)
      const shellStrategy = mapResinToShell(p.allocation_policy ?? "BALANCED");
      const bClassLabel = t(strategyToI18nKey(shellStrategy));
      // T9-3: A-class badge label
      const aClassLabel = (p.region_filters?.length ?? 0) > 0
        ? "region:" + p.region_filters!.join(",").toUpperCase()
        : "manual";
      const pidLeases = leasesByPid.get(p.id) ?? [];
      list.push({
        id: "platform-" + p.name,
        type: "platform",
        position: { x: 300, y: 60 + i * 130 },
        data: {
          label: p.name,
          sub,
          leases: pidLeases,
          ports: ports.filter((x) => x.platform_name === p.name).map((x) => x.port),
          noLeases: t("topology.noLeases"),
          bClassLabel,
          aClassLabel,
        },
      });
    });
    // T9-1: C column — subscription groups (replaces region groups)
    subGroups.forEach((g, i) => {
      const healthLabel = g.healthy === g.total
        ? t("topology.healthy")
        : g.healthy + "/" + g.total + " " + t("topology.healthy");
      const regionsLabel = g.regions.length > 0
        ? g.regions.slice(0, 5).join(", ").toUpperCase() + (g.regions.length > 5 ? "+" : "")
        : "";
      const sub = [healthLabel, regionsLabel].filter(Boolean).join(" · ");
      // T9-1: Build node rows with latency color
      const nodeRows = g.nodes.map((n) => {
        const isHealthy = (n.failure_count ?? 0) === 0 && n.has_outbound !== false;
        let region = "other";
        if (n.region) region = n.region.toLowerCase();
        else if (Array.isArray(n.tags)) {
          const rt = n.tags.find((t2) => t2.tag && t2.tag.length <= 3);
          if (rt) region = rt.tag.toLowerCase();
        }
        // Latency color: green <200ms, yellow 200-500ms, red >500ms, gray timeout
        // Resin doesn't expose per-node latency in v1.2.0 node_list; use health as proxy.
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
        position: { x: 640, y: 60 + i * 120 },
        data: {
          label: g.subscriptionName,
          sub,
          nodes: nodeRows,
        },
      });
    });
    return list;
  }, [platforms, subGroups, leases, ports, t, i18n.isInitialized, i18n.language]);

  // T9-2: Edges with strategy labels
  const edges: Edge[] = useMemo(() => {
    // Use new subscription-group shape for edge building
    const adapted = subGroups.map((g) => ({ subscriptionName: g.subscriptionName, regions: g.regions }));
    return buildEdges(platforms, adapted, ports) as Edge[];
  }, [platforms, subGroups, ports]);

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
    // Add all regions from this subscription group to the platform's region_filters
    const current = plat.region_filters ?? [];
    const newRegions = sg.regions.filter((r) => !current.includes(r));
    if (newRegions.length === 0) { patchingRef.current = false; return; }
    const next = [...current, ...newRegions];
    try {
      await backupBeforeEdit();
      await ipcPlatformUpdate(platName, undefined, undefined, next);
      await sync();
    } catch { /* error surface in toast */ }
    patchingRef.current = false;
  }, [platforms, subGroups, sync]);

  const onEdgesDelete = useCallback(async (delEdges: Edge[]) => {
    for (const e of delEdges) {
      if (!e.source.startsWith("platform-") || !e.target.startsWith("subgroup-")) continue;
      if (patchingRef.current) continue;
      // T9-2: auto-strategy edges are non-deletable; only manual edges can be deleted.
      // Currently all edges are region-based (auto), so deletion is a no-op.
      // When manual strategy is added in future, this guard will check edge.data.strategyType.
    }
  }, []);

  return (
    <section className="h-full flex flex-col">
      {sidecarStatus === "unhealthy" && (
        <div className="mb-2 flex items-center gap-2 rounded-md border border-red-300 dark:border-red-800 bg-red-50 dark:bg-red-950/40 px-3 py-2 text-xs text-red-700 dark:text-red-300">
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
      {subGroups.length === 0 && (
        <div className="px-1 pb-2 text-xs text-zinc-400">{t("topology.noNodes")}</div>
      )}
      {platforms.length === 0 && (
        <div className="px-1 pb-2 text-xs text-zinc-400">{t("topology.noPlatforms")}</div>
      )}
      {ports.length === 0 && (
        <div className="px-1 pb-2 text-xs text-zinc-400">{t("topology.noPorts")}</div>
      )}
      <div className={"flex-1 min-h-[400px] transition-opacity duration-200 " + (ready ? "opacity-100" : "opacity-0")}>
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
