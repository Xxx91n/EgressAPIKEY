import { useEffect, useState } from "react";
import { usePoll } from "../hooks/usePoll";
import { useTranslation } from "react-i18next";
import { Activity, Stethoscope, Flame, Globe, Server, Loader2, FolderOpen, ArrowRight, Zap } from "lucide-react"; // T8-2/T8-3 added Zap for verify button
import { openPath } from "@tauri-apps/plugin-opener";
import { invoke } from "@tauri-apps/api/core";
import {
  getDiagPollInterval,
  setDiagPollInterval,
} from "../lib/settings";
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
  ipcMetricsRealtimeThroughput,
  ipcMetricsProbeHistory,
  // request-log detail drawer.
  ipcRequestLogDetail,
  ipcRequestLogPayloads,
  decodePayloadPart,
  PAYLOAD_DISPLAY_CAP_BYTES,
  type SidecarStatus,
  type FirewallStatus,
  type RequestLogEntry,
  type ExitIpProbe,
  type PortHealthCheck,
  type MetricsThroughput,
  type MetricsProbeHistory,
  type RequestLogDetail,
  type RequestLogPayloads,
} from "../lib/ipc";
import { HeadlessCapabilityNotice, commandBlocked } from "../components/HeadlessCapabilityNotice";

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

// (ADR-0064): zero-dependency inline SVG line chart. Recharts was
// considered and rejected for the minimal set (ponytail: no new deps for a
// two-card diagnostic; revisit when the full 12-endpoint Metrics tab lands).
// Renders one polyline per series over a shared time axis; empty/NaN input
// renders as the empty-state string instead of a broken axis.
function MetricsSparkline({
  points,
  color,
}: {
  points: { ts: number; value: number }[];
  color: string;
}) {
  if (points.length === 0) return null;
  const W = 480;
  const H = 96;
  const xs = points.map((p) => p.ts);
  const ys = points.map((p) => p.value);
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const maxY = Math.max(...ys, 0);
  const spanX = maxX - minX || 1;
  const spanY = maxY || 1;
  const path = points
    .map((p, i) => {
      const x = ((p.ts - minX) / spanX) * (W - 4) + 2;
      const y = H - 2 - (p.value / spanY) * (H - 6);
      return `${i === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  return (
    <svg viewBox={`0 0 ${W} ${H}`} className="w-full h-24" data-testid="metrics-sparkline">
      <path d={path} fill="none" stroke={color} strokeWidth="1.5" />
    </svg>
  );
}

export /** T21: one collapsible payload half (headers/body). b64 decoded via
 *  decodePayloadPart — 1 MB display cap enforced there, flag drives the label. */
function PayloadPart({ label, b64, truncatedUpstream, testId }: {
  label: string;
  b64: string;
  truncatedUpstream: boolean;
  testId: string;
}) {
  const { t } = useTranslation();
  const decoded = decodePayloadPart(b64);
  if (!decoded.text && !truncatedUpstream) return null;
  return (
    <details className="rounded border border-zinc-200 dark:border-zinc-700">
      <summary className="px-2 py-1 text-xs cursor-pointer text-zinc-600 dark:text-zinc-300">{label}</summary>
      {truncatedUpstream && (
        <p className="px-2 text-xs text-amber-600 dark:text-amber-400" data-testid={testId + "-upstream-truncated"}>
          {t("diagnostics.payloadTruncatedUpstream")}
        </p>
      )}
      {decoded.displayTruncated && (
        <p className="px-2 text-xs text-amber-600 dark:text-amber-400" data-testid={testId + "-display-truncated"}>
          {t("diagnostics.payloadDisplayTruncated", { n: PAYLOAD_DISPLAY_CAP_BYTES })}
        </p>
      )}
      <pre className="px-2 py-1 text-xs whitespace-pre-wrap break-all max-h-48 overflow-y-auto" data-testid={testId}>{decoded.text}</pre>
    </details>
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
  // Strategy verification state
  const [platformNames, setPlatformNames] = useState<string[]>([]);
  const [selectedPlatform, setSelectedPlatform] = useState("");
  const [sampleCount, setSampleCount] = useState(10);
  const [verifyResult, setVerifyResult] = useState<Record<string, unknown> | null>(null);
  const [verifyBusy, setVerifyBusy] = useState(false);
  const [healthPort, setHealthPort] = useState("1790");
  const [healthProto, setHealthProto] = useState("mixed");
  const [healthResult, setHealthResult] = useState<PortHealthCheck | null>(null);
  const [healthBusy, setHealthBusy] = useState(false);
  // (ADR-0064): Resin metrics minimal-set state (pull model).
  const [metricsThroughput, setMetricsThroughput] = useState<MetricsThroughput | null>(null);
  const [metricsProbes, setMetricsProbes] = useState<MetricsProbeHistory | null>(null);
  const [probeRange, setProbeRange] = useState<"1h" | "24h" | "7d">("24h");
// request-log detail drawer state (pull model — opened by
  // clicking a request_log_tail row; the two detail commands are called only
  // then, riding the same untrusted-coercion wrappers as the tail).
  const [detailLogId, setDetailLogId] = useState<string | null>(null);
  const [logDetail, setLogDetail] = useState<RequestLogDetail | null>(null);
  const [logPayloads, setLogPayloads] = useState<RequestLogPayloads | null>(null);
  const [detailBusy, setDetailBusy] = useState(false);
  const [detailError, setDetailError] = useState(false);
  // Load poll interval from settings (typed L1 command pair, no bare store invoke)
  useEffect(() => {
    (async () => {
      try {
        const stored = await getDiagPollInterval();
        if (stored >= 1000 && stored <= 60000) setPollInterval(stored);
      } catch { /* default 5000 */ }
    })();
  }, []);

  // Save poll interval when changed
  const handlePollChange = async (ms: number) => {
    const clamped = Math.max(1000, Math.min(60000, ms));
    setPollInterval(clamped);
    try { await setDiagPollInterval(clamped); } catch { /* vitest no-op */ }
  };

  // (ADR-0064): probe-history fetch with a from/to window computed here.
  // toISOString() is RFC3339 with millis — accepted by the upstream
  // RFC3339Nano parser; the window is boundary-validated on both the TS and
  // the Rust command side (§7.5 dual cover).
  const PROBE_RANGE_MS: Record<string, number> = {
    "1h": 3600_000,
    "24h": 24 * 3600_000,
    "7d": 7 * 24 * 3600_000,
  };
  const fetchProbeHistory = async (range: string) => {
    const to = new Date();
    const from = new Date(Date.now() - (PROBE_RANGE_MS[range] ?? PROBE_RANGE_MS["24h"]));
    try {
      const r = await ipcMetricsProbeHistory(from.toISOString(), to.toISOString());
      setMetricsProbes(r);
    } catch {
      setMetricsProbes(null);
    }
  };

  const handleProbeRangeChange = (range: "1h" | "24h" | "7d") => {
    setProbeRange(range);
    void fetchProbeHistory(range);
  };

  // open the detail drawer for one request-log row. Rows without an id
  // (wire-shape regression or older sidecar) stay inert.
  const openLogDetail = async (log: RequestLogEntry) => {
    if (!log.id) return;
    setDetailLogId(log.id);
    setDetailBusy(true);
    setDetailError(false);
    setLogDetail(null);
    setLogPayloads(null);
    try {
      const [d, payloads] = await Promise.all([
        ipcRequestLogDetail(log.id),
        ipcRequestLogPayloads(log.id).catch(() => null),
      ]);
      setLogDetail(d);
      setLogPayloads(payloads);
    } catch {
      setDetailError(true);
    } finally {
      setDetailBusy(false);
    }
  };

  // Main refresh: fetch all diagnostic data
  const refreshDiagnostics = async () => {
    setDiagBusy(true);
    try {
      const [status, fw, logs, sideLogs, tp] = await Promise.all([
        ipcGetSidecarStatus().catch(() => null),
        ipcCheckFirewallStatus().catch(() => null),
        ipcRequestLogTail(50).catch(() => []),
        invoke<string[]>("get_sidecar_logs").catch(() => []),
        // (ADR-0064): realtime throughput rides the same poll cycle.
        ipcMetricsRealtimeThroughput().catch(() => null),
      ]);
      if (status) setSidecarStatus(status);
      if (fw) setFirewallStatus(fw);
      setReqLogs(Array.isArray(logs) ? logs : []);
      setSidecarLogs(Array.isArray(sideLogs) ? sideLogs : []);
      setMetricsThroughput(tp);
      void fetchProbeHistory(probeRange);
    } finally {
      setDiagBusy(false);
    }
  };

  // usePoll replaces hand-rolled setInterval + visibilitychange.
  usePoll(refreshDiagnostics, { intervalMs: pollInterval, fireImmediately: true });

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

  // Load platform names for strategy verify dropdown
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

  // Run strategy verification
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

  // Close all connections (kill sidecar — drops all active TCP)
  const handleCloseAll = async () => {
    if (!window.confirm(t("connectionControl.confirmClose"))) return;
    try {
      await ipcCloseAllConnections();
    } catch { /* not in tauri */ }
  };

  // Reset kernel (kill + restart sidecar)
  const handleResetKernel = async () => {
    if (!window.confirm(t("connectionControl.confirmReset"))) return;
    try {
      await ipcResetKernel();
    } catch { /* not in tauri */ }
  };

  return (
    <div className="w-full max-w-none px-6 space-y-4" data-testid="diag-view">
      <HeadlessCapabilityNotice commands={["get_sidecar_status", "check_firewall_status", "probe_exit_ip", "strategy_verify", "close_all_connections", "reset_kernel"]} />
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
                  <tr
                    key={i}
                    className={"border-t border-zinc-100 dark:border-zinc-800" + (log.id ? " cursor-pointer hover:bg-zinc-50 dark:hover:bg-zinc-800" : "")}
                    data-testid="diag-log-row"
                    title={log.id ? t("diagnostics.logDetail") : undefined}
                    onClick={() => void openLogDetail(log)}
                  >
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
        {detailLogId && (
          <div className="rounded-md border border-zinc-300 dark:border-zinc-700 p-3 space-y-2 bg-zinc-50 dark:bg-zinc-900" data-testid="diag-log-detail">
            <div className="flex items-center justify-between">
              <p className="text-xs font-semibold text-zinc-700 dark:text-zinc-300">{t("diagnostics.logDetail")}</p>
              <button
                data-testid="diag-log-detail-close"
                className={btnCls}
                onClick={() => setDetailLogId(null)}
              >
                {t("diagnostics.close")}
              </button>
            </div>
            {detailBusy ? (
              <p className="text-xs text-zinc-500 flex items-center gap-1" data-testid="diag-log-detail-busy">
                <Loader2 size={12} className="animate-spin" />
                {t("diagnostics.logDetail")}
              </p>
            ) : detailError ? (
              <p className="text-xs text-red-600" data-testid="diag-log-detail-error">{t("diagnostics.detailUnavailable")}</p>
            ) : logDetail ? (
              <div className="space-y-2">
                <div className="grid grid-cols-2 md:grid-cols-4 gap-x-3 gap-y-1 text-xs" data-testid="diag-log-detail-grid">
                  {([
                    ["id", logDetail.id],
                    ["ts", logDetail.ts],
                    ["http_method", logDetail.http_method],
                    ["http_status", String(logDetail.http_status)],
                    ["platform_name", logDetail.platform_name],
                    ["account", logDetail.account],
                    ["target_host", logDetail.target_host],
                    ["target_url", logDetail.target_url],
                    ["node_tag", logDetail.node_tag],
                    ["egress_ip", logDetail.egress_ip],
                    ["duration_ms", String(logDetail.duration_ms)],
                    ["first_byte_duration_ms", String(logDetail.first_byte_duration_ms)],
                    ["net_ok", logDetail.net_ok ? t("diagnostics.netOk") : t("diagnostics.netFail")],
                    ["ingress_bytes", String(logDetail.ingress_bytes)],
                    ["egress_bytes", String(logDetail.egress_bytes)],
                    ["resin_error", logDetail.resin_error],
                  ] as const).map(([k, v]) => (
                    <div key={k} data-testid={"diag-log-detail-" + k}>
                      <span className="font-mono text-zinc-500 dark:text-zinc-400">{k}</span>
                      <p className="text-zinc-700 dark:text-zinc-300 break-all">{v || "—"}</p>
                    </div>
                  ))}
                </div>
                {logDetail.payload_present && logPayloads ? (
                  <div className="space-y-1" data-testid="diag-log-payloads">
                    <p className="text-xs font-semibold text-zinc-600 dark:text-zinc-300">{t("diagnostics.payloadsTitle")}</p>
                    <PayloadPart label={t("diagnostics.reqHeaders")} b64={logPayloads.req_headers_b64} truncatedUpstream={logPayloads.truncated.req_headers} testId="diag-payload-req-headers" />
                    <PayloadPart label={t("diagnostics.reqBody")} b64={logPayloads.req_body_b64} truncatedUpstream={logPayloads.truncated.req_body} testId="diag-payload-req-body" />
                    <PayloadPart label={t("diagnostics.respHeaders")} b64={logPayloads.resp_headers_b64} truncatedUpstream={logPayloads.truncated.resp_headers} testId="diag-payload-resp-headers" />
                    <PayloadPart label={t("diagnostics.respBody")} b64={logPayloads.resp_body_b64} truncatedUpstream={logPayloads.truncated.resp_body} testId="diag-payload-resp-body" />
                  </div>
                ) : (
                  <p className="text-xs text-zinc-500" data-testid="diag-log-no-payload">{t("diagnostics.noPayload")}</p>
                )}
              </div>
            ) : null}
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
            <option value="mixed">MIXED</option>
            <option value="http">HTTP</option>
            <option value="socks5">SOCKS5</option>
          </select>
          <button data-testid="diag-probe-btn" onClick={() => void handleProbe()} disabled={probeBusy || commandBlocked("probe_exit_ip")} className={btnCls}>
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
            <option value="mixed">MIXED</option>
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

      {/* T19 (ADR-0064): Resin metrics minimal set — realtime throughput +
          probe history. Pull model; rides the diagnostics poll cycle. */}
      <DiagCard icon={<Activity size={16} strokeWidth={1.75} />} title={t("metrics.title")}>
        <div className="space-y-3">
          <div>
            <div className="flex items-center justify-between">
              <span className="text-xs text-zinc-500 dark:text-zinc-400">{t("metrics.throughput")}</span>
              {metricsThroughput && metricsThroughput.items.length > 0 && (
                <span className="text-xs text-zinc-400" data-testid="metrics-throughput-latest">
                  ↓ {(metricsThroughput.items[metricsThroughput.items.length - 1].egress_bps / 1000).toFixed(1)} kb/s
                </span>
              )}
            </div>
            {metricsThroughput && metricsThroughput.items.length > 0 ? (
              <MetricsSparkline
                points={metricsThroughput.items
                  .filter((p) => p && typeof p.ts === "string" && Number.isFinite(Date.parse(p.ts)))
                  .map((p) => ({ ts: Date.parse(p.ts), value: Number(p.egress_bps) || 0 }))}
                color="#3b82f6"
              />
            ) : (
              <p className="text-xs text-zinc-500" data-testid="metrics-throughput-empty">{t("metrics.noData")}</p>
            )}
          </div>
          <div>
            <div className="flex items-center justify-between">
              <span className="text-xs text-zinc-500 dark:text-zinc-400">{t("metrics.probeHistory")}</span>
              <select
                data-testid="metrics-probe-range"
                value={probeRange}
                onChange={(e) => handleProbeRangeChange(e.target.value as "1h" | "24h" | "7d")}
                className="text-xs rounded border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 px-2 py-1"
              >
                <option value="1h">1h</option>
                <option value="24h">24h</option>
                <option value="7d">7d</option>
              </select>
            </div>
            {metricsProbes && metricsProbes.items.length > 0 ? (
              <MetricsSparkline
                points={metricsProbes.items
                  .filter((b) => b && typeof b.bucket_start === "string" && Number.isFinite(Date.parse(b.bucket_start)))
                  .map((b) => ({ ts: Date.parse(b.bucket_start), value: Number(b.total_count) || 0 }))}
                color="#10b981"
              />
            ) : (
              <p className="text-xs text-zinc-500" data-testid="metrics-probes-empty">{t("metrics.noData")}</p>
            )}
          </div>
        </div>
      </DiagCard>

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
              disabled={verifyBusy || !selectedPlatform || commandBlocked("strategy_verify")}
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
          <button disabled={commandBlocked("close_all_connections")}
            onClick={() => void handleCloseAll()}
            className="px-3 py-1.5 rounded text-xs font-medium bg-red-100 hover:bg-red-200 dark:bg-red-900/30 dark:hover:bg-red-800/40 text-red-700 dark:text-red-400 transition-colors"
            data-testid="conn-close-all"
          >
            <Flame size={14} strokeWidth={1.75} className="inline mr-1" />
            {t("diagnostics.closeAllConnections")}
          </button>
          <button disabled={commandBlocked("reset_kernel")}
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
