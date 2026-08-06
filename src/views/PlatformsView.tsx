import { useEffect, useState, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Trash2, Loader2, AlertCircle, CheckCircle2, Plug } from "lucide-react";
import { useAppStore } from "../store/appStore";
import {
  ipcPlatformRemove,
  ipcPlatformListFull,
  ipcPlatformLeases,
  ipcPlatformCreateWithFields,
  ipcPlatformUpdate,
  ipcPortList,
  ipcPortUpsert,
  ipcPortRemove,
  ALLOCATION_POLICIES,
  type AllocationPolicy,
  type PortMapping,
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
  const containerRef = useRef<HTMLDivElement>(null);
  const resizingRef = useRef(false);

  const refreshPorts = useCallback(async () => {
    try { setPorts(await ipcPortList()); } catch { /* outside Tauri */ }
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

  useEffect(() => {
    loadSplitRatio().then((r) => { if (typeof r === "number" && r > 0.15 && r < 0.85) setSplitRatio(r); }).catch(() => {});
    void refreshPorts();
    void refreshPlatforms();
  }, [refreshPorts, refreshPlatforms]);

  const showToast = (kind: "ok" | "err", msg: string) => {
    setToast({ kind, msg });
    window.setTimeout(() => setToast(null), 3500);
  };

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
      await refreshPorts();
    } catch (e) { showToast("err", e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  };

  const handleRemovePort = async (port: number) => {
    setBusy(true);
    try { await ipcPortRemove(port); showToast("ok", t("platform.portRemoved")); await refreshPorts(); }
    catch (e) { showToast("err", e instanceof Error ? e.message : String(e)); }
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
      await refreshPorts();
    } catch (e) { showToast("err", e instanceof Error ? e.message : String(e)); }
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
    } catch (e) { setCreateFormError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  };

  const handleDeletePlatform = async (name: string) => {
    setBusy(true);
    try { await ipcPlatformRemove(name); removePlatform(name); showToast("ok", t("platform.deleteConfirm")); await refreshPlatforms(); }
    catch (e) { showToast("err", e instanceof Error ? e.message : String(e)); }
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
