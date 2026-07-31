import { useTranslation } from "react-i18next";
import {
  ReactFlow, Background, BackgroundVariant, Controls, MiniMap,
  Handle, Position, type Node, type Edge, type Connection, type NodeProps,
} from "@xyflow/react";
import { useEffect, useMemo, useState, useCallback } from "react";
import "@xyflow/react/dist/style.css";
import { useAppStore } from "../store/appStore";
import {
  ipcPlatformListFull, ipcNodeList, ipcPlatformUpdate,
} from "../lib/ipc";
import { listen } from "@tauri-apps/api/event";
import type { ColorMode } from "@xyflow/react";
import { AlertTriangle } from "lucide-react";

/// TopologyView — Phase R2 three-column key-to-egress canvas.
///
///   A (Entry proxy port)  ──always──>  B (Platforms)  ──region match──>  C (IP channels / nodes)
///
/// - A: single node, the Resin forward-proxy listen port (from settings gatewayBind).
/// - B: one node per platform (from platform_list_full). Shows name, upstream
///   regex_filters, allocation_policy, routable_node_count.
/// - C: one node per NODE-REGION-GROUP (from node_list, grouped by region).
///   Shows region + healthy count. Resin does NOT expose per-node egress IPs;
///   the canvas shows display_tag + health.
/// - B->C edges: drawn when the platform's region_filters includes the C group's
///   region. Dragging a new edge from B to a C group = PATCH the platform's
///   region_filters to include that region (live hot-switch). Deleting an edge
///   = PATCH region_filters to remove that region.
///
/// The Resin sidecar owns the actual key->lane->exit-IP mapping; the canvas
/// only MIRRORS the platform/region binding and lets the user hot-edit it.

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

interface NodeGroup {
  region: string;
  nodes: NodeItem[];
  healthy: number;
  total: number;
}

/// Parse the Resin items-wrapper (or bare array) into a typed list.
function parsePlatforms(raw: unknown): PlatformFull[] {
  if (!raw || typeof raw !== "object") return [];
  const v = raw as Record<string, unknown>;
  const items = Array.isArray(v.items) ? v.items : (Array.isArray(v) ? v : []);
  return items.filter((x): x is Record<string, unknown> => !!x && typeof x === "object").map((p) => ({
    id: String(p.id ?? ""),
    name: String(p.name ?? ""),
    regex_filters: Array.isArray(p.regex_filters) ? (p.regex_filters as string[]) : null,
    region_filters: Array.isArray(p.region_filters) ? (p.region_filters as string[]) : null,
    allocation_policy: String(p.allocation_policy ?? "BALANCED"),
    routable_node_count: Number(p.routable_node_count ?? 0),
    sticky_ttl: String(p.sticky_ttl ?? "168h0m0s"),
  }));
}

/// Parse the Resin items-wrapper for nodes and group by region.
function parseNodeGroups(raw: unknown): NodeGroup[] {
  if (!raw || typeof raw !== "object") return [];
  const v = raw as Record<string, unknown>;
  const items: NodeItem[] = Array.isArray(v.items)
    ? v.items.filter((x): x is NodeItem => !!x && typeof x === "object")
    : Array.isArray(v)
      ? v.filter((x): x is NodeItem => !!x && typeof x === "object")
      : [];
  const byRegion = new Map<string, NodeItem[]>();
  for (const n of items) {
    // Derive region from tags or region field. Resin nodes carry region in tags
    // (tag: "HK") or a region field. Fallback: "other".
    let region = "other";
    if (n.region) region = n.region.toLowerCase();
    else if (Array.isArray(n.tags)) {
      const regionTag = n.tags.find((t) => t.tag && t.tag.length <= 3);
      if (regionTag) region = regionTag.tag.toLowerCase();
    }
    const arr = byRegion.get(region) ?? [];
    arr.push(n);
    byRegion.set(region, arr);
  }
  const groups: NodeGroup[] = [];
  for (const [region, nodes] of byRegion) {
    const healthy = nodes.filter((n) => (n.failure_count ?? 0) === 0 && n.has_outbound !== false).length;
    groups.push({ region, nodes, healthy, total: nodes.length });
  }
  return groups.sort((a, b) => a.region.localeCompare(b.region));
}

/// Custom node components so the canvas shows structured content, not a bare label.
function EntryNode({ data }: NodeProps) {
  return (
    <div className="rounded-lg border border-blue-400 dark:border-blue-600 bg-blue-50 dark:bg-blue-950/50 px-4 py-3 text-xs min-w-[140px]">
      <Handle type="source" position={Position.Right} />
      <div className="font-semibold text-blue-700 dark:text-blue-300">{String(data.label)}</div>
    </div>
  );
}

function PlatformNode({ data }: NodeProps) {
  const d = data as Record<string, unknown>;
  return (
    <div className="rounded-lg border border-zinc-400 dark:border-zinc-600 bg-white dark:bg-zinc-900 px-4 py-3 text-xs min-w-[160px] max-w-[220px]">
      <Handle type="target" position={Position.Left} />
      <Handle type="source" position={Position.Right} />
      <div className="font-semibold text-zinc-800 dark:text-zinc-100">{String(d.label)}</div>
      {typeof d.sub === "string" && d.sub && <div className="text-zinc-500 dark:text-zinc-400 mt-1 text-[10px]">{d.sub}</div>}
    </div>
  );
}

function NodeGroupNode({ data }: NodeProps) {
  const d = data as Record<string, unknown>;
  return (
    <div className="rounded-lg border border-emerald-400 dark:border-emerald-600 bg-emerald-50 dark:bg-emerald-950/50 px-4 py-3 text-xs min-w-[140px]">
      <Handle type="target" position={Position.Left} />
      <div className="font-semibold text-emerald-700 dark:text-emerald-300">{String(d.label)}</div>
      {typeof d.sub === "string" && d.sub && <div className="text-emerald-600/70 dark:text-emerald-400/70 mt-1 text-[10px]">{d.sub}</div>}
    </div>
  );
}

const nodeTypes = { entry: EntryNode, platform: PlatformNode, nodeGroup: NodeGroupNode };

export function TopologyView() {
  const { t, i18n } = useTranslation();
  const theme = useAppStore((s) => s.theme);
  // The entry port is the Resin forward-proxy listen port (owned by the
  // sidecar). We render it as a conceptual entry point; the exact port is
  // in settings.json gatewayBind but the canvas does not need it to draw.
  const [platforms, setPlatforms] = useState<PlatformFull[]>([]);
  const [nodeGroups, setNodeGroups] = useState<NodeGroup[]>([]);
  const [sidecarStatus, setSidecarStatus] = useState<"healthy" | "unhealthy" | null>(null);
  const [patching, setPatching] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<string>("sidecar-status", (e) => {
      setSidecarStatus(e.payload as "healthy" | "unhealthy");
    }).then((fn) => { unlisten = fn; }).catch(() => {});
    return () => { if (unlisten) unlisten(); };
  }, []);

  const sync = useCallback(async () => {
    try {
      const [plRaw, nRaw] = await Promise.all([
        ipcPlatformListFull(),
        ipcNodeList(),
      ]);
      setPlatforms(parsePlatforms(plRaw));
      setNodeGroups(parseNodeGroups(nRaw));
    } catch {
      // Outside Tauri (vitest) or sidecar down — keep last state.
    }
  }, []);

  useEffect(() => {
    void sync();
    const id = setInterval(() => void sync(), 5000);
    // Bug #2 fix: re-sync on refocus so the canvas never stays blank.
    const onVis = () => { if (!document.hidden) void sync(); };
    document.addEventListener("visibilitychange", onVis);
    return () => { clearInterval(id); document.removeEventListener("visibilitychange", onVis); };
  }, [sync]);

  /// Drag-to-connect: when the user draws an edge from a platform to a node-group,
  /// PATCH the platform's region_filters to include that region. This is the
  /// hot-switch — the canvas edge appears immediately, and Resin picks it up.
  const onConnect = useCallback(async (conn: Connection) => {
    // source = platform-<name>, target = nodegroup-<region>
    if (!conn.source || !conn.target) return;
    if (!conn.source.startsWith("platform-") || !conn.target.startsWith("nodegroup-")) return;
    const platName = conn.source.slice("platform-".length);
    const region = conn.target.slice("nodegroup-".length);
    const plat = platforms.find((p) => p.name === platName);
    if (!plat) return;
    const current = plat.region_filters ?? [];
    if (current.includes(region)) return; // already bound
    const next = [...current, region];
    setPatching(true);
    try {
      await ipcPlatformUpdate(platName, undefined, undefined, next);
      // Optimistic local update so the edge appears instantly.
      setPlatforms((prev) => prev.map((p) =>
        p.name === platName ? { ...p, region_filters: next } : p
      ));
    } catch {
      // Revert on failure.
      void sync();
    } finally {
      setPatching(false);
    }
  }, [platforms, sync]);

  /// Delete edge = remove the region from the platform's region_filters.
  const onEdgesDelete = useCallback(async (edges: Edge[]) => {
    for (const e of edges) {
      if (!e.source.startsWith("platform-") || !e.target.startsWith("nodegroup-")) continue;
      const platName = e.source.slice("platform-".length);
      const region = e.target.slice("nodegroup-".length);
      const plat = platforms.find((p) => p.name === platName);
      if (!plat || !plat.region_filters) continue;
      const next = plat.region_filters.filter((r) => r !== region);
      setPatching(true);
      try {
        await ipcPlatformUpdate(platName, undefined, undefined, next.length > 0 ? next : []);
        setPlatforms((prev) => prev.map((p) =>
          p.name === platName ? { ...p, region_filters: next.length > 0 ? next : null } : p
        ));
      } catch {
        void sync();
      } finally {
        setPatching(false);
      }
    }
  }, [platforms, sync]);

  const colorMode: ColorMode = theme;

  // Build the three columns.
  const nodes: Node[] = useMemo(() => {
    const list: Node[] = [];
    // A: entry port (single node).
    const port = "forward proxy";
    list.push({
      id: "entry-port",
      type: "entry",
      position: { x: 0, y: 200 },
      data: { label: `${t("topology.entryPort")}:\n${port}` },
    });
    // B: platforms.
    platforms.forEach((p, i) => {
      const filters = p.regex_filters?.length
        ? t("topology.filters", { filters: p.regex_filters.join(", ") })
        : "";
      const policy = t("topology.policy", { policy: p.allocation_policy });
      const routable = t("topology.routable", { count: p.routable_node_count });
      const sub = [filters, policy, routable].filter(Boolean).join("\n");
      list.push({
        id: `platform-${p.name}`,
        type: "platform",
        position: { x: 300, y: 60 + i * 130 },
        data: { label: p.name, sub },
      });
    });
    // C: node groups by region.
    nodeGroups.forEach((g, i) => {
      const healthLabel = g.healthy === g.total
        ? t("topology.healthy")
        : `${g.healthy}/${g.total} ${t("topology.healthy")}`;
      list.push({
        id: `nodegroup-${g.region}`,
        type: "nodeGroup",
        position: { x: 640, y: 60 + i * 100 },
        data: { label: t("topology.region", { region: g.region }), sub: healthLabel },
      });
    });
    return list;
  }, [platforms, nodeGroups, t, i18n.language]);

  // Edges: A->B always connected; B->C when region_filters matches.
  const edges: Edge[] = useMemo(() => {
    const list: Edge[] = [];
    // A->B: entry port connects to every platform.
    for (const p of platforms) {
      list.push({
        id: `e-entry-${p.name}`,
        source: "entry-port",
        target: `platform-${p.name}`,
        animated: true,
      });
    }
    // B->C: platform -> node-group when region_filters includes the group region.
    for (const p of platforms) {
      const regions = p.region_filters ?? [];
      for (const g of nodeGroups) {
        if (regions.includes(g.region)) {
          list.push({
            id: `e-${p.name}-${g.region}`,
            source: `platform-${p.name}`,
            target: `nodegroup-${g.region}`,
          });
        }
      }
    }
    return list;
  }, [platforms, nodeGroups]);

  return (
    <section className="h-full flex flex-col">
      {sidecarStatus === "unhealthy" && (
        <div className="mb-2 flex items-center gap-2 rounded-md border border-red-300 dark:border-red-800 bg-red-50 dark:bg-red-950/40 px-3 py-2 text-xs text-red-700 dark:text-red-300">
          <AlertTriangle size={14} className="shrink-0" />
          <span>{t("topology.sidecarUnhealthy")}</span>
        </div>
      )}
      <div className="flex items-center justify-between px-1 pb-2">
        <span className="text-xs text-zinc-500 dark:text-zinc-400">{t("topology.dragHint")}</span>
        {patching && (
          <span className="text-xs font-mono text-amber-500">PATCH…</span>
        )}
      </div>
      {platforms.length === 0 && (
        <div className="px-1 pb-2 text-xs text-zinc-400">{t("topology.noPlatforms")}</div>
      )}
      {nodeGroups.length === 0 && (
        <div className="px-1 pb-2 text-xs text-zinc-400">{t("topology.noNodes")}</div>
      )}
      <div className="flex-1 border border-zinc-200 dark:border-zinc-800 rounded">
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          onConnect={onConnect}
          onEdgesDelete={onEdgesDelete}
          fitView
          colorMode={colorMode}
        >
          <Background variant={BackgroundVariant.Dots} gap={18} size={1.4} />
          <Controls />
          <MiniMap pannable zoomable />
        </ReactFlow>
      </div>
    </section>
  );
}
