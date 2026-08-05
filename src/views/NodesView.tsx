import { useEffect, useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { Server, RefreshCw, Activity, Globe, AlertCircle, Info, ShieldCheck } from "lucide-react";
import { ipcNodeList, ipcNodePoolSnapshot, ipcIpReputationSnapshot, type ReputationSnapshot } from "../lib/ipc";

/// NodesView — Phase R3 node/IP-channel management tab.
///
/// Displays the Resin proxy node pool (the "C category" ip/ip channels) from
/// GET /api/v1/nodes + the aggregate node-pool health snapshot. Each row shows
/// display_tag, region, health (failure_count + has_outbound), and the
/// subscription it came from. This is the whitebox surface for IP channels.
///
/// The Resin sidecar owns node lifecycle (fetch from subscriptions, health
/// probe, circuit breaker). This view mirrors that state and lets the user
/// verify which exit IPs are live. Import happens in the Subscriptions tab.

interface NodeItem {
  node_hash?: string;
  display_tag?: string;
  name?: string;
  has_outbound?: boolean;
  failure_count?: number;
  region?: string | null;
  circuit_open_since?: string | null;
  tags?: { subscriptionName: string; tag: string }[];
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

export function NodesView() {
  const { t } = useTranslation();
  const [nodes, setNodes] = useState<NodeItem[]>([]);
  const [pool, setPool] = useState<PoolSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastRefresh, setLastRefresh] = useState<number>(0);
  const [reputation, setReputation] = useState<ReputationSnapshot>({ provider: null, status: "disabled", entries: [] });

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
      setError(e instanceof Error ? e.message : String(e));
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    void refresh();
    const id = setInterval(refresh, 10000);
    return () => clearInterval(id);
  }, [refresh]);

  const healthyCount = nodes.filter((n) => (n.failure_count ?? 0) === 0 && n.has_outbound !== false).length;

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
          {t("nodes.refresh")}
        </button>
      </div>

      <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg p-3 bg-white dark:bg-zinc-950 flex flex-wrap items-center gap-3">
        <ShieldCheck size={18} className="text-zinc-500" />
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium text-zinc-900 dark:text-zinc-100">{t("nodes.reputationTitle")}</div>
          {reputation.status === "ok" ? (
            <div className="text-xs text-zinc-500 dark:text-zinc-400">
              {t("nodes.reputationSummary", { count: reputation.entries.length })}
              {reputation.entries.slice(0, 4).map((entry) => ` ${entry.ip}${entry.score == null ? "" : ` · ${entry.score}`}${entry.cached ? ` · ${t("nodes.reputationCached")}` : ""}`).join(" | ")}
            </div>
          ) : (
            <div className="text-xs text-zinc-500 dark:text-zinc-400">{t(reputation.status === "not_configured" ? "nodes.reputationNotConfigured" : "nodes.reputationDisabled")}</div>
          )}
        </div>
      </div>

      {/* Aggregate stats */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <StatCard icon={<Activity size={16} />} label={t("nodes.total")} value={pool?.total_nodes ?? nodes.length} />
        <StatCard icon={<Activity size={16} />} label={t("nodes.healthy")} value={pool?.healthy_nodes ?? healthyCount} accent="green" />
        <StatCard icon={<Globe size={16} />} label={t("nodes.egressIps")} value={pool?.egress_ip_count ?? 0} />
        <StatCard icon={<Globe size={16} />} label={t("nodes.healthyEgress")} value={pool?.healthy_egress_ip_count ?? 0} accent="green" />
      </div>

      {error && (
        <div className="flex items-center gap-2 p-3 rounded-md bg-red-50 dark:bg-red-950/50 text-red-700 dark:text-red-300 text-sm">
          <AlertCircle size={16} />
          {error}
        </div>
      )}

      {/* P21-C: Egress policy guidance + protocol weight display */}
      <div className="space-y-2">
        <div className="flex items-start gap-2 p-3 rounded-md bg-blue-50/50 dark:bg-blue-950/20 border border-blue-200 dark:border-blue-900 text-xs text-blue-700 dark:text-blue-300">
          <Info size={14} className="shrink-0 mt-0.5" />
          <span>{t("nodes.egressPolicyNote")}</span>
        </div>
        <div className="p-3 rounded-md bg-zinc-50 dark:bg-zinc-900/50 border border-zinc-200 dark:border-zinc-800">
          <h3 className="text-xs font-semibold text-zinc-600 dark:text-zinc-400 mb-1">{t("nodes.protocolWeights")}</h3>
          <p className="text-xs text-zinc-500 dark:text-zinc-400">{t("nodes.protocolWeightDesc")}</p>
        </div>
      </div>

      {loading ? (
        <div className="text-sm text-zinc-400 py-8 text-center">{t("nodes.loading")}</div>
      ) : nodes.length === 0 ? (
        <div className="text-sm text-zinc-400 py-8 text-center">{t("nodes.empty")}</div>
      ) : (
        <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg overflow-hidden">
          <table className="w-full text-sm">
            <thead className="bg-zinc-50 dark:bg-zinc-900 text-zinc-500 dark:text-zinc-400 text-xs uppercase tracking-wide">
              <tr>
                <th className="text-left px-4 py-2.5 font-medium">{t("nodes.colTag")}</th>
                <th className="text-left px-4 py-2.5 font-medium">{t("nodes.colRegion")}</th>
                <th className="text-left px-4 py-2.5 font-medium">{t("nodes.colHealth")}</th>
                <th className="text-left px-4 py-2.5 font-medium">{t("nodes.colSub")}</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-zinc-100 dark:divide-zinc-800">
              {nodes.map((n, i) => {
                const healthy = (n.failure_count ?? 0) === 0 && n.has_outbound !== false;
                const sub = n.tags?.[0]?.subscriptionName ?? n.tags?.[0]?.tag ?? "";
                return (
                  <tr key={n.node_hash ?? i} className="hover:bg-zinc-50 dark:hover:bg-zinc-900/50">
                    <td className="px-4 py-2.5 text-zinc-900 dark:text-zinc-100 font-mono text-xs">
                      {n.display_tag ?? n.name ?? n.node_hash?.slice(0, 12) ?? "-"}
                    </td>
                    <td className="px-4 py-2.5 text-zinc-600 dark:text-zinc-400">
                      {n.region ?? "-"}
                    </td>
                    <td className="px-4 py-2.5">
                      <span className={healthy ? "text-green-600 dark:text-green-400" : "text-red-600 dark:text-red-400"}>
                        {healthy ? t("nodes.healthy") : t("nodes.unhealthy")} ({n.failure_count ?? 0})
                      </span>
                    </td>
                    <td className="px-4 py-2.5 text-zinc-600 dark:text-zinc-400 text-xs">{sub || "-"}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      {lastRefresh > 0 && (
        <div className="text-xs text-zinc-400 text-right">
          {t("nodes.lastRefresh", { time: new Date(lastRefresh).toLocaleTimeString() })}
        </div>
      )}
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
