import { useTranslation } from "react-i18next";
import {
  ReactFlow, Background, BackgroundVariant, Controls, MiniMap,
  Handle, Position, type Node, type Edge, type Connection, type NodeProps,
  useReactFlow, ReactFlowProvider, type OnMoveEnd,
} from "@xyflow/react";
import { useEffect, useMemo, useState, useCallback, useRef } from "react";
import "@xyflow/react/dist/style.css";
import { useAppStore } from "../store/appStore";
import {
  ipcPlatformListFull, ipcNodeList, ipcPlatformUpdate, ipcBackupCreate,
  ipcLeaseMap, ipcPortList, ipcWhiteboxReload, type LeaseEntry, type PortMapping,
} from "../lib/ipc";
import { loadTopologyViewport, saveTopologyViewport } from "../lib/settings";
import { strategyToI18nKey } from "../lib/strategy";
import { listen } from "@tauri-apps/api/event";
import type { ColorMode } from "@xyflow/react";
import { AlertTriangle, Loader2 } from "lucide-react";

/// TopologyView - Phase R2 three-column key-to-egress canvas.
///
///   A (Entry proxy port) --> B (Platforms) --region match--> C (IP channels / nodes)
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
/// - P19 item 4: every drag edit (onConnect / onEdgesDelete) auto-creates a
///   Resin config backup BEFORE the PATCH. If the user breaks the routing,
///   they can Settings > Config > Import to roll back. Best-effort: backup_create
///   is wrapped (catch -> ignore) so a backup failure never blocks the edit.
/// - P19 item 1: viewport (x, y, zoom) persists to settings.json
///   topologyViewport on every onMoveEnd; on mount we setViewport back so the
///   user lands at the exact pan/zoom they left.
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
  /// T4-5: subscription sources that contribute nodes to this region group.
  subscriptions: string[];
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
    // T4-5: collect unique subscription names from node tags.
    const subs = new Set<string>();
    for (const n of nodes) {
      if (Array.isArray(n.tags)) {
        for (const t of n.tags) {
          if (t.subscriptionName) subs.add(t.subscriptionName);
          else if (t.tag && t.tag.length > 3) subs.add(t.tag);
        }
      }
    }
    groups.push({ region, nodes, healthy, total: nodes.length, subscriptions: [...subs].sort() });
  }
  return groups.sort((a, b) => a.region.localeCompare(b.region));
}

/// Custom node components so the canvas shows structured content, not a bare label.
/// ADR-0012: EntryPortNode renders one actual listener port (socks5/http).
/// Each port IS the identity (port=identity per ADR-0012). Shows port number,
/// protocol, label, and the bound platform_name + account.
function EntryPortNode({ data }: NodeProps) {
  const d = data as Record<string, unknown>;
  return (
    <div className="rounded-lg border border-blue-400 dark:border-blue-600 bg-blue-50 dark:bg-blue-950/50 px-4 py-3 text-xs min-w-[140px] max-w-[200px]">
      <Handle type="source" position={Position.Right} />
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

/// A4-3: PlatformNode now renders a small chip list of the platform's active
/// leases (account short + egress_ip + target domain). Each lease is one
/// Resin V1 Platform.Account identity on the lease; an independent egress IP proves
/// the (key, endpoint) -> distinct IP contract is live, not prose.
function PlatformNode({ data }: NodeProps) {
  const d = data as Record<string, unknown>;
  const leases = (Array.isArray(d.leases) ? d.leases : []) as Array<{
    account: string; egress_ip: string; target_domain: string;
  }>;
  const portChips = Array.isArray(d.ports) ? d.ports as number[] : [];
  return (
    <div className="rounded-lg border border-zinc-400 dark:border-zinc-600 bg-white dark:bg-zinc-900 px-4 py-3 text-xs min-w-[160px] max-w-[240px]">
      <Handle type="target" position={Position.Left} />
      <Handle type="source" position={Position.Right} />
      <div className="font-semibold text-zinc-800 dark:text-zinc-100">{String(d.label)}</div>
      {typeof d.sub === "string" && d.sub && <div className="text-zinc-500 dark:text-zinc-400 mt-1 text-[10px]">{d.sub}</div>}
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

const nodeTypes = { entryPort: EntryPortNode, platform: PlatformNode, nodeGroup: NodeGroupNode };

/// Best-effort auto-backup before a topology edit (P19 item 4). Failures are
/// swallowed: a broken backup should never block a user's routing change.
async function backupBeforeEdit(): Promise<void> {
  try { await ipcBackupCreate(); } catch { /* best-effort, ignored */ }
}

/// Inner canvas owns the actual <ReactFlow> and needs useReactFlow(), which
/// requires a <ReactFlowProvider> ancestor. TopologyView is the exported shell
/// that wires the provider so callers can mount <TopologyView /> directly.


/// Q2-Bug1 closed-loop: pure helpers for region_filters add/remove so the
/// dedup + idempotency contract is unit-tested without a live ReactFlow drag.
/// - addRegion: returns next array (or null-safe empty) with the region exactly
///   once. Already-present -> returns the same reference (idempotent, no PATCH).
/// - removeRegion: returns the array without the region; empty -> [] (never null,
///   so the PATCH always sends an array the server accepts).
export function addRegionFilter(current: string[] | null, region: string): string[] {
  const cur = current ?? [];
  if (cur.includes(region)) return cur; // idempotent: no-op, caller should skip PATCH
  return [...cur, region];
}
export function removeRegionFilter(current: string[] | null, region: string): string[] {
  const cur = current ?? [];
  return cur.filter((r) => r !== region);
}

// C1-2: pure async helper that does backup -> PATCH region_filters -> sync.
// Extracted from onConnect/onEdgesDelete so vitest can mock the deps and
// assert the PATCH+sync ordering + the already-bound skip without a live
// ReactFlow. The `alreadyBound` short-circuit lives at the call site
// (onConnect checks `current.includes(region)`) so the racy double-PATCH
// guard is the idempotent addRegionFilter returning the same ref BEFORE we
// reach this helper.|| P24-Q2 patchingRef guards the second concurrent drag.
export async function patchAndSyncOnce(args: {
  platName: string;
  current: string[] | null;
  region: string;
  mode: "add" | "remove";
  sync: () => Promise<void>;
  ipcUpdate: (name: string, _a: undefined, _b: undefined, filters: string[]) => Promise<unknown>;
  backup?: () => Promise<void>;
}): Promise<{ patched: boolean; next: string[] }> {
  const { platName, current, region, mode, sync, ipcUpdate, backup } = args;
  const cur = current ?? [];
  const next = mode === "add" ? addRegionFilter(cur, region) : removeRegionFilter(cur, region);
  if (mode === "add" && cur.includes(region)) {
    // already bound -> idempotent skip; no PATCH, no sync.
    return { patched: false, next: cur };
  }
  if (backup) { try { await backup(); } catch { /* swallow: backup failure never blocks the routing change */ } }
  await ipcUpdate(platName, undefined, undefined, next.length > 0 ? next : []);
  await sync(); // always re-GET from the server so the canvas reflects the authoritative post-PATCH state.
  return { patched: true, next };
}

// C1-3: pure edge builder. Edge list = A->B entry edges (one per platform)
// + B->C platform->nodeGroup edge when the platform region_filters
// includes the node group region. Extracted from the useMemo so vitest
// can assert that removing a region from region_filters deletes the edge
// without mounting a live ReactFlow.
type EdgeWithLabel = { id: string; source: string; target: string; animated?: boolean; label?: string };
export function buildEdges(
  platforms: { name: string; region_filters: string[] | null; allocation_policy?: string }[],
  nodeGroups: { region: string }[],
  ports: { port: number; platform_name: string }[] = [],
): EdgeWithLabel[] {
  const list: EdgeWithLabel[] = [];
  // A->B: entry port -> platform (by platform_name match).
  for (const p of ports) {
    const plat = platforms.find((x) => x.name === p.platform_name);
    if (plat) {
      list.push({ id: "e-port-" + p.port + "-" + p.platform_name, source: "entry-port-" + p.port, target: "platform-" + p.platform_name, animated: true });
    }
  }
  // If no ports defined yet, fall back to A->B for every platform (scaffolding edge).
  if (ports.length === 0) {
    for (const p of platforms) {
      list.push({ id: "e-entry-" + p.name, source: "entry-port", target: "platform-" + p.name, animated: true });
    }
  }
  // B->C: platform -> nodeGroup (by region_filters match).
  // T4-5: edges carry strategy labels so the canvas shows WHY the binding exists.
  for (const p of platforms) {
    const regions = p.region_filters ?? [];
    for (const g of nodeGroups) {
      if (regions.includes(g.region)) {
        list.push({
          id: "e-" + p.name + "-" + g.region,
          source: "platform-" + p.name,
          target: "nodegroup-" + g.region,
          label: "region:" + g.region,
        });
      }
    }
  }
  return list;
}

function TopologyCanvas() {
  const { t, i18n } = useTranslation();
  const theme = useAppStore((s) => s.theme);
  const reactFlow = useReactFlow();
  // The entry port is the Resin forward-proxy listen port (owned by the
  // sidecar). We render it as a conceptual entry point; the exact port is
  // in settings.json gatewayBind but the canvas does not need it to draw.
  const [platforms, setPlatforms] = useState<PlatformFull[]>([]);
  const [nodeGroups, setNodeGroups] = useState<NodeGroup[]>([]);
  const [ports, setPorts] = useState<PortMapping[]>([]);
  const [leases, setLeases] = useState<LeaseEntry[]>([]);
  const [sidecarStatus, setSidecarStatus] = useState<"healthy" | "unhealthy" | "restarting" | "terminated" | null>(null);
  const [patching, setPatching] = useState(false);
  // Q2-Bug1: ref-based reentry lock so two rapid drags cannot both read a stale
  // region_filters snapshot and race the PATCH (the state update is async; the
  // state flag is not a reliable guard inside the same handler invocation).
  const patchingRef = useRef(false);
  // T6-6: tracks previous sidecar state for the port rebind sync transition.
  const sidecarStatusRef = useRef<string | null>(null);
  // P19 item 1: tracks whether we have already restored the saved viewport so
  // the conditional fitView() only runs on first paint when no previous
  // viewport was persisted. Without this gate, ReactFlow fitView() would snap
  // back to a framed view on every refresh.
  // P20 item 1: gate canvas visibility until viewport is either restored
  // from settings or fitView'd. Hides the initial-position flash that the
  // old fitView={!viewportRestored} prop caused - ReactFlow rendered a
  // framed view then setViewport() snapped to the saved spot, visible to the user.
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<string>("sidecar-status", (e) => {
      const prev = sidecarStatusRef.current;
      const next = e.payload as "healthy" | "unhealthy" | "restarting" | "terminated";
      sidecarStatusRef.current = next;
      setSidecarStatus(next);
      // T6-6: port rebind sync — when sidecar recovers from restarting -> healthy,
      // reload the whitebox port mappings so the DB + listeners are re-synced.
      if (prev === "restarting" && next === "healthy") {
        void ipcWhiteboxReload().then(() => sync()).catch(() => {});
      }
    }).then((fn) => { unlisten = fn; }).catch(() => {});
    return () => { if (unlisten) unlisten(); };
  }, []);

  const sync = useCallback(async () => {
    try {
      const [plRaw, nRaw, lRaw, pRaw] = await Promise.all([
        ipcPlatformListFull(),
        ipcNodeList(),
        ipcLeaseMap(),
        ipcPortList(),
      ]);
      setPlatforms(parsePlatforms(plRaw));
      setNodeGroups(parseNodeGroups(nRaw));
      setLeases(Array.isArray(lRaw) ? lRaw : []);
      setPorts(Array.isArray(pRaw) ? pRaw : []);
    } catch {
      // Outside Tauri (vitest) or sidecar down - keep last state.
    }
  }, []);

  useEffect(() => {
    void sync();
    // P20 item 1: viewport restore is now done in onInit (below) so it runs
    // after the ReactFlow instance has mounted. The old code called
    // reactFlow.setViewport() in a useEffect that fired BEFORE ReactFlow's
    // internal init - the setViewport was a no-op and the fitView prop
    // rendered a framed view, causing the flash. onInit fires once RF is up.
    const id = setInterval(() => void sync(), 5000);
    // Bug #2 fix: re-sync on refocus so the canvas never stays blank.
    const onVis = () => { if (!document.hidden) void sync(); };
    document.addEventListener("visibilitychange", onVis);
    return () => { clearInterval(id); document.removeEventListener("visibilitychange", onVis); };
  }, [sync, reactFlow]);

  /// Drag-to-connect: when the user draws an edge from a platform to a node-group,
  /// PATCH the platform's region_filters to include that region. This is the
  /// hot-switch - the canvas edge appears immediately, and Resin picks it up.
  /// P19 item 4: best-effort backup BEFORE the PATCH so the change is reversible.
  const onConnect = useCallback(async (conn: Connection) => {
    // source = platform-<name>, target = nodegroup-<region>
    if (!conn.source || !conn.target) return;
    if (!conn.source.startsWith("platform-") || !conn.target.startsWith("nodegroup-")) return;
    // Q2-Bug1: reentry guard via ref (state flag is async, unreliable in-handler)
    if (patchingRef.current) return;
    const platName = conn.source.slice("platform-".length);
    const region = conn.target.slice("nodegroup-".length);
    const plat = platforms.find((p) => p.name === platName);
    if (!plat) return;
    const current = plat.region_filters ?? [];
    if (current.includes(region)) return; // already bound (idempotent)
    patchingRef.current = true;
    setPatching(true);
    try {
      await patchAndSyncOnce({ platName, current: plat.region_filters, region, mode: "add", sync, ipcUpdate: ipcPlatformUpdate, backup: backupBeforeEdit });
    } catch {
      await sync(); // server rejected -> resync to drop the stale optimistic edge
    } finally {
      patchingRef.current = false;
      setPatching(false);
    }
  }, [platforms, sync]);

  /// Delete edge = remove the region from the platform's region_filters.
  /// P19 item 4: best-effort backup BEFORE the PATCH so the change is reversible.
  const onEdgesDelete = useCallback(async (edges: Edge[]) => {
    if (patchingRef.current) return; // Q2-Bug1: reentry guard
    patchingRef.current = true;
    setPatching(true);
    try {
      for (const e of edges) {
        if (!e.source.startsWith("platform-") || !e.target.startsWith("nodegroup-")) continue;
        const platName = e.source.slice("platform-".length);
        const region = e.target.slice("nodegroup-".length);
        const plat = platforms.find((p) => p.name === platName);
        if (!plat || !plat.region_filters) continue;
        await patchAndSyncOnce({ platName, current: plat.region_filters, region, mode: "remove", sync, ipcUpdate: ipcPlatformUpdate, backup: backupBeforeEdit });
      }
    } catch {
      await sync();
    } finally {
      patchingRef.current = false;
      setPatching(false);
    }
  }, [platforms, sync]);

  /// P19 item 1: save the viewport after the user finishes panning/zooming so
  /// the next mount can restore it. Skipped until the initial restore completes
  /// so the setViewport-from-storage call does not trigger an immediate
  /// onMoveEnd that overwrites the value we just read.
  const onMoveEnd: OnMoveEnd = useCallback((_evt, viewport) => {
    if (!ready) return;
    if (!viewport || typeof viewport.x !== "number" || typeof viewport.y !== "number" || typeof viewport.zoom !== "number") return;
    try {
      void saveTopologyViewport({ x: viewport.x, y: viewport.y, zoom: viewport.zoom });
    } catch { /* vitest, ignore */ }
  }, [ready]);

  /// P20 item 1: onInit fires once ReactFlow is mounted and ready to accept
  /// setViewport / fitView. We either restore the saved viewport (no flash) or
  /// fitView({ maxZoom: 1 }) to frame all content (the new default initial view
  /// that covers every node). Either way, after it runs we flip `ready` so the
  /// opacity gate lifts and onMoveEnd starts persisting.
  const onInit = useCallback((_instance: unknown) => {
    void (async () => {
      try {
        const vp = await loadTopologyViewport();
        if (vp && typeof vp.x === "number" && typeof vp.y === "number" && typeof vp.zoom === "number") {
          reactFlow.setViewport({ x: vp.x, y: vp.y, zoom: vp.zoom });
        } else {
          // No saved viewport: default to fitView covering ALL content.
          // maxZoom: 1 prevents zoom-in on small graphs (P20 item 1).
          requestAnimationFrame(() => {
            try { reactFlow.fitView({ maxZoom: 1 }); } catch { /* vitest */ }
          });
        }
      } catch { /* vitest, no reactflow */ }
      // Lift the visibility gate on the next frame so the user never sees the
      // pre-restore layout. rAF waits one paint, hiding the flash.
      requestAnimationFrame(() => setReady(true));
    })();
  }, [reactFlow]);

  const colorMode: ColorMode = theme;

  // C2-7: i18n.isInitialized gate. Until i18n has finished init/changeLanguage, return an
  // empty node list so we never paint a box with the fallback-locale text (the
  // "刚打开 GUI 是 zh, 画布内节点框还是 en" symptom from P13 B7 followup). Mirrors the
  // useTranslation() ready pattern documented at https://react.i18next.com/latest/usetranslation-hook
  // (i18n.language alone does not block the initial paint when lazy-loaded chunks
  // resolve after first mount).
  const nodes: Node[] = useMemo(() => {
    if (!i18n.isInitialized || !i18n.language) return [];
    const list: Node[] = [];
    // A: entry ports (one node per port from ipcPortList). ADR-0012: port=identity.
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
    // Fallback: if no ports configured, show a single placeholder so the canvas is not empty.
    if (ports.length === 0) {
      list.push({
        id: "entry-port",
        type: "entryPort",
        position: { x: 0, y: 200 },
        data: { label: t("topology.entryPort"), port: 0, protocol: "", boundPlatform: "", account: "" },
      });
    }
    // B: platforms. A4-3: attach the platform's active leases (matched on
    // platform_id) so the chip list under each card proves the
    // port-identity -> platform -> egress-IP contract (ADR-0012).
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
      const policy = t("topology.policy", { policy: t(strategyToI18nKey(p.allocation_policy ?? "BALANCED")) });
      const routable = t("topology.routable", { count: p.routable_node_count });
      // T4-5: include A-class strategy summary (how many region_filters bound).
      const aClassSummary = (p.region_filters?.length ?? 0) > 0
        ? "A: region(" + p.region_filters!.length + ")"
        : "A: manual";
      const sub = [filters, policy, routable, aClassSummary].filter(Boolean).join("\n");
      const pidLeases = leasesByPid.get(p.id) ?? [];
      list.push({
        id: "platform-" + p.name,
        type: "platform",
        position: { x: 300, y: 60 + i * 130 },
        data: { label: p.name, sub, leases: pidLeases, ports: ports.filter((x) => x.platform_name === p.name).map((x) => x.port), noLeases: t("topology.noLeases") },
      });
    });
    // C: node groups by region.
    nodeGroups.forEach((g, i) => {
      const healthLabel = g.healthy === g.total
        ? t("topology.healthy")
        : g.healthy + "/" + g.total + " " + t("topology.healthy");
      // T4-5: show subscription sources below the region label.
      const subsLabel = g.subscriptions.length > 0
        ? g.subscriptions.slice(0, 3).join(", ") + (g.subscriptions.length > 3 ? "+" : "")
        : "";
      const sub = [healthLabel, subsLabel].filter(Boolean).join(" · ");
      list.push({
        id: "nodegroup-" + g.region,
        type: "nodeGroup",
        position: { x: 640, y: 60 + i * 100 },
        data: { label: t("topology.region", { region: g.region }), sub, subscriptions: g.subscriptions },
      });
    });
    return list;
  }, [platforms, nodeGroups, leases, t, i18n.isInitialized, i18n.language]);

  // Edges: A->B always connected; B->C when region_filters matches.
  const edges: Edge[] = useMemo(() => buildEdges(platforms, nodeGroups, ports) as Edge[], [platforms, nodeGroups, ports]);

  return (
    <section className="h-full flex flex-col">
      {sidecarStatus === "unhealthy" && (
        <div className="mb-2 flex items-center gap-2 rounded-md border border-red-300 dark:border-red-800 bg-red-50 dark:bg-red-950/40 px-3 py-2 text-xs text-red-700 dark:text-red-300">
          <AlertTriangle size={14} className="shrink-0" />
          <span>{t("topology.sidecarUnhealthy")}</span>
        </div>
      )}
      {sidecarStatus === "restarting" && (
        <div className="mb-2 flex items-center gap-2 rounded-md border border-blue-300 dark:border-blue-800 bg-blue-50 dark:bg-blue-950/40 px-3 py-2 text-xs text-blue-700 dark:text-blue-300" data-testid="topology-restarting-banner">
          <Loader2 size={14} className="shrink-0 animate-spin" />
          <span>{t("topology.sidecarRestarting")}</span>
        </div>
      )}
      {sidecarStatus === "terminated" && (
        <div className="mb-2 flex items-center gap-2 rounded-md border border-red-500 dark:border-red-700 bg-red-100 dark:bg-red-900/40 px-3 py-2 text-xs text-red-800 dark:text-red-200" data-testid="topology-terminated-banner">
          <AlertTriangle size={14} className="shrink-0" />
          <span>{t("topology.sidecarTerminated")}</span>
        </div>
      )}
      {nodeGroups.length === 0 && !sidecarStatus && (
        <div className="mb-2 flex items-center gap-2 rounded-md border border-amber-300 dark:border-amber-800 bg-amber-50 dark:bg-amber-950/40 px-3 py-2 text-xs text-amber-700 dark:text-amber-300" data-testid="topology-no-nodes-banner">
          <AlertTriangle size={14} className="shrink-0" />
          <span>{t("networkLayer.noNodes")}</span>
        </div>
      )}
      <div className="flex items-center justify-between px-1 pb-2">
        <span className="text-xs text-zinc-500 dark:text-zinc-400">{t("topology.dragHint")}</span>
        {patching && (
          <span className="text-xs font-mono text-amber-500">PATCH...</span>
        )}
      </div>
      {ports.length === 0 && (
        <div className="px-1 pb-2 text-xs text-zinc-400">{t("topology.noPorts")}</div>
      )}
      {platforms.length === 0 && (
        <div className="px-1 pb-2 text-xs text-zinc-400">{t("topology.noPlatforms")}</div>
      )}
      {nodeGroups.length === 0 && (
        <div className="px-1 pb-2 text-xs text-zinc-400">{t("topology.noNodes")}</div>
      )}
      <div className={"flex-1 min-h-[400px] border border-zinc-200 dark:border-zinc-800 rounded transition-opacity duration-150 " + (ready ? "opacity-100" : "opacity-0")}>
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
          defaultEdgeOptions={{ type: "smoothstep", animated: true, style: { fontSize: 10 } }}
        >
          <Background variant={BackgroundVariant.Dots} gap={18} size={1.4} />
          <Controls />
          <MiniMap pannable zoomable />
        </ReactFlow>
      </div>
    </section>
  );
}

/// Exported shell: wraps TopologyCanvas in ReactFlowProvider so useReactFlow()
/// (P19 item 1: viewport get/set) is in scope. The App.tsx call site
/// (<TopologyView />) stays unchanged.
export function TopologyView() {
  return (
    <ReactFlowProvider>
      <TopologyCanvas />
    </ReactFlowProvider>
  );
}
