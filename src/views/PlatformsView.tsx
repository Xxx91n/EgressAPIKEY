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
  ipcPortRemove,
  ipcPortAuthInfo,
  ipcPortHealthCheck,
  ALLOCATION_POLICIES,
  type AllocationPolicy,
  type PortMapping,
  type PortAuthInfo,
  type PortHealthCheck,
} from "../lib/ipc";
import { policyToI18nKey } from "../lib/policy";
import { loadSplitRatio, saveSplitRatio } from "../lib/settings";

/** Phase 5 / ADR-0012: left = Entry Ports, right = Platforms. Port = identity. */
interface PlatformInfoFull {
  name: string;
  allocationPolicy: string;
  regexFilters: string[];
  regionFilters: string[];
  routableNodeCount: number;
  stickyTtl: string;
}

export function PlatformsView() {
  const { t } = useTranslation();
  const removePlatform = useAppStore((s) => s.removePlatform);
  const [ports, setPorts] = useState<PortMapping[]>([]);
  const [newPort, setNewPort] = useState("17990");
  const [newProto, setNewProto] = useState<"socks5" | "http">("socks5");
  const [newLabel, setNewLabel] = useState("");
  const [newPlatformName, setNewPlatformName] = useState("Default");
  const [platforms, setPlatforms] = useState<PlatformInfoFull[]>([]);
  const [leasesPerPlatform, setLeasesPerPlatform] = useState<Record<string, unknown[]>>({});
  const [draggingPort, setDraggingPort] = useState<number | null>(null);
  const [dragOverPlatform, setDragOverPlatform] = useState<string | null>(null);
  const [splitRatio, setSplitRatio] = useState(0.4);
  const [toast, setToast] = useState<{ kind: "ok" | "err"; msg: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [createName, setCreateName] = useState("");
  const [createPolicy, setCreatePolicy] = useState<AllocationPolicy>("BALANCED");
  const [createFormError, setCreateFormError] = useState<string | null>(null);
  /// ADR-0021 Q1: per-port auth info cache (port -> credentials displayed inline).
  const [authInfo, setAuthInfo] = useState<Record<number, PortAuthInfo>>({});
  /// ADR-0021 Q1: per-port health probe result (port -> chip color + reason).
  const [health, setHealth] = useState<Record<number, PortHealthCheck>>({});
  const [copiedPort, setCopiedPort] = useState<number | null>(null);

  // T4-4: strategy panel state.
  const [strategyConfig, setStrategyConfig] = useState<StrategyConfig>({ version: 1, platforms: [] });
  const [strategyBusy, setStrategyBusy] = useState(false);
  const [strategyRegionsInput, setStrategyRegionsInput] = useState<Record<string, string>>({});
  const [strategySubsInput, setStrategySubsInput] = useState<Record<string, string>>({});
  const [strategyTopNInput, setStrategyTopNInput] = useState<Record<string, string>>({});

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

  const handleAddRegion = (platformName: string) => {
    const input = (strategyRegionsInput[platformName] ?? "").trim();
    if (!input) return;
    const regions = input.split(",").map((r) => r.trim().toLowerCase()).filter(Boolean);
    const existing = strategyConfig.platforms.find((p) => p.platform_name === platformName);
    const current = existing?.regions ?? [];
    const merged = [...new Set([...current, ...regions])];
    updateStrategyField(platformName, "regions", merged);
    setStrategyRegionsInput((s) => ({ ...s, [platformName]: "" }));
  };

  const handleAddSub = (platformName: string) => {
    const input = (strategySubsInput[platformName] ?? "").trim();
    if (!input) return;
    const subs = input.split(",").map((s) => s.trim()).filter(Boolean);
    const existing = strategyConfig.platforms.find((p) => p.platform_name === platformName);
    const current = existing?.subscriptions ?? [];
    const merged = [...new Set([...current, ...subs])];
    updateStrategyField(platformName, "subscriptions", merged);
    setStrategySubsInput((s) => ({ ...s, [platformName]: "" }));
  };

  const removeRegion = (platformName: string, region: string) => {
    const existing = strategyConfig.platforms.find((p) => p.platform_name === platformName);
    const current = existing?.regions ?? [];
    updateStrategyField(platformName, "regions", current.filter((r) => r !== region));
  };

  const removeSub = (platformName: string, sub: string) => {
    const existing = strategyConfig.platforms.find((p) => p.platform_name === platformName);
    const current = existing?.subscriptions ?? [];
    updateStrategyField(platformName, "subscriptions", current.filter((s) => s !== sub));
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
        allocationPolicy: String(p.allocation_policy ?? "BALANCED"),
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
        try { return [p.port, await ipcPortHealthCheck(p.port)] as const; } catch { return null; }
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
    loadSplitRatio().then((r) => { if (typeof r === "number" && r > 0.15 && r < 0.85) setSplitRatio(r); }).catch(() => {});
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
        platform_name: newPlatformName.trim() || "Default",
        account: "port-" + port,
        label: newLabel.trim() || ("entry-" + port),
        enabled: true,
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
      await ipcPortUpsert({
        port: row.port,
        protocol: row.protocol,
        platform_name: platformName,
        account: row.account || ("port-" + row.port),
        label: row.label,
        enabled: row.enabled,
      });
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
      await ipcPlatformCreateWithFields({ name, allocation_policy: createPolicy });
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
            <button type="button" disabled={busy} onClick={() => void handleAddPort()} className="inline-flex w-full items-center justify-center gap-1 rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground disabled:opacity-50" data-testid="port-add">
              {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Plus className="h-4 w-4" />}
              {t("platform.addPort")}
            </button>
          </div>
          <ul className="min-h-0 flex-1 space-y-2 overflow-auto p-3">
            {ports.length === 0 && <li className="text-xs text-muted-foreground">{t("platform.noPorts")}</li>}
            {ports.map((p) => (
              <li key={p.port} className={"cursor-grab rounded-md border bg-card p-3 text-sm " + (draggingPort === p.port ? "opacity-60" : "")} onPointerDown={() => setDraggingPort(p.port)} data-testid={"port-row-" + p.port}>
                <div className="flex items-start justify-between gap-2">
                  <div>
                    <div className="flex items-center gap-2 font-medium">
                      <Plug className="h-3.5 w-3.5" />
                      <span>{":" + p.port}</span>
                      <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] uppercase">{p.protocol}</span>
                    </div>
                    <div className="mt-1 text-xs text-muted-foreground">{(p.label || t("platform.entryPorts")) + " · " + p.platform_name + "." + p.account}</div>
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
                    const healthIcon = h ? (h.socks5_ok ? <ShieldCheck className="h-3 w-3 text-emerald-500" /> : h.protocol_mismatch ? <ShieldAlert className="h-3 w-3 text-amber-500" /> : <ShieldAlert className="h-3 w-3 text-red-500" />) : null;
                    return (
                      <>
                        {healthIcon}
                        <span>{h ? (h.socks5_ok ? t("platform.healthOk") : h.protocol_mismatch ? t("platform.healthProtocolMismatch") : t("platform.healthUnavailable")) : ""}</span>
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
                {authInfo[p.port] && (
                  <div className="mt-1 break-all text-[10px] text-muted-foreground/80">
                    {t("platform.socks5Auth")}: {authInfo[p.port].username} · {t("platform.passwordMasked")}
                  </div>
                )}
              </li>
            ))}
          </ul>
        </div>

        <div className="w-1.5 cursor-col-resize bg-border hover:bg-primary/40" onPointerDown={onSplitterPointerDown} onPointerMove={onSplitterPointerMove} onPointerUp={onSplitterPointerUp} data-testid="platforms-splitter" />

        <div className="flex min-h-0 flex-1 flex-col overflow-hidden" data-testid="platforms-pane" onPointerUp={() => { if (draggingPort != null && !dragOverPlatform) setDraggingPort(null); }}>
          <div className="border-b px-3 py-2 text-sm font-medium">{t("platform.activated")}</div>
          <ul className="min-h-0 flex-1 space-y-2 overflow-auto p-3">
            {platforms.length === 0 && <li className="text-xs text-muted-foreground">{t("platform.empty")}</li>}
            {platforms.map((p) => {
              const bound = ports.filter((x) => x.platform_name === p.name);
              const leases = leasesPerPlatform[p.name] ?? [];
              return (
                <li key={p.name} className={"rounded-md border bg-card p-3 " + (dragOverPlatform === p.name ? "ring-2 ring-primary" : "")} onPointerEnter={() => { if (draggingPort != null) setDragOverPlatform(p.name); }} onPointerLeave={() => { if (dragOverPlatform === p.name) setDragOverPlatform(null); }} onPointerUp={() => { if (draggingPort != null) void bindPortToPlatform(draggingPort, p.name); }} data-testid={"platform-card-" + p.name}>
                  <div className="flex items-start justify-between gap-2">
                    <div>
                      <div className="font-medium">{p.name}</div>
                      <div className="mt-1 text-xs text-muted-foreground">{t(policyToI18nKey(p.allocationPolicy)) + " · " + t("platform.leases") + ": " + leases.length + " · " + t("platform.routableNodes") + ": " + p.routableNodeCount}</div>
                      {bound.length > 0 && (
                        <div className="mt-2 flex flex-wrap gap-1">
                          {bound.map((b) => (
                            <span key={b.port} className="rounded bg-muted px-1.5 py-0.5 text-[11px]">{":" + b.port + "/" + b.protocol}</span>
                          ))}
                        </div>
                      )}
                    </div>
                    <div className="flex items-center gap-1">
                      <select className="rounded border bg-background px-1 py-0.5 text-[11px]" value={p.allocationPolicy} onChange={(e) => { const policy = e.target.value as AllocationPolicy; void ipcPlatformUpdate(p.name, policy).then(() => refreshPlatforms()).catch((err) => showToast("err", err instanceof Error ? err.message : String(err))); }}>
                        {ALLOCATION_POLICIES.map((pol) => (<option key={pol} value={pol}>{t(policyToI18nKey(pol))}</option>))}
                      </select>
                      <button type="button" className="rounded p-1 text-muted-foreground hover:text-red-500" onClick={() => void handleDeletePlatform(p.name)}>
                        <Trash2 className="h-4 w-4" />
                      </button>
                    </div>
                  </div>
                </li>
              );
            })}
          </ul>

        {/* T4-4: Strategy panel */}
        <div className="border-t px-3 py-2" data-testid="strategy-panel">
          <div className="flex items-center justify-between gap-2">
            <span className="text-sm font-medium">{t("strategy.title")}</span>
            <button type="button" disabled={strategyBusy} onClick={() => void handleApplyStrategy()} className="inline-flex items-center gap-1 rounded-md bg-primary px-2.5 py-1 text-xs text-primary-foreground disabled:opacity-50" data-testid="strategy-apply">
              {strategyBusy ? <Loader2 className="h-3 w-3 animate-spin" /> : <CheckCircle2 className="h-3 w-3" />}
              {t("strategy.apply")}
            </button>
          </div>

          <div className="mt-2 space-y-2 max-h-40 overflow-auto">
            {platforms.length === 0 && <p className="text-xs text-muted-foreground">{t("strategy.noPlatforms")}</p>}
            {platforms.map((p) => {
              const entry = strategyConfig.platforms.find((s) => s.platform_name === p.name);
              const aClass = entry?.a_class ?? "manual";
              const bClass = entry?.b_class ?? "random";
              const regions = entry?.regions ?? [];
              const subs = entry?.subscriptions ?? [];
              const topN = entry?.top_n ?? 10;
              return (
                <div key={"strategy-" + p.name} className="rounded-md border bg-muted/30 p-2 text-xs" data-testid={"strategy-row-" + p.name}>
                  <div className="mb-1.5 flex items-center gap-2">
                    <span className="font-medium">{p.name}</span>
                  </div>
                  <div className="grid grid-cols-2 gap-1.5">
                    <label className="flex flex-col gap-0.5">
                      <span className="text-[10px] text-muted-foreground">{t("strategy.aClass")}</span>
                      <select className="rounded border bg-background px-1.5 py-1 text-[11px]" value={aClass} onChange={(e) => updateStrategyField(p.name, "a_class", e.target.value)} data-testid={"strategy-aclass-" + p.name}>
                        <option value="manual">{t("strategy.manual")}</option>
                        <option value="region">{t("strategy.region")}</option>
                        <option value="quality">{t("strategy.quality")}</option>
                        <option value="subscription">{t("strategy.subscription")}</option>
                      </select>
                    </label>
                    <label className="flex flex-col gap-0.5">
                      <span className="text-[10px] text-muted-foreground">{t("strategy.bClass")}</span>
                      <select className="rounded border bg-background px-1.5 py-1 text-[11px]" value={bClass} onChange={(e) => updateStrategyField(p.name, "b_class", e.target.value)} data-testid={"strategy-bclass-" + p.name}>
                        <option value="balanced">{t("strategy.balanced")}</option>
                        <option value="prefer_low_latency">{t("strategy.preferLowLatency")}</option>
                        <option value="prefer_idle_ip">{t("strategy.preferIdleIp")}</option>
                      </select>
                    </label>
                  </div>

                  {aClass === "region" && (
                    <div className="mt-1.5">
                      <div className="flex gap-1">
                        <input className="flex-1 rounded border bg-background px-1.5 py-1 text-[11px]" value={strategyRegionsInput[p.name] ?? ""} onChange={(e) => setStrategyRegionsInput((s) => ({ ...s, [p.name]: e.target.value }))} placeholder={t("strategy.regionsHint")} onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); handleAddRegion(p.name); } }} data-testid={"strategy-regions-input-" + p.name} />
                        <button type="button" className="rounded border px-1.5 py-1 text-[11px]" onClick={() => handleAddRegion(p.name)}>+</button>
                      </div>
                      {regions.length > 0 && (
                        <div className="mt-1 flex flex-wrap gap-1">
                          {regions.map((r) => (
                            <button key={r} type="button" className="inline-flex items-center gap-0.5 rounded bg-muted px-1.5 py-0.5 text-[10px]" onClick={() => removeRegion(p.name, r)}>
                              {r} <span className="text-red-500">x</span>
                            </button>
                          ))}
                        </div>
                      )}
                    </div>
                  )}

                  {aClass === "subscription" && (
                    <div className="mt-1.5">
                      <div className="flex gap-1">
                        <input className="flex-1 rounded border bg-background px-1.5 py-1 text-[11px]" value={strategySubsInput[p.name] ?? ""} onChange={(e) => setStrategySubsInput((s) => ({ ...s, [p.name]: e.target.value }))} placeholder={t("strategy.subsHint")} onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); handleAddSub(p.name); } }} data-testid={"strategy-subs-input-" + p.name} />
                        <button type="button" className="rounded border px-1.5 py-1 text-[11px]" onClick={() => handleAddSub(p.name)}>+</button>
                      </div>
                      {subs.length > 0 && (
                        <div className="mt-1 flex flex-wrap gap-1">
                          {subs.map((s) => (
                            <button key={s} type="button" className="inline-flex items-center gap-0.5 rounded bg-muted px-1.5 py-0.5 text-[10px]" onClick={() => removeSub(p.name, s)}>
                              {s} <span className="text-red-500">x</span>
                            </button>
                          ))}
                        </div>
                      )}
                    </div>
                  )}

                  {aClass === "quality" && (
                    <label className="mt-1.5 flex items-center gap-1.5">
                      <span className="text-[10px] text-muted-foreground">{t("strategy.topN")}</span>
                      <input type="number" min={1} max={1000} className="w-20 rounded border bg-background px-1.5 py-1 text-[11px]" value={strategyTopNInput[p.name] ?? String(topN)} onChange={(e) => { setStrategyTopNInput((s) => ({ ...s, [p.name]: e.target.value })); updateStrategyField(p.name, "top_n", e.target.value); }} data-testid={"strategy-topn-" + p.name} />
                    </label>
                  )}
                </div>
              );
            })}
          </div>
        </div>
        </div>
      </div>

      {createDialogOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
          <div className="w-full max-w-md rounded-lg border bg-background p-4 shadow-xl" data-testid="platform-create-dialog">
            <h2 className="text-base font-semibold">{t("platform.createTitle")}</h2>
            <div className="mt-3 space-y-2">
              <input className="w-full rounded border bg-background px-2 py-1.5 text-sm" value={createName} onChange={(e) => setCreateName(e.target.value)} placeholder={t("platform.name")} data-testid="platform-create-name" />
              <select className="w-full rounded border bg-background px-2 py-1.5 text-sm" value={createPolicy} onChange={(e) => setCreatePolicy(e.target.value as AllocationPolicy)}>
                {ALLOCATION_POLICIES.map((pol) => (<option key={pol} value={pol}>{t(policyToI18nKey(pol))}</option>))}
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
