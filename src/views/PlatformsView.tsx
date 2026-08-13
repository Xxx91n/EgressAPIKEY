import { useEffect, useState, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Trash2, Loader2, AlertCircle, CheckCircle2, Plug, ShieldCheck, ShieldAlert, Copy } from "lucide-react";
import { useAppStore } from "../store/appStore";
import { translateError } from "../lib/i18n-error";
import {
  ipcPlatformRemove,
  ipcPlatformListFull,
  ipcStrategyConfigGet,
  ipcStrategyConfigPut,
  ipcStrategyApply,
  type PlatformStrategy,
  type StrategyConfig,
  ipcPlatformLeases,
  ipcPlatformCreateWithFields,
  ipcPlatformUpdate,
  ipcPortList,
  ipcPortUpsert,
  ipcPortBindPlatform,
  ipcPortRemove,
  ipcPortAuthInfo,
  ipcPortHealthCheck,
  ipcPortSuggest,
  ipcNodeList,
  ipcSubscriptionList,

  type PortMapping,
  type PortAuthInfo,
  type PortHealthCheck,
} from "../lib/ipc";
import { strategyToI18nKey, strategyToResinPolicy, mapResinToShell, STRATEGY_IDS, type StrategyId } from "../lib/strategy";
import { loadSplitRatio, saveSplitRatio, loadPortAuthDefault, savePortAuthDefault } from "../lib/settings";

/** Phase 5 / ADR-0012: left = Entry Ports, right = Platforms. Port = identity. */
interface PlatformInfoFull {
  name: string;
  allocationPolicy: string;
  regexFilters: string[];
  regionFilters: string[];
  routableNodeCount: number;
  stickyTtl: string;
}

interface NodeEntry { display_tag: string; region: string; node_hash: string; }

export function PlatformsView() {
  const { t } = useTranslation();
  const removePlatform = useAppStore((s) => s.removePlatform);
  const [ports, setPorts] = useState<PortMapping[]>([]);
  const [newPort, setNewPort] = useState("17990");
  const [newProto, setNewProto] = useState<"socks5" | "http">("socks5");
  const [newLabel, setNewLabel] = useState("");
  const [newPlatformName, setNewPlatformName] = useState("Default");
  const [newAuthRequired, setNewAuthRequired] = useState(true);
  const [platforms, setPlatforms] = useState<PlatformInfoFull[]>([]);
  const [leasesPerPlatform, setLeasesPerPlatform] = useState<Record<string, unknown[]>>({});
  const [draggingPort, setDraggingPort] = useState<number | null>(null);
  const [dragOverPlatform, setDragOverPlatform] = useState<string | null>(null);
  const [splitRatio, setSplitRatio] = useState(0.4);
  const [toast, setToast] = useState<{ kind: "ok" | "err"; msg: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [createName, setCreateName] = useState("");
  const [createPolicy, setCreatePolicy] = useState<StrategyId>("random");
  const [createFormError, setCreateFormError] = useState<string | null>(null);
  /// ADR-0021 Q1: per-port auth info cache (port -> credentials displayed inline).
  const [authInfo, setAuthInfo] = useState<Record<number, PortAuthInfo>>({});
  /// ADR-0021 Q1: per-port health probe result (port -> chip color + reason).
  const [health, setHealth] = useState<Record<number, PortHealthCheck>>({});
  const [copiedPort, setCopiedPort] = useState<number | null>(null);

  // T4-4: strategy panel state.
  const [strategyConfig, setStrategyConfig] = useState<StrategyConfig>({ version: 1, platforms: [] });
  const [strategyBusy, setStrategyBusy] = useState(false);
  const [strategyTopNInput, setStrategyTopNInput] = useState<Record<string, string>>({});
  // T10-2: node list + subscription list for chip-based A-class selectors.
  const [nodeList, setNodeList] = useState<NodeEntry[]>([]);
  const [subList, setSubList] = useState<{ name: string; node_count: number }[]>([]);
  /// Collapsible inline strategy panel: which platform card is expanded.
  const refreshStrategy = useCallback(async () => {
    try {
      const cfg = await ipcStrategyConfigGet();
      if (cfg && cfg.version === 1 && Array.isArray(cfg.platforms)) setStrategyConfig(cfg);
    } catch { /* outside Tauri */ }
  }, []);


  const updateStrategyField = (platformName: string, field: keyof PlatformStrategy, value: string | string[]) => {
    setStrategyConfig((prev) => {
      let platforms = [...prev.platforms];
      let idx = platforms.findIndex((p) => p.platform_name === platformName);
      if (idx === -1) {
        platforms.push({ platform_name: platformName, a_class: "manual", b_class: "random" });
        idx = platforms.length - 1;
      }
      platforms[idx] = { ...platforms[idx], [field]: value };
      return { ...prev, platforms };
    });
  };

  const handleApplyStrategy = async () => {
    setStrategyBusy(true);
    try {
      await ipcStrategyConfigPut(strategyConfig);
      const result = await ipcStrategyApply();
      const allPatched = result.platforms.every((p) => p.patched);
      showToast(allPatched ? "ok" : "err", allPatched ? t("strategy.applyOk") : t("strategy.applyPartial"));
      await refreshPlatforms();
    } catch (e) { showToast("err", translateError(e, t)); }
    finally { setStrategyBusy(false); }
  };





  const containerRef = useRef<HTMLDivElement>(null);
  const resizingRef = useRef(false);

  const refreshPorts = useCallback(async (): Promise<PortMapping[]> => {
    try {
      const list = await ipcPortList();
      setPorts(list);
      return list;
    } catch { /* outside Tauri */ return []; }
  }, []);

  const refreshPlatforms = useCallback(async () => {
    try {
      const raw = await ipcPlatformListFull();
      const items = Array.isArray(raw) ? raw : ((raw as Record<string, unknown>)?.items ?? []);
      const mapped = (items as Record<string, unknown>[]).map((p) => ({
        name: String(p.name ?? ""),
        allocationPolicy: mapResinToShell(String(p.allocation_policy ?? "BALANCED")),
        regexFilters: Array.isArray(p.regex_filters) ? (p.regex_filters as string[]) : [],
        regionFilters: Array.isArray(p.region_filters) ? (p.region_filters as string[]) : [],
        routableNodeCount: Number(p.routable_node_count ?? 0),
        stickyTtl: String(p.sticky_ttl ?? ""),
      })).filter((p) => p.name);
      setPlatforms(mapped);
      const leaseMap: Record<string, unknown[]> = {};
      await Promise.all(mapped.map(async (p) => {
        try {
          const leases = await ipcPlatformLeases(p.name);
          const lv = leases as unknown;
          leaseMap[p.name] = (Array.isArray(lv) ? lv : ((lv as Record<string, unknown>)?.items ?? [])) as unknown[];
        } catch { leaseMap[p.name] = []; }
      }));
      setLeasesPerPlatform(leaseMap);
    } catch { /* outside Tauri */ }
  }, []);

  /// ADR-0021 Q1: after each ports refresh, fetch auth-info + health probes
  /// in parallel. The probes are best-effort (outside Tauri in vitest) so
  /// the failure path leaves the chip unrendered rather than crashing the view.
  const refreshPortAuthAndHealth = useCallback(async (list: PortMapping[]) => {
    if (list.length === 0) return;
    const [authResults, healthResults] = await Promise.all([
      Promise.all(list.map(async (p) => {
        try { return [p.port, await ipcPortAuthInfo(p.port)] as const; } catch { return null; }
      })),
      Promise.all(list.map(async (p) => {
        try { return [p.port, await ipcPortHealthCheck(p.port, p.protocol)] as const; } catch { return null; }
      })),
    ]);
    const authMap: Record<number, PortAuthInfo> = {};
    for (const r of authResults) if (r) authMap[r[0]] = r[1];
    setAuthInfo(authMap);
    const healthMap: Record<number, PortHealthCheck> = {};
    for (const r of healthResults) if (r) healthMap[r[0]] = r[1];
    setHealth(healthMap);
  }, []);

  const showToast = (kind: "ok" | "err", msg: string) => {
    setToast({ kind, msg });
    window.setTimeout(() => setToast(null), 3500);
  };

  /// Copy SOCKS5 credentials `username:password` to the clipboard; show a
  /// transient "copied" badge on the row so the user has visual feedback.
  const copyCredentials = (port: number, auth: PortAuthInfo) => {
    const cred = `${auth.username}:${auth.password}`;
    try {
      void navigator.clipboard?.writeText(cred).then(() => {
        setCopiedPort(port);
        window.setTimeout(() => setCopiedPort((c) => (c === port ? null : c)), 1500);
      });
    } catch { /* clipboard may be unavailable outside https or in vitest */ }
  };

  useEffect(() => {
    loadSplitRatio().then((r) => { if (typeof r === "number" && r > 0.15 && r < 0.85) setSplitRatio(r); }).catch((e) => console.warn("[PlatformsView] loadSplitRatio failed", e));
    loadPortAuthDefault().then((v) => { if (typeof v === "boolean") setNewAuthRequired(v); }).catch((e) => console.warn("[PlatformsView] loadPortAuthDefault failed", e));
    ipcPortSuggest().then((p) => { setNewPort(String(p)); }).catch((e) => console.warn("[PlatformsView] ipcPortSuggest failed", e));
    ipcNodeList().then((raw) => { const arr = (Array.isArray(raw) ? raw : ((raw as Record<string, unknown>)?.items ?? [])) as Record<string, unknown>[]; setNodeList(arr.map((n) => ({ display_tag: String(n.display_tag ?? ""), region: String(n.region ?? ""), node_hash: String(n.node_hash ?? "") }))); }).catch((e) => console.warn("[PlatformsView] ipcNodeList failed", e));
    ipcSubscriptionList().then((subs) => { setSubList(subs.map((s) => ({ name: s.name, node_count: s.node_count }))); }).catch((e) => console.warn("[PlatformsView] ipcSubscriptionList failed", e));
    void refreshPorts().then((list) => { void refreshPortAuthAndHealth(list); });
    void refreshPlatforms();
    void refreshStrategy();
  }, [refreshPorts, refreshPortAuthAndHealth, refreshPlatforms, refreshStrategy]);

  const handleAddPort = async () => {
    const port = Number(newPort);
    if (!Number.isInteger(port) || port < 1024 || port > 65535) { showToast("err", t("platform.portInvalid")); return; }
    if (ports.some((p) => p.port === port)) { showToast("err", t("platform.portDuplicate")); return; }
    setBusy(true);
    try {
      await ipcPortUpsert({
        port,
        protocol: newProto,
        platform_name: newPlatformName.trim(),
        account: "port-" + port,
        label: newLabel.trim() || ("entry-" + port),
        enabled: true,
        auth_required: newAuthRequired,
      });
      setNewLabel("");
      showToast("ok", t("platform.portAddOk"));
      await refreshPortAuthAndHealth(await refreshPorts());
    } catch (e) { showToast("err", translateError(e, t)); }
    finally { setBusy(false); }
  };

  const handleRemovePort = async (port: number) => {
    setBusy(true);
    try {
      await ipcPortRemove(port);
      showToast("ok", t("platform.portRemoved"));
      // Drop the stale health/auth entry so the row does not flash the old
      // chip when the re-render sees the port gone before the health probe runs.
      setHealth((s) => { const x = { ...s }; delete x[port]; return x; });
      setAuthInfo((s) => { const x = { ...s }; delete x[port]; return x; });
      await refreshPortAuthAndHealth(await refreshPorts());
    }
    catch (e) { showToast("err", translateError(e, t)); }
    finally { setBusy(false); }
  };

  const bindPortToPlatform = async (port: number, platformName: string) => {
    const row = ports.find((p) => p.port === port);
    if (!row) return;
    setBusy(true);
    try {
      await ipcPortBindPlatform(port, platformName);
      showToast("ok", t("platform.portBound", { port, platform: platformName }));
      await refreshPortAuthAndHealth(await refreshPorts());
    } catch (e) { showToast("err", translateError(e, t)); }
    finally { setBusy(false); setDraggingPort(null); setDragOverPlatform(null); }
  };

  const handleCreatePlatform = async () => {
    const name = createName.trim();
    if (!name) { setCreateFormError(t("platform.createEmptyName")); return; }
    setBusy(true); setCreateFormError(null);
    try {
      await ipcPlatformCreateWithFields({ name, allocation_policy: strategyToResinPolicy(createPolicy) });
      setCreateDialogOpen(false); setCreateName("");
      showToast("ok", t("platform.addOk"));
      await refreshPlatforms();
    } catch (e) { setCreateFormError(translateError(e, t)); }
    finally { setBusy(false); }
  };

  const handleDeletePlatform = async (name: string) => {
    setBusy(true);
    try { await ipcPlatformRemove(name); removePlatform(name); showToast("ok", t("platform.deleteConfirm")); await refreshPlatforms(); }
    catch (e) { showToast("err", translateError(e, t)); }
    finally { setBusy(false); }
  };

  const onSplitterPointerDown = (e: React.PointerEvent) => {
    resizingRef.current = true;
    (e.target as HTMLElement).setPointerCapture?.(e.pointerId);
  };
  const onSplitterPointerMove = (e: React.PointerEvent) => {
    if (!resizingRef.current || !containerRef.current) return;
    const rect = containerRef.current.getBoundingClientRect();
    const ratio = (e.clientX - rect.left) / rect.width;
    setSplitRatio(Math.min(0.75, Math.max(0.25, ratio)));
  };
  const onSplitterPointerUp = () => {
    if (!resizingRef.current) return;
    resizingRef.current = false;
    void saveSplitRatio(splitRatio);
  };

  return (
    <section className="flex h-full w-full flex-col gap-3 p-4" data-testid="platforms-view">
      <header className="flex items-center justify-between gap-2">
        <div>
          <h1 className="text-lg font-semibold tracking-tight">{t("platform.title")}</h1>
          <p className="text-xs text-muted-foreground">{t("platform.splitHint")}</p>
        </div>
        <button type="button" className="inline-flex items-center gap-1 rounded-md border px-3 py-1.5 text-sm" onClick={() => setCreateDialogOpen(true)} data-testid="platform-create-open">
          <Plus className="h-4 w-4" />
          {t("platform.createTitle")}
        </button>
      </header>

      {toast && (
        <div className={"flex items-center gap-2 rounded-md border px-3 py-2 text-sm " + (toast.kind === "ok" ? "border-emerald-500/40 bg-emerald-500/10" : "border-red-500/40 bg-red-500/10")} data-testid="platforms-toast">
          {toast.kind === "ok" ? <CheckCircle2 className="h-4 w-4" /> : <AlertCircle className="h-4 w-4" />}
          <span>{toast.msg}</span>
        </div>
      )}

      <div ref={containerRef} className="flex min-h-0 flex-1 overflow-hidden rounded-lg border">
        <div className="flex min-h-0 flex-col overflow-hidden" style={{ width: (splitRatio * 100) + "%" }} data-testid="ports-pane">
          <div className="border-b px-3 py-2 text-sm font-medium">{t("platform.entryPorts")}</div>
          <div className="space-y-2 border-b p-3">
            <div className="grid grid-cols-2 gap-2">
              <input className="rounded border bg-background px-2 py-1.5 text-sm" value={newPort} onChange={(e) => setNewPort(e.target.value)} placeholder={t("platform.port")} data-testid="port-input" />
              <select className="rounded border bg-background px-2 py-1.5 text-sm" value={newProto} onChange={(e) => setNewProto(e.target.value as "socks5" | "http")} data-testid="port-protocol">
                <option value="socks5">SOCKS5</option>
                <option value="http">HTTP</option>
              </select>
              <input className="rounded border bg-background px-2 py-1.5 text-sm" value={newPlatformName} onChange={(e) => setNewPlatformName(e.target.value)} placeholder={t("platform.name")} data-testid="port-platform" />
              <input className="rounded border bg-background px-2 py-1.5 text-sm" value={newLabel} onChange={(e) => setNewLabel(e.target.value)} placeholder={t("platform.portLabel")} data-testid="port-label" />
            </div>
            <label className="flex items-center gap-1.5 text-xs text-muted-foreground" data-testid="port-auth-toggle">
              <input type="checkbox" checked={newAuthRequired} onChange={(e) => { setNewAuthRequired(e.target.checked); void savePortAuthDefault(e.target.checked); }} className="h-3.5 w-3.5" />
              {t("platform.requireAuth")}
            </label>
            <button type="button" disabled={busy} onClick={() => void handleAddPort()} className="inline-flex w-full items-center justify-center gap-1 rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground disabled:opacity-50" data-testid="port-add">
              {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Plus className="h-4 w-4" />}
              {t("platform.addPort")}
            </button>
          </div>
          <ul className="min-h-0 flex-1 space-y-2 overflow-auto p-3">
            {ports.length === 0 && <li className="text-xs text-muted-foreground">{t("platform.noPorts")}</li>}
            {ports.map((p) => (
              <li key={p.port} className={"cursor-grab rounded-md border bg-card p-3 text-sm " + (draggingPort === p.port ? "opacity-50 cursor-grabbing" : "")} onPointerDown={(e) => { e.preventDefault(); document.body.style.userSelect = "none"; setDraggingPort(p.port); }} onPointerUp={() => { document.body.style.userSelect = ""; }} data-testid={"port-row-" + p.port}>
                <div className="flex items-start justify-between gap-2">
                  <div>
                    <div className="flex items-center gap-2 font-medium">
                      <Plug className="h-3.5 w-3.5" />
                      <span>{":" + p.port}</span>
                      <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] uppercase">{p.protocol}</span>
                    </div>
                    <div className="mt-1 text-xs text-muted-foreground">{(p.label || t("platform.entryPorts")) + " · " + (p.platform_name || t("platform.unbound")) + "." + p.account}</div>
                  </div>
                  <button type="button" className="rounded p-1 text-muted-foreground hover:text-red-500" onClick={() => void handleRemovePort(p.port)} aria-label={t("common.delete")}>
                    <Trash2 className="h-4 w-4" />
                  </button>
                </div>
                {/* ADR-0021 Q1: per-port health chip + SOCKS5 credentials */}
                <div className="mt-2 flex items-center gap-2 text-[11px] text-muted-foreground" data-testid={"port-auth-" + p.port}>
                  {(() => {
                    const h = health[p.port];
                    const a = authInfo[p.port];
                    const healthIcon = h ? (h.reachable && h.reason === "ok" ? <ShieldCheck className="h-3 w-3 text-emerald-500" /> : h.protocol_mismatch ? <ShieldAlert className="h-3 w-3 text-amber-500" /> : !h.reachable ? <ShieldAlert className="h-3 w-3 text-red-500" /> : <ShieldCheck className="h-3 w-3 text-emerald-500" />) : null;
                    return (
                      <>
                        {healthIcon}
                        <span>{h ? (h.reachable && h.reason === "ok" ? t("platform.healthOk") : h.protocol_mismatch ? t("platform.healthProtocolMismatch") : t("platform.healthUnavailable")) : ""}</span>
                        {h && h.latency_ms > 0 && <span className="text-muted-foreground/70">· {h.latency_ms}ms</span>}
                        {a && (
                          <button
                            type="button"
                            className="ml-auto inline-flex items-center gap-1 rounded p-1 hover:text-primary"
                            onClick={() => copyCredentials(p.port, a)}
                            aria-label={t("platform.copyCredentials")}
                            title={t("platform.copyCredentials")}
                          >
                            <Copy className="h-3 w-3" />
                            {copiedPort === p.port ? t("platform.copied") : ""}
                          </button>
                        )}
                      </>
                    );
                  })()}
                </div>
                {authInfo[p.port] && p.protocol === "socks5" && authInfo[p.port].auth_required && (
                  <div className="mt-1 break-all text-[10px] text-muted-foreground/80">
                    {t("platform.socks5Auth")}: {authInfo[p.port].username} · {t("platform.passwordMasked")}
                  </div>
                )}
                {authInfo[p.port] && p.protocol === "http" && authInfo[p.port].auth_required && (
                  <div className="mt-1 break-all text-[10px] text-muted-foreground/80">
                    {t("platform.httpAuth")}: {authInfo[p.port].username} · {t("platform.passwordMasked")}
                  </div>
                )}
                {authInfo[p.port] && !authInfo[p.port].auth_required && (
                  <div className="mt-1 text-[10px] text-emerald-600 dark:text-emerald-400">
                    {t("platform.noAuthRequired")}
                  </div>
                )}
              </li>
            ))}
          </ul>
        </div>

        <div className="w-1.5 cursor-col-resize bg-border hover:bg-primary/40" onPointerDown={onSplitterPointerDown} onPointerMove={onSplitterPointerMove} onPointerUp={onSplitterPointerUp} data-testid="platforms-splitter" />

        <div className="flex min-h-0 flex-1 flex-col overflow-hidden" data-testid="platforms-pane" onPointerUp={() => { if (draggingPort != null) { document.body.style.userSelect = ""; setDraggingPort(null); setDragOverPlatform(null); } }}>
          <div className="border-b px-3 py-2 text-sm font-medium">{t("platform.activated")}</div>
          <ul className="min-h-0 flex-1 space-y-2 overflow-auto p-3">
            {platforms.length === 0 && <li className="text-xs text-muted-foreground">{t("platform.empty")}</li>}
            {platforms.map((p) => {
              const bound = ports.filter((x) => x.platform_name === p.name);
              const leases = leasesPerPlatform[p.name] ?? [];
              const entry = strategyConfig.platforms.find((s) => s.platform_name === p.name);
              const aClass = entry?.a_class ?? "manual";
              const regions = entry?.regions ?? [];
              const subs = entry?.subscriptions ?? [];
              const topN = entry?.top_n ?? 10;
              const manualNodes = (entry as Record<string, unknown> | undefined)?.manual_nodes as string[] | undefined ?? [];
              const distinctRegions = [...new Set(nodeList.map((n) => n.region).filter(Boolean))];
              const top3Preview = nodeList.slice(0, 3).map((n) => n.display_tag + "(" + n.region + ")").join(", ");
              return (
                <li key={p.name} className={"rounded-md border bg-card p-3 " + (dragOverPlatform === p.name ? "ring-2 ring-offset-2 ring-primary scale-[1.02] transition" : "")} onPointerEnter={() => { if (draggingPort != null) setDragOverPlatform(p.name); }} onPointerLeave={() => { if (dragOverPlatform === p.name) setDragOverPlatform(null); }} onPointerUp={() => { if (draggingPort != null) void bindPortToPlatform(draggingPort, p.name); }} data-testid={"platform-card-" + p.name}>
                  {/* T10-5: card top row ? name + leases/nodes pill + delete, NO B-class summary text */}
                  <div className="flex items-start justify-between gap-2">
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <span className="font-medium">{p.name}</span>
                      </div>
                      {bound.length > 0 && (
                        <div className="mt-2 flex flex-wrap gap-1">
                          {bound.map((b) => (
                            <span key={b.port} className="rounded bg-muted px-1.5 py-0.5 text-[11px]">{":" + b.port + "/" + b.protocol}</span>
                          ))}
                        </div>
                      )}
                    </div>
                    <div className="flex items-center gap-1.5">
                      {/* T10-5: leases/nodes pill in right-top */}
                      <span className="rounded-full bg-muted px-2 py-0.5 text-[10px] text-muted-foreground" data-testid={"platform-stats-pill-" + p.name}>
                        {t("platform.leases") + ": " + leases.length + " \u00b7 " + t("platform.routableNodes") + ": " + p.routableNodeCount}
                      </span>
                      <button type="button" className="rounded p-1 text-muted-foreground hover:text-red-500" onClick={() => void handleDeletePlatform(p.name)}>
                        <Trash2 className="h-4 w-4" />
                      </button>
                    </div>
                  </div>
                  {/* T10-1: A/B left-right split card layout, 50/50 width */}
                  <div className="mt-2.5 border-t pt-2 flex gap-3" data-testid={"strategy-split-" + p.name}>
                    {/* Left: A-class strategy (50%) */}
                    <div className="flex-1 min-w-0" data-testid={"strategy-aclass-pane-" + p.name}>
                      <div className="flex items-center justify-between gap-1 mb-1.5">
                        <span className="text-[10px] font-medium uppercase text-muted-foreground">{t("strategy.aClass")}</span>
                        <button type="button" disabled={strategyBusy} onClick={() => void handleApplyStrategy()} className="inline-flex items-center gap-1 rounded bg-primary px-2 py-0.5 text-[10px] text-primary-foreground disabled:opacity-50" data-testid="strategy-apply">
                          {strategyBusy ? <Loader2 className="h-3 w-3 animate-spin" /> : <CheckCircle2 className="h-3 w-3" />}
                          {t("strategy.apply")}
                        </button>
                      </div>
                      {/* A-class type selector as ToggleGroup chips */}
                      <div className="flex flex-wrap gap-1" data-testid={"strategy-aclass-chips-" + p.name}>
                        {["manual", "region", "quality", "subscription"].map((mode) => (
                          <button key={mode} type="button" disabled={strategyBusy}
                            className={"rounded px-2 py-0.5 text-[10px] border transition " + (aClass === mode ? "bg-primary text-primary-foreground border-primary" : "bg-background text-muted-foreground border-border hover:bg-muted")}
                            onClick={() => updateStrategyField(p.name, "a_class", mode)}
                            data-testid={"strategy-aclass-" + mode + "-" + p.name}>
                            {t("strategy." + mode)}
                          </button>
                        ))}
                      </div>
                      {/* T10-2: A-class mode-specific chip selectors */}
                      {aClass === "manual" && (
                        <div className="mt-1.5 flex flex-wrap gap-1 max-h-32 overflow-auto" data-testid={"strategy-manual-chips-" + p.name}>
                          {nodeList.length === 0 && <span className="text-[10px] text-muted-foreground">{t("strategy.manualSelect")}</span>}
                          {nodeList.map((n) => {
                            const selected = manualNodes.includes(n.node_hash);
                            return (
                              <button key={n.node_hash} type="button"
                                className={"rounded px-1.5 py-0.5 text-[10px] border transition " + (selected ? "bg-blue-500 text-white border-blue-500" : "bg-background text-muted-foreground border-border hover:bg-muted")}
                                onClick={() => {
                                  const newVal = selected ? manualNodes.filter((h) => h !== n.node_hash) : [...manualNodes, n.node_hash];
                                  updateStrategyField(p.name, "manual_nodes" as keyof PlatformStrategy, newVal as unknown as string);
                                }}
                                data-testid={"strategy-manual-chip-" + n.node_hash}>
                                {n.display_tag}
                              </button>
                            );
                          })}
                        </div>
                      )}
                      {aClass === "region" && (
                        <div className="mt-1.5 flex flex-wrap gap-1" data-testid={"strategy-region-chips-" + p.name}>
                          {distinctRegions.length === 0 && <span className="text-[10px] text-muted-foreground">{t("strategy.regionSelect")}</span>}
                          {distinctRegions.map((r) => {
                            const selected = regions.includes(r);
                            return (
                              <button key={r} type="button"
                                className={"rounded px-1.5 py-0.5 text-[10px] border transition " + (selected ? "bg-blue-500 text-white border-blue-500" : "bg-background text-muted-foreground border-border hover:bg-muted")}
                                onClick={() => {
                                  const newVal = selected ? regions.filter((x) => x !== r) : [...regions, r];
                                  updateStrategyField(p.name, "regions", newVal);
                                }}
                                data-testid={"strategy-region-chip-" + r}>
                                {r}
                              </button>
                            );
                          })}
                        </div>
                      )}
                      {aClass === "subscription" && (
                        <div className="mt-1.5 flex flex-wrap gap-1" data-testid={"strategy-subs-chips-" + p.name}>
                          {subList.length === 0 && <span className="text-[10px] text-muted-foreground">{t("strategy.subscriptionSelect")}</span>}
                          {subList.map((s) => {
                            const selected = subs.includes(s.name);
                            return (
                              <button key={s.name} type="button"
                                className={"rounded px-1.5 py-0.5 text-[10px] border transition " + (selected ? "bg-primary text-primary-foreground border-primary" : "bg-background text-muted-foreground border-border hover:bg-muted")}
                                onClick={() => {
                                  const newVal = selected ? subs.filter((x) => x !== s.name) : [...subs, s.name];
                                  updateStrategyField(p.name, "subscriptions", newVal);
                                }}
                                data-testid={"strategy-sub-chip-" + s.name}>
                                {s.name + " (" + s.node_count + ")"}
                              </button>
                            );
                          })}
                        </div>
                      )}
                      {aClass === "quality" && (
                        <div className="mt-1.5 space-y-1" data-testid={"strategy-quality-pane-" + p.name}>
                          <label className="flex items-center gap-1.5">
                            <span className="text-[10px] text-muted-foreground">{t("strategy.topN")}</span>
                            <input type="number" min={1} max={1000} className="w-20 rounded border bg-background px-1.5 py-1 text-[11px]" value={strategyTopNInput[p.name] ?? String(topN)} onChange={(e) => { setStrategyTopNInput((s) => ({ ...s, [p.name]: e.target.value })); updateStrategyField(p.name, "top_n", e.target.value); }} data-testid={"strategy-topn-" + p.name} />
                          </label>
                          {top3Preview && (
                            <div className="text-[10px] text-muted-foreground" data-testid={"strategy-quality-preview-" + p.name}>
                              {t("strategy.qualityPreview") + ": " + top3Preview}
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                    {/* Right: B-class strategy (50%) */}
                    <div className="flex-1 min-w-0" data-testid={"strategy-bclass-pane-" + p.name}>
                      <span className="text-[10px] font-medium uppercase text-muted-foreground block mb-1.5">{t("strategy.bClass")}</span>
                      {/* T10-1: B-class as ToggleGroup single-select chips */}
                      <div className="flex flex-wrap gap-1" data-testid={"strategy-bclass-chips-" + p.name}>
                        {STRATEGY_IDS.map((s) => (
                          <button key={s} type="button"
                            className={"rounded px-2 py-0.5 text-[10px] border transition " + (p.allocationPolicy === s ? "bg-primary text-primary-foreground border-primary" : "bg-background text-muted-foreground border-border hover:bg-muted")}
                            onClick={() => { void ipcPlatformUpdate(p.name, s).then(() => refreshPlatforms()).catch((e2) => showToast("err", translateError(e2, t))); }}
                            data-testid={"strategy-bclass-" + s + "-" + p.name}>
                            {t(strategyToI18nKey(s))}
                          </button>
                        ))}
                      </div>
                    </div>
                  </div>
                </li>
              );
            })}
          </ul>
        </div>
      </div>

      {createDialogOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
          <div className="w-full max-w-md rounded-lg border bg-background p-4 shadow-xl" data-testid="platform-create-dialog">
            <h2 className="text-base font-semibold">{t("platform.createTitle")}</h2>
            <div className="mt-3 space-y-2">
              <input className="w-full rounded border bg-background px-2 py-1.5 text-sm" value={createName} onChange={(e) => setCreateName(e.target.value)} placeholder={t("platform.name")} data-testid="platform-create-name" />
              <select className="w-full rounded border bg-background px-2 py-1.5 text-sm" value={createPolicy} onChange={(e) => setCreatePolicy(e.target.value as StrategyId)}>
                {STRATEGY_IDS.map((s) => (<option key={s} value={s}>{t(strategyToI18nKey(s))}</option>))}
              </select>
              {createFormError && <p className="text-xs text-red-500">{createFormError}</p>}
            </div>
            <div className="mt-4 flex justify-end gap-2">
              <button type="button" className="rounded border px-3 py-1.5 text-sm" onClick={() => setCreateDialogOpen(false)}>{t("common.cancel")}</button>
              <button type="button" className="rounded bg-primary px-3 py-1.5 text-sm text-primary-foreground" disabled={busy} onClick={() => void handleCreatePlatform()} data-testid="platform-create-submit">{t("platform.createSubmit")}</button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
