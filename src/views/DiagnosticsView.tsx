import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Activity, Stethoscope, Flame, Globe, Server, Loader2, FolderOpen, ArrowRight, Zap } from "lucide-react"; // T8-2/T8-3 added Zap for verify button
import { openPath } from "@tauri-apps/plugin-opener";
import { invoke } from "@tauri-apps/api/core";
import {
  ipcGetSidecarStatus,
  ipcCheckFirewallStatus,
  ipcRequestLogTail,
  ipcProbeExitIp,
  ipcPortHealthCheck,
  ipcPlatformListFull,
  ipcStrategyVerify,
  ipcCloseAllConnections,
  ipcResetKernel,
  type SidecarStatus,
  type FirewallStatus,
  type RequestLogEntry,
  type ExitIpProbe,
  type PortHealthCheck,
} from "../lib/ipc";

const btnCls = "px-2.5 py-1.5 rounded text-xs font-medium transition-colors bg-zinc-100 hover:bg-zinc-200 dark:bg-zinc-800 dark:hover:bg-zinc-700 text-zinc-700 dark:text-zinc-300 disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-1.5";

function DiagCard({ icon, title, children }: { icon: React.ReactNode; title: string; children: React.ReactNode }) {
  return (
    <section className="rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-4 space-y-2">
      <h3 className="text-sm font-semibold text-zinc-800 dark:text-zinc-200 flex items-center gap-2">{icon}{title}</h3>
      {children}
    </section>
  );
}

function DiagField({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-xs text-zinc-500 dark:text-zinc-400">{label}</span>
      {children}
    </div>
  );
}

export function DiagnosticsView() {
  const { t } = useTranslation();
  const [sidecarStatus, setSidecarStatus] = useState<SidecarStatus | null>(null);
  const [firewallStatus, setFirewallStatus] = useState<FirewallStatus | null>(null);
  const [reqLogs, setReqLogs] = useState<RequestLogEntry[]>([]);
  const [sidecarLogs, setSidecarLogs] = useState<string[]>([]);
  const [diagBusy, setDiagBusy] = useState(false);
  const [pollInterval, setPollInterval] = useState(5000);
  const [probePort, setProbePort] = useState("1790");
  const [probeProto, setProbeProto] = useState("http");
  const [probeResult, setProbeResult] = useState<ExitIpProbe | null>(null);
  const [probeBusy, setProbeBusy] = useState(false);
  // T8-2: Strategy verification state
  const [platformNames, setPlatformNames] = useState<string[]>([]);
  const [selectedPlatform, setSelectedPlatform] = useState("");
  const [sampleCount, setSampleCount] = useState(10);
  const [verifyResult, setVerifyResult] = useState<Record<string, unknown> | null>(null);
  const [verifyBusy, setVerifyBusy] = useState(false);
  const [healthPort, setHealthPort] = useState("1790");
  const [healthProto, setHealthProto] = useState("socks5");
  const [healthResult, setHealthResult] = useState<PortHealthCheck | null>(null);
  const [healthBusy, setHealthBusy] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Load poll interval from settings
  useEffect(() => {
    (async () => {
      try {
        const stored = await invoke<number>("get_store_value", { key: "diagPollInterval", store: "settings.json" });
        if (typeof stored === "number" && stored >= 1000 && stored <= 60000) setPollInterval(stored);
      } catch { /* default 5000 */ }
    })();
  }, []);

  // Save poll interval when changed
  const handlePollChange = async (ms: number) => {
    const clamped = Math.max(1000, Math.min(60000, ms));
    setPollInterval(clamped);
    try { await invoke("set_store_value", { key: "diagPollInterval", value: clamped, store: "settings.json" }); } catch { /* vitest no-op */ }
  };

  // Main refresh: fetch all diagnostic data
  const refreshDiagnostics = async () => {
    setDiagBusy(true);
    try {
      const [status, fw, logs, sideLogs] = await Promise.all([
        ipcGetSidecarStatus().catch(() => null),
        ipcCheckFirewallStatus().catch(() => null),
        ipcRequestLogTail(50).catch(() => []),
        invoke<string[]>("get_sidecar_logs").catch(() => []),
      ]);
      if (status) setSidecarStatus(status);
      if (fw) setFirewallStatus(fw);
      setReqLogs(Array.isArray(logs) ? logs : []);
      setSidecarLogs(Array.isArray(sideLogs) ? sideLogs : []);
    } finally {
      setDiagBusy(false);
    }
  };

  // Auto-poll with visibilitychange pause
  useEffect(() => {
    refreshDiagnostics();
    let cancelled = false;

    const startPolling = () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
      intervalRef.current = setInterval(() => {
        if (!cancelled && !document.hidden) refreshDiagnostics();
      }, pollInterval);
    };
    startPolling();

    const onVisibility = () => { /* visibilitychange just gates the interval */ };
    document.addEventListener("visibilitychange", onVisibility);

    return () => {
      cancelled = true;
      if (intervalRef.current) clearInterval(intervalRef.current);
      document.removeEventListener("visibilitychange", onVisibility);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pollInterval]);

  // Exit IP probe
  const handleProbe = async () => {
    const port = parseInt(probePort, 10);
    if (!port || port < 1024 || port > 65535) return;
    setProbeBusy(true);
    try {
      const r = await ipcProbeExitIp(port, probeProto);
      setProbeResult(r);
    } catch (e) {
      setProbeResult({ port, protocol: probeProto, exit_ip: "", latency_ms: 0, status: 0 } as ExitIpProbe);
    } finally {
      setProbeBusy(false);
    }
  };

  // Port health check
  const handleHealth = async () => {
    const port = parseInt(healthPort, 10);
    if (!port || port < 1024 || port > 65535) return;
    setHealthBusy(true);
    try {
      const r = await ipcPortHealthCheck(port, healthProto);
      setHealthResult(r);
    } catch (e) {
      setHealthResult({ port, reachable: false, socks5_ok: false, protocol_mismatch: false, latency_ms: 0, reason: "refused" } as PortHealthCheck);
    } finally {
      setHealthBusy(false);
    }
  };

  // Open log directory
  const openLogDir = async () => {
    try {
      const dir = await invoke<string>("get_log_dir");
      if (dir) await openPath(dir);
    } catch { /* not in tauri */ }
  };

  // T8-2: Load platform names for strategy verify dropdown
  useEffect(() => {
    (async () => {
      try {
        const res = await ipcPlatformListFull();
        const items = (res as { items?: { name?: string }[] })?.items ?? [];
        const names = items.map((it) => it.name ?? "").filter(Boolean) as string[];
        setPlatformNames(names);
      } catch { /* not in tauri */ }
    })();
  }, []);

  // T8-2: Run strategy verification
  const runStrategyVerify = async () => {
    if (!selectedPlatform) return;
    setVerifyBusy(true);
    setVerifyResult(null);
    try {
      const res = await ipcStrategyVerify(selectedPlatform, sampleCount);
      setVerifyResult(res as Record<string, unknown>);
    } catch (e) {
      setVerifyResult({ error: e instanceof Error ? e.message : String(e) });
    } finally {
      setVerifyBusy(false);
    }
  };

  // T8-6: Close all connections (kill sidecar — drops all active TCP)
  const handleCloseAll = async () => {
    if (!window.confirm(t("connectionControl.confirmClose"))) return;
    try {
      await ipcCloseAllConnections();
    } catch { /* not in tauri */ }
  };

  // T8-6: Reset kernel (kill + restart sidecar)
  const handleResetKernel = async () => {
    if (!window.confirm(t("connectionControl.confirmReset"))) return;
    try {
      await ipcResetKernel();
    } catch { /* not in tauri */ }
  };

  return (
    <div className="w-full max-w-none px-6 space-y-4" data-testid="diag-view">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold text-zinc-800 dark:text-zinc-200 flex items-center gap-2">
          <Stethoscope size={20} strokeWidth={1.75} />
          {t("diagnostics.title")}
        </h2>
        <div className="flex items-center gap-2">
          <label className="text-xs text-zinc-500 dark:text-zinc-400">{t("diagnostics.pollInterval")}</label>
          <select
            data-testid="diag-poll-select"
            value={pollInterval}
            onChange={(e) => handlePollChange(Number(e.target.value))}
            className="text-xs rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 px-2 py-1"
          >
            <option value={1000}>1s</option>
            <option value={2000}>2s</option>
            <option value={5000}>5s</option>
            <option value={10000}>10s</option>
            <option value={30000}>30s</option>
            <option value={60000}>60s</option>
          </select>
          <button data-testid="diag-refresh-btn" onClick={() => void refreshDiagnostics()} disabled={diagBusy} className={btnCls}>
            {diagBusy ? <Loader2 size={14} className="animate-spin" /> : <Activity size={14} strokeWidth={1.75} />}
            {t("diagnostics.refresh")}
          </button>
        </div>
      </div>

      {/* Sidecar status card */}
      <DiagCard icon={<Server size={16} strokeWidth={1.75} />} title={t("diagnostics.sidecarStatus")}>
        <div className="grid grid-cols-2 md:grid-cols-3 gap-2">
          <DiagField label="Port">
            <p className="text-sm text-zinc-700 dark:text-zinc-300" data-testid="diag-sidecar-port">
              {sidecarStatus ? sidecarStatus.api_port : t("diagnostics.refresh")}
            </p>
          </DiagField>
          <DiagField label="Mode">
            <p className="text-sm text-zinc-700 dark:text-zinc-300" data-testid="diag-sidecar-mode">
              {sidecarStatus ? sidecarStatus.mode : "—"}
            </p>
          </DiagField>
          <DiagField label="PID">
            <p className="text-sm text-zinc-700 dark:text-zinc-300" data-testid="diag-sidecar-pid">
              {sidecarStatus ? (sidecarStatus.pid || "N/A") : "—"}
            </p>
          </DiagField>
          <DiagField label="Healthz">
            <p className="text-sm text-zinc-700 dark:text-zinc-300" data-testid="diag-healthz">
              {sidecarStatus ? (sidecarStatus.healthz_last_check || "—") : "—"}
            </p>
          </DiagField>
          <DiagField label="IPC latency">
            <p className="text-sm text-zinc-700 dark:text-zinc-300" data-testid="diag-ipc-latency">
              {sidecarStatus ? sidecarStatus.ipc_latency_us + " µs" : "—"}
            </p>
          </DiagField>
        </div>
      </DiagCard>

      {/* Firewall status card */}
      <DiagCard icon={<Flame size={16} strokeWidth={1.75} />} title={t("diagnostics.firewall")}>
        {firewallStatus ? (
          <div className="space-y-1">
            <p className={"text-sm " + (firewallStatus.firewall_on ? "text-amber-600 dark:text-amber-400" : "text-green-600 dark:text-green-400")} data-testid="diag-firewall-status">
              {firewallStatus.firewall_on ? t("networkLayer.firewallOn") : t("networkLayer.firewallOff")}
            </p>
            <p className="text-xs text-zinc-500 dark:text-zinc-400">{firewallStatus.detail}</p>
          </div>
        ) : (
          <p className="text-xs text-zinc-500">{t("diagnostics.refresh")}</p>
        )}
      </DiagCard>

      {/* Request log table card */}
      <DiagCard icon={<Activity size={16} strokeWidth={1.75} />} title={t("diagnostics.reqLog")}>
        {reqLogs.length === 0 ? (
          <p className="text-xs text-zinc-500" data-testid="diag-no-logs">{t("diagnostics.noLogs")}</p>
        ) : (
          <div className="overflow-x-auto max-h-64 overflow-y-auto rounded-md border border-zinc-200 dark:border-zinc-700" data-testid="diag-log-table">
            <table className="w-full text-xs">
              <thead className="sticky top-0 bg-zinc-50 dark:bg-zinc-800">
                <tr className="text-left text-zinc-500 dark:text-zinc-400">
                  <th className="px-2 py-1">Time</th>
                  <th className="px-2 py-1">Platform</th>
                  <th className="px-2 py-1">Account</th>
                  <th className="px-2 py-1">Target</th>
                  <th className="px-2 py-1">Exit IP</th>
                  <th className="px-2 py-1">Method</th>
                  <th className="px-2 py-1">Status</th>
                  <th className="px-2 py-1">Duration</th>
                </tr>
              </thead>
              <tbody>
                {reqLogs.map((log, i) => (
                  <tr key={i} className="border-t border-zinc-100 dark:border-zinc-800">
                    <td className="px-2 py-1 text-zinc-500">{log.ts}</td>
                    <td className="px-2 py-1">{log.platform_name}</td>
                    <td className="px-2 py-1">{log.account}</td>
                    <td className="px-2 py-1">{log.target_host}</td>
                    <td className="px-2 py-1">{log.egress_ip}</td>
                    <td className="px-2 py-1">{log.http_method}</td>
                    <td className={"px-2 py-1 " + (log.http_status >= 200 && log.http_status < 300 ? "text-green-600" : "text-red-600")}>{log.http_status}</td>
                    <td className="px-2 py-1">{log.duration_ms}ms</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </DiagCard>

      {/* Exit IP probe card */}
      <DiagCard icon={<Globe size={16} strokeWidth={1.75} />} title={t("diagnostics.exitIpProbe")}>
        <div className="flex items-center gap-2 flex-wrap">
          <input
            type="number"
            data-testid="diag-probe-port"
            value={probePort}
            onChange={(e) => setProbePort(e.target.value)}
            className="w-20 text-xs rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 px-2 py-1"
            placeholder="Port"
          />
          <select
            data-testid="diag-probe-proto"
            value={probeProto}
            onChange={(e) => setProbeProto(e.target.value)}
            className="text-xs rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 px-2 py-1"
          >
            <option value="http">HTTP</option>
            <option value="socks5">SOCKS5</option>
          </select>
          <button data-testid="diag-probe-btn" onClick={() => void handleProbe()} disabled={probeBusy} className={btnCls}>
            {probeBusy ? <Loader2 size={14} className="animate-spin" /> : <Zap size={14} strokeWidth={1.75} />}
            {t("diagnostics.exitIpProbe")}
          </button>
          {probeResult && (
            <div className="text-xs space-y-0.5" data-testid="diag-probe-result">
              <p>IP: {probeResult.exit_ip || "N/A"}</p>
              <p>Latency: {probeResult.latency_ms}ms</p>
              <p>Status: {probeResult.status}</p>
              
            </div>
          )}
        </div>
      </DiagCard>

      {/* Port health check card */}
      <DiagCard icon={<Activity size={16} strokeWidth={1.75} />} title={t("diagnostics.portHealth")}>
        <div className="flex items-center gap-2 flex-wrap">
          <input
            type="number"
            data-testid="diag-health-port"
            value={healthPort}
            onChange={(e) => setHealthPort(e.target.value)}
            className="w-20 text-xs rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 px-2 py-1"
            placeholder="Port"
          />
          <select
            data-testid="diag-health-proto"
            value={healthProto}
            onChange={(e) => setHealthProto(e.target.value)}
            className="text-xs rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 px-2 py-1"
          >
            <option value="socks5">SOCKS5</option>
            <option value="http">HTTP</option>
          </select>
          <button data-testid="diag-health-btn" onClick={() => void handleHealth()} disabled={healthBusy} className={btnCls}>
            {healthBusy ? <Loader2 size={14} className="animate-spin" /> : <ArrowRight size={14} strokeWidth={1.75} />}
            {t("diagnostics.portHealth")}
          </button>
          {healthResult && (
            <div className="text-xs space-y-0.5" data-testid="diag-health-result">
              <p>Reachable: {healthResult.reachable ? "YES" : "NO"}</p>
              <p>Latency: {healthResult.latency_ms}ms</p>
              
            </div>
          )}
        </div>
      </DiagCard>

      {/* Sidecar log buffer card */}
      <DiagCard icon={<Server size={16} strokeWidth={1.75} />} title={t("diagnostics.sidecarLogs")}>
        <div className="max-h-48 overflow-y-auto rounded-md border border-zinc-200 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-950 p-2" data-testid="diag-sidecar-logs">
          {sidecarLogs.length === 0 ? (
            <p className="text-xs text-zinc-500">{t("diagnostics.noLogs")}</p>
          ) : (
            <pre className="text-xs text-zinc-600 dark:text-zinc-400 whitespace-pre-wrap font-mono">
              {sidecarLogs.join("\n")}
            </pre>
          )}
        </div>
      </DiagCard>

      {/* Log directory button */}
      <button
        data-testid="diag-open-logdir"
        onClick={() => void openLogDir()}
        className={btnCls}
      >
        <FolderOpen size={14} strokeWidth={1.75} />
        {t("diagnostics.openLogDir")}
      </button>

      {/* T8-2: Strategy verification */}
      <DiagCard icon={<Activity size={16} strokeWidth={1.75} />} title={t("strategyVerify.title")}>
        <div className="flex flex-col gap-3">
          <div className="flex gap-2 items-end">
            <DiagField label={t("strategyVerify.platform")}>
              <select
                value={selectedPlatform}
                onChange={(e) => setSelectedPlatform(e.target.value)}
                className="rounded border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-800 px-2 py-1 text-xs"
                data-testid="verify-platform-sel"
              >
                {platformNames.map((p) => <option key={p} value={p}>{p}</option>)}
              </select>
            </DiagField>
            <DiagField label={t("strategyVerify.sampleCount")}>
              <input
                type="number"
                min={3}
                max={50}
                value={sampleCount}
                onChange={(e) => setSampleCount(Number(e.target.value) || 10)}
                className="rounded border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-800 px-2 py-1 text-xs w-20"
                data-testid="verify-sample-count"
              />
            </DiagField>
            <button
              onClick={() => void runStrategyVerify()}
              disabled={verifyBusy || !selectedPlatform}
              className={btnCls}
              data-testid="verify-run-btn"
            >
              {verifyBusy ? <Loader2 size={14} className="animate-spin" /> : <Zap size={14} strokeWidth={1.75} />}
              {t("strategyVerify.run")}
            </button>
          </div>
          {verifyResult && (
            <div className="rounded-md border border-zinc-200 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-950 p-3 space-y-1" data-testid="verify-result">
              <div className="flex gap-4 text-xs">
                <span className="text-zinc-500">{t("diagnostics.portHealth.platform")}: <b className="text-zinc-700 dark:text-zinc-300">{String(verifyResult.platform ?? "-")}</b></span>
                <span className="text-zinc-500">{t("diagnostics.portHealth.policy")}: <b className="text-zinc-700 dark:text-zinc-300">{String(verifyResult.strategy ?? "-")}</b></span>
                <span className="text-zinc-500">{t("strategyVerify.uniqueIps")}: <b className="text-zinc-700 dark:text-zinc-300">{String(verifyResult.unique_ips ?? "-")}</b></span>
                <span className="text-zinc-500">{t("strategyVerify.latency")}: <b className="text-zinc-700 dark:text-zinc-300">{String(verifyResult.avg_latency_ms ?? "-")}ms</b></span>
              </div>
              {verifyResult.distribution != null && (
                <div className="space-y-0.5">
                  {Object.entries((verifyResult.distribution ?? {}) as Record<string, number>).map(([ip, count]) => (
                    <div key={ip} className="flex items-center gap-2 text-xs">
                      <span className="font-mono text-zinc-600 dark:text-zinc-400" key={ip}>{ip}</span>
                      <div className="flex-1 bg-zinc-200 dark:bg-zinc-800 rounded-full h-2 overflow-hidden">
                        <div className="bg-blue-500 h-full" style={{ width: `${(count as number) / Number(verifyResult.sample_count ?? 1) * 100}%` }} />
                      </div>
                      <span className="text-zinc-400">{String(count)}x</span>
                    </div>
                  ))}
                </div>
              )}
              {verifyResult.error != null && (
                <p className="text-xs text-red-500">{String(verifyResult.error)}</p>
              )}
            </div>
          )}
        </div>
      </DiagCard>

      {/* T8-6: Connection control */}
      <DiagCard icon={<Flame size={16} strokeWidth={1.75} />} title={t("connectionControl.title")}>
        <div className="flex gap-2">
          <button
            onClick={() => void handleCloseAll()}
            className="px-3 py-1.5 rounded text-xs font-medium bg-red-100 hover:bg-red-200 dark:bg-red-900/30 dark:hover:bg-red-800/40 text-red-700 dark:text-red-400 transition-colors"
            data-testid="conn-close-all"
          >
            <Flame size={14} strokeWidth={1.75} className="inline mr-1" />
            {t("diagnostics.closeAllConnections")}
          </button>
          <button
            onClick={() => void handleResetKernel()}
            className="px-3 py-1.5 rounded text-xs font-medium bg-orange-100 hover:bg-orange-200 dark:bg-orange-900/30 dark:hover:bg-orange-800/40 text-orange-700 dark:text-orange-400 transition-colors"
            data-testid="conn-reset-kernel"
          >
            <Activity size={14} strokeWidth={1.75} className="inline mr-1" />
            {t("diagnostics.resetKernel")}
          </button>
        </div>
      </DiagCard>
    </div>
  );
}
