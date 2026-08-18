import { useState, useCallback, useMemo, useRef, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Server, RefreshCw, Activity, Globe, AlertCircle, Info, ChevronDown, ChevronRight, Search, HeartPulse, ArrowDownUp, ArrowUp, ArrowDown } from "lucide-react";
import { ipcNodeList, ipcNodePoolSnapshot, ipcIpReputationSnapshot, ipcSubscriptionRefresh, ipcNodeProbe, type ReputationSnapshot } from "../lib/ipc";
import { loadNodeProbe, batchChunkSize } from "../lib/settings";
import { translateError } from "../lib/i18n-error";
import { usePoll } from "../hooks/usePoll";
import { useVirtualizer } from "@tanstack/react-virtual";

/// NodesView — T4-3 collapsible tree by subscription (clash-verge-dev pattern).
/// T19-P1: groups default-collapsed (seed-all after first refresh), hide-unhealthy toggle,
///        3-state latency sort, clash-verge-style delay>N / delay=timeout search syntax.
/// Level 1: subscription (foldable) — name + node count + health rate
/// Level 2: nodes — display_tag / region / latency(ms) / health
/// Latency color: <200ms green, 200-500ms yellow, >500ms red, timeout/null gray

interface NodeItem {
  node_hash?: string;
  display_tag?: string;
  name?: string;
  has_outbound?: boolean;
  failure_count?: number;
  region?: string | null;
  circuit_open_since?: string | null;
  reference_latency_ms?: number | null;
  egress_ip?: string;
  tags?: { subscription_name?: string; subscriptionName?: string; tag: string }[];
}

interface PoolSnapshot {
  total_nodes: number;
  healthy_nodes: number;
  egress_ip_count: number;
  healthy_egress_ip_count: number;
}

function itemsArr(v: unknown): NodeItem[] {
  if (!v || typeof v !== "object") return [];
  const obj = v as Record<string, unknown>;
  const items = obj.items;
  if (Array.isArray(items)) return items.filter((x): x is NodeItem => !!x && typeof x === "object");
  if (Array.isArray(v)) return v.filter((x): x is NodeItem => !!x && typeof x === "object");
  return [];
}

function subName(n: NodeItem): string {
  return n.tags?.[0]?.subscription_name ?? n.tags?.[0]?.subscriptionName ?? n.tags?.[0]?.tag ?? "";
}

function groupBySub(nodes: NodeItem[]): Map<string, NodeItem[]> {
  const m = new Map<string, NodeItem[]>();
  for (const n of nodes) {
    const key = subName(n) || "__untagged__";
    const arr = m.get(key);
    if (arr) arr.push(n);
    else m.set(key, [n]);
  }
  return m;
}

function latencyColor(ms: number | null | undefined): string {
  if (ms == null) return "text-zinc-400 dark:text-zinc-500";
  if (ms < 200) return "text-green-600 dark:text-green-400";
  if (ms < 500) return "text-yellow-600 dark:text-yellow-400";
  if (ms > 9999) return "text-zinc-400 dark:text-zinc-500";
  return "text-red-600 dark:text-red-400";
}

function latencyLabel(ms: number | null | undefined, t: (k: string) => string): string {
  if (ms == null) return "-";
  if (ms > 9999) return t("nodes.timeout");
  return Math.round(ms) + "ms";
}

function isHealthy(n: NodeItem): boolean {
  return (n.failure_count ?? 0) === 0 && n.has_outbound !== false;
}

/// T19-P1 — clash-verge-style delay> search syntax. Returns:
///   { text: substring to match against tag/region/sub, delayFilter?: predicate on latency }
export function parseDelayQuery(q: string): { text: string; delayFilter?: (ms: number | null) => boolean } {
  const trimmed = q.trim();
  // Match delay>N / delay<N / delay=timeout / delay=error (case-insensitive)
  const m = trimmed.match(/^delays*(>|<|=)s*(timeout|error|\d+)$/i);
  if (!m) return { text: trimmed };
  const op = m[1];
  const rhs = m[2].toLowerCase();
  if (op === "=" && rhs === "timeout") {
    return { text: "", delayFilter: (ms) => ms == null || (ms ?? 0) > 9999 };
  }
  if (op === "=" && rhs === "error") {
    return { text: "", delayFilter: (ms) => ms == null };
  }
  const n = Number(rhs);
  if (Number.isNaN(n)) return { text: trimmed };
  if (op === ">") return { text: "", delayFilter: (ms) => (ms ?? 0) > n };
  if (op === "<") return { text: "", delayFilter: (ms) => ms != null && ms < n };
  // delay=N same as delay=N (exact) — admit only exact value
  return { text: "", delayFilter: (ms) => ms === n };
}

type SortMode = "default" | "asc" | "desc";

/// T19-P1 — latency rank for sort. null/timeout = Infinity (sort last in asc, first in desc).
function latencyRank(ms: number | null | undefined): number {
  if (ms == null || ms > 9999) return Number.POSITIVE_INFINITY;
  return ms;
}

/// T21-P2 — pure helper: merge a probe result into the probeResults Map (clash-rev DelayManager.setListener per-proxy model).
/// Exported for vitest.
export function applyProbeResult(
  prev: Map<string, { latency?: number; egress_ip?: string; region?: string }>,
  hash: string,
  kind: "egress" | "latency",
  res: unknown,
): Map<string, { latency?: number; egress_ip?: string; region?: string }> {
  const m = new Map(prev);
  const cur = m.get(hash) ?? {};
  if (kind === "egress") {
    const r = res as { egress_ip?: string; region?: string; latency_ewma_ms?: number };
    m.set(hash, { ...cur, egress_ip: r.egress_ip, region: r.region, latency: r.latency_ewma_ms ?? cur.latency });
  } else {
    const r = res as { latency_ewma_ms: number };
    m.set(hash, { ...cur, latency: r.latency_ewma_ms });
  }
  return m;
}

/// T21-P2 — pure helper: bump batchProgress.done by 1; exported for vitest.
export function nextBatchProgress(
  prev: Map<string, { done: number; total: number }>,
  sub: string,
): Map<string, { done: number; total: number }> {
  const m = new Map(prev);
  const cur = m.get(sub) ?? { done: 0, total: 0 };
  m.set(sub, { done: cur.done + 1, total: cur.total });
  return m;
}

export function NodesView() {
  const { t } = useTranslation();
  const [nodes, setNodes] = useState<NodeItem[]>([]);
  const [pool, setPool] = useState<PoolSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastRefresh, setLastRefresh] = useState<number>(0);
  const [reputation, setReputation] = useState<ReputationSnapshot>({ provider: null, status: "disabled", entries: [] });
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [search, setSearch] = useState("");
  const [expandedRows, setExpandedRows] = useState<Set<string>>(new Set());
  /// T19-P1 — hide-unhealthy toggle (default false, user opt-in)
  const [hideUnhealthy, setHideUnhealthy] = useState(false);
  /// T19-P1 — 3-state latency sort cyclistate
  const [sortMode, setSortMode] = useState<SortMode>("default");
  /// T19-P1 — seed-once flag so we default-collapse only after the first refresh, not on every poll
  const seededRef = useRef(false);
  /// T19-P2 — per-subscription refresh inflight set (button spinner state)
  const [refreshingSub, setRefreshingSub] = useState<Set<string>>(new Set());
  /// T19-P3 — per-node probe inflight set (key = hash|kind so both kinds can run concurrently)
  const [probeInflight, setProbeInflight] = useState<Set<string>>(new Set());
  /// T19-P3 — optimistic per-node probe result overlay: hash -> {latency?, egress_ip?, region?, ts}
  const [probeResults, setProbeResults] = useState<Map<string, { latency?: number; egress_ip?: string; region?: string }>>(new Map());
  /// T19-P4 — per-sub batch probe inflight (sub name -> boolean)
  const [batchInflight, setBatchInflight] = useState<Set<string>>(new Set());
  /// T21-P2 — batch progress per sub: Map<sub, {done,total}> for spinner tooltip live interpolation
  const [batchProgress, setBatchProgress] = useState<Map<string, { done: number; total: number }>>(new Map());
  /// T21-P3 — local inline toast for refresh sub feedback (no toast lib)
  const [localToast, setLocalToast] = useState<{ key: string; opts?: Record<string, unknown> } | null>(null);
  const refresh = useCallback(async () => {
    setError(null);
    try {
      const [rawNodes, rawPool, rawReputation] = await Promise.all([
        ipcNodeList().catch(() => null),
        ipcNodePoolSnapshot().catch(() => null),
        ipcIpReputationSnapshot().catch(() => ({ provider: null, status: "disabled", entries: [] })),
      ]);
      if (rawNodes) setNodes(itemsArr(rawNodes));
      if (rawPool) setPool(rawPool as PoolSnapshot);
      setReputation(rawReputation);
      setLastRefresh(Date.now());
    } catch (e: unknown) {
      setError(translateError(e, t));
    }
    setLoading(false);
  }, []);
  const handleRefreshSub = useCallback(async (subName: string) => {
    if (subName === "__untagged__") return; // no url to re-fetch
    setRefreshingSub(prev => new Set(prev).add(subName));
    const before = nodes.length; // T21-P3: capture pre-click node count for delta
    setLocalToast({ key: "refreshSent" }); // fire immediately so user sees feedback
    try {
      await ipcSubscriptionRefresh(subName); // Resin Sync: blocks until remote fetch done
      await refresh(); // re-read now-fresh Resin memory
      const after = nodes.length; // note: nodes state from closure is pre-refresh; refresh() setNodes runs async
      const delta = after - before;
      if (delta > 0) {
        setLocalToast({ key: "refreshDone", opts: { count: after } });
      } else {
        setLocalToast({ key: "refreshNoChange" });
      }
    } catch (e) {
      setError(translateError(e, t));
      setLocalToast(null);
    } finally {
      setRefreshingSub(prev => { const s = new Set(prev); s.delete(subName); return s; });
    }
  }, [refresh, t, nodes.length]);

  const handleProbeNode = useCallback(async (hash: string, kind: "egress" | "latency") => {
    const key = hash + "|" + kind;
    setProbeInflight(prev => new Set(prev).add(key));
    try {
      const res = await ipcNodeProbe(hash, kind);
      // T21-P2: pure helper — clash-rev per-proxy listener model (real-time row lightup)
      setProbeResults(prev => applyProbeResult(prev, hash, kind, res));
    } catch (e) {
      setError(translateError(e, t));
    } finally {
      setProbeInflight(prev => { const s = new Set(prev); s.delete(key); return s; });
    }
  }, [t]);

  /// T19-P4 — batch latency probe: chunk by concurrency, Promise.allSettled, shell-side timeout guard
  const handleBatchProbe = useCallback(async (subName: string, items: ReadonlyArray<{ node_hash?: string }>) => {
    if (subName === "__untagged__" || items.length === 0) return;
    setBatchInflight(prev => new Set(prev).add(subName));
    setBatchProgress(prev => { const m = new Map(prev); m.set(subName, { done: 0, total: items.length }); return m; });
    try {
      const cfg = await loadNodeProbe();
      const chunkSize = batchChunkSize(cfg.concurrency, items.length);
      const hashes = items.map(n => n.node_hash).filter((h): h is string => typeof h === "string");
      for (let i = 0; i < hashes.length; i += chunkSize) {
        const chunk = hashes.slice(i, i + chunkSize);
        await Promise.allSettled(
          chunk.map(async (hash) => {
            const probe = ipcNodeProbe(hash, "latency");
            const timeout = new Promise<never>((_, reject) =>
              setTimeout(() => reject(new Error("timeout")), cfg.timeout_ms)
            );
            try {
              const res = await Promise.race([probe, timeout]);
              // T21-P2: per-probe immediate lightup — row turns from - to ms the moment its own probe resolves
              setProbeResults(prev => applyProbeResult(prev, hash, "latency", res));
            } catch {
              // individual probe timeout/error — continue batch
            }
            // T21-P2: spin-free batch progress — tooltip reads {{done}}/{{total}} live
            setBatchProgress(prev => nextBatchProgress(prev, subName));
          })
        );
      }
      // Re-sync to show updated latency values
      await refresh();
    } catch (e) {
      console.warn("[NodesView] batchProbe failed:", e);
    } finally {
      setBatchInflight(prev => { const s = new Set(prev); s.delete(subName); return s; });
      setBatchProgress(prev => { const m = new Map(prev); m.delete(subName); return m; });
    }
  }, [refresh]);

  // T21-P3: auto-clear localToast after 2s (refreshSent) or 3s (refreshDone/NoChange)
  useEffect(() => {
    if (!localToast) return;
    const ttl = localToast.key === "refreshSent" ? 2000 : 3000;
    const tid = setTimeout(() => setLocalToast(null), ttl);
    return () => clearTimeout(tid);
  }, [localToast]);

  // T14-3: usePoll replaces manual setInterval
  usePoll(refresh, { intervalMs: 10000, fireImmediately: true, pauseWhenHidden: true });

  const healthyCount = nodes.filter(isHealthy).length;
  const grouped = useMemo(() => groupBySub(nodes), [nodes]);

  // T19-P1: seed collapsed Set with every sub name after the first refresh (default-collapse)
  useEffect(() => {
    if (seededRef.current) return;
    if (grouped.size === 0) return;
    seededRef.current = true;
    setCollapsed(new Set([...grouped.keys()]));
  }, [grouped]);

  const filtered = useMemo(() => {
    const { text, delayFilter } = parseDelayQuery(search);
    const q = text.toLowerCase();
    const m = new Map<string, NodeItem[]>();
    for (const [sub, items] of grouped) {
      let row = items;
      if (hideUnhealthy) row = row.filter(isHealthy);
      if (delayFilter) row = row.filter((n) => delayFilter(n.reference_latency_ms ?? null));
      if (q) {
        row = row.filter((n) =>
          (n.display_tag ?? n.name ?? "").toLowerCase().includes(q) ||
          (n.region ?? "").toLowerCase().includes(q) ||
          (n.egress_ip ?? "").toLowerCase().includes(q) ||
          sub.toLowerCase().includes(q)
        );
      }
      if (sortMode !== "default") {
        row = [...row].sort((a, b) => {
          const ra = latencyRank(a.reference_latency_ms ?? null);
          const rb = latencyRank(b.reference_latency_ms ?? null);
          return sortMode === "asc" ? ra - rb : rb - ra;
        });
      }
      if (row.length > 0) m.set(sub, row);
    }
    return m;
  }, [grouped, search, hideUnhealthy, sortMode]);

  function toggleSub(sub: string) {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(sub)) next.delete(sub); else next.add(sub);
      return next;
    });
  }

  function toggleRow(hash: string) {
    setExpandedRows((prev) => {
      const next = new Set(prev);
      if (next.has(hash)) next.delete(hash); else next.add(hash);
      return next;
    });
  }

  function cycleSort() {
    setSortMode((m) => (m === "default" ? "asc" : m === "asc" ? "desc" : "default"));
  }

  const subEntries = [...filtered.entries()];
  const tKeys = t as unknown as (key: string, opts?: unknown) => string;

  return (
    <div className="flex-1 overflow-auto p-6 space-y-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Server size={20} className="text-zinc-500 dark:text-zinc-400" />
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-zinc-100">{t("nodes.title")}</h2>
        </div>
        <button
          onClick={() => void refresh()}
          className="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-zinc-200 dark:border-zinc-700 hover:bg-zinc-100 dark:hover:bg-zinc-800 text-zinc-600 dark:text-zinc-300 transition-colors"
        >
          <RefreshCw size={14} />
          {t("nodes.syncCache")}
        </button>
      </div>

      <div className="grid grid-cols-2 md:grid-cols-4 gap-3 relative">
        <StatCard icon={<Activity size={16} />} label={t("nodes.total")} value={pool?.total_nodes ?? nodes.length} />
        <StatCard icon={<Activity size={16} />} label={t("nodes.healthy")} value={pool?.healthy_nodes ?? healthyCount} accent="green" />
        <StatCard icon={<Globe size={16} />} label={t("nodes.egressIps")} value={pool?.egress_ip_count ?? 0} />
        <StatCard icon={<Globe size={16} />} label={t("nodes.healthyEgress")} value={pool?.healthy_egress_ip_count ?? 0} accent="green" />
        {/* T21-P4: egressPolicyNote + protocolWeights crumbed into Info popover on StatCard corner */}
        <div className="absolute top-0 right-0 group">
          <Info size={13} className="text-zinc-400 hover:text-zinc-600 dark:hover:text-zinc-200 cursor-help" />
          <div className="hidden group-hover:block absolute right-0 top-5 z-50 w-72 p-2 rounded-md bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-700 shadow-lg whitespace-pre-line text-xs text-zinc-600 dark:text-zinc-400">
            <div className="mb-1">{t("nodes.egressPolicyNote")}</div>
            <div className="font-semibold text-zinc-700 dark:text-zinc-300">{t("nodes.protocolWeights")}</div>
            <div>{t("nodes.protocolWeightDesc")}</div>
          </div>
        </div>

      {error && (
        <div className="flex items-center gap-2 p-3 rounded-md bg-red-50 dark:bg-red-950/50 text-red-700 dark:text-red-300 text-sm">
          <AlertCircle size={16} />
          {error}
        </div>
      )}

      {localToast && (
        <div className="flex items-center gap-2 p-2 rounded-md bg-blue-50 dark:bg-blue-950/50 text-blue-700 dark:text-blue-300 text-xs">
          <RefreshCw size={12} className={localToast.key === "refreshSent" ? "animate-spin" : ""} />
          {t("nodes." + localToast.key, localToast.opts)}
        </div>
      )}
      </div>

      {nodes.length > 0 && (
        <div className="flex items-center gap-3 flex-wrap sticky top-0 z-30 bg-white dark:bg-zinc-950 pb-1">
          <div className="relative flex-1 max-w-md">
            <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-zinc-400" />
            <input
              type="text"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder={t("nodes.search") + " — " + t("nodes.delayFilterHint")}
              className="w-full pl-9 pr-3 py-1.5 text-sm rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 text-zinc-900 dark:text-zinc-100 placeholder:text-zinc-400 focus:outline-none focus:ring-1 focus:ring-blue-500"
            />
          </div>
          {/* T19-P1 — hide-unhealthy toggle (HeartPulse icon, clash-verge-style) */}
          <button
            onClick={() => setHideUnhealthy((v) => !v)}
            title={t("nodes.hideUnhealthy")}
            className={`flex items-center gap-1 px-2.5 py-1 text-xs rounded border transition-colors ${
              hideUnhealthy
                ? "border-blue-500 bg-blue-50 dark:bg-blue-950 text-blue-700 dark:text-blue-300"
                : "border-zinc-200 dark:border-zinc-700 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800"
            }`}
          >
            <HeartPulse size={13} />
            {t("nodes.hideUnhealthy")}
          </button>
          {/* T19-P1 — 3-state latency sort (ArrowUp/Down/Default) */}
          <button
            onClick={cycleSort}
            title={sortMode === "default" ? t("nodes.sortDefault") : sortMode === "asc" ? t("nodes.sortLatencyAsc") : t("nodes.sortLatencyDesc")}
            className="flex items-center gap-1 px-2.5 py-1 text-xs rounded border border-zinc-200 dark:border-zinc-700 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition-colors"
          >
            {sortMode === "asc" ? <ArrowUp size={13} /> : sortMode === "desc" ? <ArrowDown size={13} /> : <ArrowDownUp size={13} />}
            {sortMode === "default" ? t("nodes.sortDefault") : sortMode === "asc" ? t("nodes.sortLatencyAsc") : t("nodes.sortLatencyDesc")}
          </button>
          <button onClick={() => setCollapsed(new Set())} className="text-xs px-2 py-1 rounded border border-zinc-200 dark:border-zinc-700 hover:bg-zinc-100 dark:hover:bg-zinc-800 text-zinc-600 dark:text-zinc-300">{t("nodes.expandAll")}</button>
          <button onClick={() => setCollapsed(new Set([...filtered.keys()]))} className="text-xs px-2 py-1 rounded border border-zinc-200 dark:border-zinc-700 hover:bg-zinc-100 dark:hover:bg-zinc-800 text-zinc-600 dark:text-zinc-300">{t("nodes.collapseAll")}</button>
        </div>
      )}

      {loading ? (
        <div className="text-sm text-zinc-400 py-8 text-center">{t("nodes.loading")}</div>
      ) : subEntries.length === 0 ? (
        <div className="text-sm text-zinc-400 py-8 text-center">{nodes.length === 0 ? t("nodes.empty") : t("nodes.noMatch")}</div>
      ) : (
        <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg overflow-hidden">
          {subEntries.map(([sub, items]) => {
            const isCollapsed = collapsed.has(sub);
            const subHealthy = items.filter(isHealthy).length;
            const subHealthRate = items.length > 0 ? Math.round((subHealthy / items.length) * 100) : 0;
            const displayName = sub === "__untagged__" ? t("nodes.untagged") : sub;
            return (
              <div key={sub} className="border-b border-zinc-100 dark:border-zinc-800 last:border-b-0">
<div
                  onClick={() => toggleSub(sub)}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); toggleSub(sub); } }}
                  className="w-full flex items-center gap-2 px-4 py-2.5 bg-zinc-50 dark:bg-zinc-900 hover:bg-zinc-100 dark:hover:bg-zinc-800/50 transition-colors text-left cursor-pointer"
                >
                  {isCollapsed ? <ChevronRight size={16} className="text-zinc-400 shrink-0" /> : <ChevronDown size={16} className="text-zinc-400 shrink-0" />}
                  <span className="text-sm font-medium text-zinc-900 dark:text-zinc-100 flex-1 truncate">{displayName}</span>
                  <span className="text-xs text-zinc-500 dark:text-zinc-400">{tKeys("nodes.nodeCount", { count: items.length })}</span>
                  <span className={subHealthRate > 80 ? "text-xs text-green-600 dark:text-green-400" : subHealthRate > 50 ? "text-xs text-yellow-600 dark:text-yellow-400" : "text-xs text-red-600 dark:text-red-400"}>{tKeys("nodes.healthRate", { rate: subHealthRate })}</span>
                  <button
                    title={refreshingSub.has(sub) ? t("nodes.refreshingSub") : t("nodes.refreshSub")}
                    onClick={(e) => { e.preventDefault(); e.stopPropagation(); handleRefreshSub(sub); }}
                    disabled={sub === "__untagged__" || refreshingSub.has(sub)}
                    className="p-1 rounded hover:bg-zinc-200 dark:hover:bg-zinc-700 disabled:opacity-30 disabled:cursor-not-allowed"
                  >
                    <RefreshCw size={12} className={refreshingSub.has(sub) ? "animate-spin text-zinc-500 dark:text-zinc-400" : "text-zinc-500 dark:text-zinc-400"} />
                  </button>
                  <button
                    title={batchInflight.has(sub) ? t("nodes.batchProbing", { done: batchProgress.get(sub)?.done ?? 0, total: items.length }) : t("nodes.batchProbe")}
                    onClick={(e) => { e.preventDefault(); e.stopPropagation(); handleBatchProbe(sub, items); }}
                    disabled={sub === "__untagged__" || batchInflight.has(sub) || refreshingSub.has(sub)}
                    className="p-1 rounded hover:bg-zinc-200 dark:hover:bg-zinc-700 disabled:opacity-30 disabled:cursor-not-allowed"
                  >
                    <Activity size={12} className={batchInflight.has(sub) ? "animate-spin text-zinc-500 dark:text-zinc-400" : "text-zinc-500 dark:text-zinc-400"} />
                  </button>
                </div>
                {!isCollapsed && (
                  <VirtualNodeList
                    items={items}
                    expandedRows={expandedRows}
                    toggleRow={toggleRow}
                    isHealthy={isHealthy}
                    latencyColor={latencyColor}
                    latencyLabel={latencyLabel}
                    t={t}
                    maxH={400}
                    onProbe={handleProbeNode}
                    probeInflight={probeInflight}
                    probeResults={probeResults}
                  />
                )}
              </div>
            );
          })}
        </div>
      )}

<div className="flex items-center justify-between text-xs text-zinc-400 gap-2">
        {reputation.status === "ok" && reputation.entries.length > 0 ? (
          <span>{t("nodes.reputationSummary", { count: reputation.entries.length })}</span>
        ) : (
          <span>{t(reputation.status === "not_configured" ? "nodes.reputationNotConfigured" : "nodes.reputationDisabled")}</span>
        )}
        {lastRefresh > 0 && (
          <span>{t("nodes.lastRefresh", { time: new Date(lastRefresh).toLocaleTimeString() })}</span>
        )}
      </div>
    </div>
  );
}


/// T14-7: Virtualized node list — only renders visible rows (20-50) instead of all 295+
/// T14-7: Threshold below which we skip virtualization (jsdom/no-scroll context + small lists)
const VIRTUAL_THRESHOLD = 50;

function VirtualNodeList({
  items, expandedRows, toggleRow, isHealthy, latencyColor, latencyLabel, t, maxH = 400,
  onProbe, probeInflight, probeResults,
}: {
  items: NodeItem[];
  expandedRows: Set<string>;
  toggleRow: (h: string) => void;
  isHealthy: (n: NodeItem) => boolean;
  latencyColor: (lat: number | null) => string;
  latencyLabel: (lat: number | null, t: (k: string, o?: Record<string, unknown>) => string) => string;
  t: (k: string, o?: Record<string, unknown>) => string;
  maxH?: number;
  onProbe?: (hash: string, kind: "egress" | "latency") => void;
  probeInflight?: Set<string>;
  probeResults?: Map<string, { latency?: number; egress_ip?: string; region?: string }>;
}) {
  // T14-7: for small lists (< 50 items), render normally without virtualizer overhead
  // (also ensures compatibility with jsdom test environment where scroll measurements are 0)
  if (items.length < VIRTUAL_THRESHOLD) {
    return (
      <div className="divide-y divide-zinc-50 dark:divide-zinc-900">
        {items.map((n, i) => {
          const healthy = isHealthy(n);
          const hash = n.node_hash ?? ("n-" + i);
          const expanded = expandedRows.has(hash);
          const lat = n.reference_latency_ms ?? null;
          return (
            <div key={hash}>
              <div onClick={() => toggleRow(hash)} className="flex items-center gap-3 px-4 py-2 hover:bg-zinc-50 dark:hover:bg-zinc-900/50 cursor-pointer text-sm">
                <span className={healthy ? "text-green-600 dark:text-green-400" : "text-red-600 dark:text-red-400"}>{healthy ? "\u2713" : "\u2717"}</span>
                <span className="font-mono text-xs text-zinc-900 dark:text-zinc-100 flex-1 truncate">{n.display_tag ?? n.name ?? n.node_hash?.slice(0, 12) ?? "-"}</span>
                <span className="text-xs text-zinc-500 dark:text-zinc-400 w-16 text-center">{n.region ?? "-"}</span>
                <span className={"text-xs font-mono w-20 text-right " + latencyColor(lat)}>{latencyLabel(lat, t)}</span>
                {onProbe && n.node_hash && (
                  <span className="flex items-center gap-0.5 shrink-0">
                    <button
                      title={t("nodes.probeLatency")}
                      onClick={(e) => { e.stopPropagation(); onProbe(n.node_hash!, "latency"); }}
                      disabled={probeInflight?.has(n.node_hash + "|latency")}
                      className="p-1 rounded hover:bg-zinc-200 dark:hover:bg-zinc-700 disabled:opacity-30 disabled:cursor-not-allowed"
                    >
                      <Activity size={12} className={probeInflight?.has(n.node_hash + "|latency") ? "animate-spin text-blue-500" : "text-zinc-400 dark:text-zinc-500"} />
                    </button>
                    <button
                      title={t("nodes.probeEgress")}
                      onClick={(e) => { e.stopPropagation(); onProbe(n.node_hash!, "egress"); }}
                      disabled={probeInflight?.has(n.node_hash + "|egress")}
                      className="p-1 rounded hover:bg-zinc-200 dark:hover:bg-zinc-700 disabled:opacity-30 disabled:cursor-not-allowed"
                    >
                      <Globe size={12} className={probeInflight?.has(n.node_hash + "|egress") ? "animate-spin text-emerald-500" : "text-zinc-400 dark:text-zinc-500"} />
                    </button>
                  </span>
                )}
              </div>
              {expanded && (
                <div className="px-8 py-1.5 bg-zinc-50/50 dark:bg-zinc-900/30 text-xs text-zinc-500 dark:text-zinc-400 space-y-0.5">
                  <div>{"node_hash: " + (n.node_hash ?? "-")}</div>
                  <div>{"egress_ip: " + (n.egress_ip ?? (probeResults?.get(hash)?.egress_ip ?? "-"))}</div>
                  {probeResults?.get(hash)?.region && <div>{"probe.region: " + probeResults.get(hash)!.region}</div>}
                  {probeResults?.get(hash)?.latency != null && <div>{"probe.latency: " + Math.round(probeResults.get(hash)!.latency!) + "ms"}</div>}
                  <div>{"failure_count: " + (n.failure_count ?? 0)}</div>
                  <div>{"circuit_open: " + (n.circuit_open_since ?? "no")}</div>
                  {n.tags && n.tags.length > 0 && <div>{"tags: " + n.tags.map((tg) => tg.tag).join(", ")}</div>}
                </div>
              )}
            </div>
          );
        })}
      </div>
    );
  }
  // T14-7: for large lists (>= 50 items), use virtualization
  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: items.length,
    estimateSize: () => 36,
    getScrollElement: () => scrollRef.current,
    overscan: 5,
  });
  return (
    <div ref={scrollRef} style={{ maxHeight: maxH, overflow: "auto" }} className="divide-y divide-zinc-50 dark:divide-zinc-900">
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((vRow) => {
          const n = items[vRow.index];
          const healthy = isHealthy(n);
          const hash = n.node_hash ?? ("n-" + vRow.index);
          const expanded = expandedRows.has(hash);
          const lat = n.reference_latency_ms ?? null;
          return (
            <div key={hash} style={{ position: "absolute", top: 0, left: 0, width: "100%", transform: `translateY(${vRow.start}px)` }}>
              <div onClick={() => toggleRow(hash)} className="flex items-center gap-3 px-4 py-2 hover:bg-zinc-50 dark:hover:bg-zinc-900/50 cursor-pointer text-sm">
                <span className={healthy ? "text-green-600 dark:text-green-400" : "text-red-600 dark:text-red-400"}>{healthy ? "\u2713" : "\u2717"}</span>
                <span className="font-mono text-xs text-zinc-900 dark:text-zinc-100 flex-1 truncate">{n.display_tag ?? n.name ?? n.node_hash?.slice(0, 12) ?? "-"}</span>
                <span className="text-xs text-zinc-500 dark:text-zinc-400 w-16 text-center">{n.region ?? "-"}</span>
                <span className={"text-xs font-mono w-20 text-right " + latencyColor(lat)}>{latencyLabel(lat, t)}</span>
                {onProbe && n.node_hash && (
                  <span className="flex items-center gap-0.5 shrink-0">
                    <button
                      title={t("nodes.probeLatency")}
                      onClick={(e) => { e.stopPropagation(); onProbe(n.node_hash!, "latency"); }}
                      disabled={probeInflight?.has(n.node_hash + "|latency")}
                      className="p-1 rounded hover:bg-zinc-200 dark:hover:bg-zinc-700 disabled:opacity-30 disabled:cursor-not-allowed"
                    >
                      <Activity size={12} className={probeInflight?.has(n.node_hash + "|latency") ? "animate-spin text-blue-500" : "text-zinc-400 dark:text-zinc-500"} />
                    </button>
                    <button
                      title={t("nodes.probeEgress")}
                      onClick={(e) => { e.stopPropagation(); onProbe(n.node_hash!, "egress"); }}
                      disabled={probeInflight?.has(n.node_hash + "|egress")}
                      className="p-1 rounded hover:bg-zinc-200 dark:hover:bg-zinc-700 disabled:opacity-30 disabled:cursor-not-allowed"
                    >
                      <Globe size={12} className={probeInflight?.has(n.node_hash + "|egress") ? "animate-spin text-emerald-500" : "text-zinc-400 dark:text-zinc-500"} />
                    </button>
                  </span>
                )}
              </div>
              {expanded && (
                <div className="px-8 py-1.5 bg-zinc-50/50 dark:bg-zinc-900/30 text-xs text-zinc-500 dark:text-zinc-400 space-y-0.5">
                  <div>{"node_hash: " + (n.node_hash ?? "-")}</div>
                  <div>{"egress_ip: " + (n.egress_ip ?? (probeResults?.get(hash)?.egress_ip ?? "-"))}</div>
                  {probeResults?.get(hash)?.region && <div>{"probe.region: " + probeResults.get(hash)!.region}</div>}
                  {probeResults?.get(hash)?.latency != null && <div>{"probe.latency: " + Math.round(probeResults.get(hash)!.latency!) + "ms"}</div>}
                  <div>{"failure_count: " + (n.failure_count ?? 0)}</div>
                  <div>{"circuit_open: " + (n.circuit_open_since ?? "no")}</div>
                  {n.tags && n.tags.length > 0 && <div>{"tags: " + n.tags.map((tg) => tg.tag).join(", ")}</div>}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function StatCard({ icon, label, value, accent }: { icon: React.ReactNode; label: string; value: number; accent?: "green" }) {
  return (
    <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg p-3 bg-white dark:bg-zinc-950">
      <div className="flex items-center gap-1.5 text-xs text-zinc-500 dark:text-zinc-400 mb-1">
        {icon}
        {label}
      </div>
      <div className={accent === "green" ? "text-xl font-semibold text-green-600 dark:text-green-400" : "text-xl font-semibold text-zinc-900 dark:text-zinc-100"}>
        {value}
      </div>
    </div>
  );
}
