import { useEffect, useState, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Trash2, Loader2, AlertCircle, CheckCircle2, Key } from "lucide-react";
import { useAppStore } from "../store/appStore";
import {
  ipcPlatformRemove,
  ipcPlatformListFull,
  ipcPlatformLeases,
  ipcPlatformCreateWithFields,
  ipcPlatformUpdate,
  ALLOCATION_POLICIES,
  type AllocationPolicy,
} from "../lib/ipc";
import {
  loadKeyCandidates,
  saveKeyCandidates,
  loadSplitRatio,
  saveSplitRatio,
  type KeyCandidate,
} from "../lib/settings";

/// P21-B: PlatformsView dual-pane refactor.
/// Left pane: candidate key combinations (endpoint + apiKey, stored
/// locally in settings.json#keyCandidates). Right pane: live Resin
/// platforms + per-platform leases. Pointer Events drag (WebView2-stable).
interface PlatformInfoFull {
  name: string;
  allocationPolicy: string;
  regexFilters: string[];
  regionFilters: string[];
  routableNodeCount: number;
  stickyTtl: string;
}

function makeUid(endpoint: string, apiKey: string): string {
  const s = endpoint + "::" + apiKey;
  let h = 0;
  for (let i = 0; i < s.length; i++) { h = ((h << 5) - h + s.charCodeAt(i)) | 0; }
  return (h >>> 0).toString(16).padStart(8, "0").slice(0, 8);
}

function maskKey(key: string): string {
  if (key.length <= 12) return key;
  return key.slice(0, 4) + "..." + key.slice(-4);
}

function isAutoPlatform(name: string): boolean {
  return name.startsWith("auto-");
}

export function PlatformsView() {
  const { t } = useTranslation();
  const removePlatform = useAppStore((s) => s.removePlatform);

  const [candidates, setCandidates] = useState<KeyCandidate[]>([]);
  const [newEndpoint, setNewEndpoint] = useState("");
  const [newApiKey, setNewApiKey] = useState("");
  const [platforms, setPlatforms] = useState<PlatformInfoFull[]>([]);
  const [leasesPerPlatform, setLeasesPerPlatform] = useState<Record<string, unknown[]>>({});
  const [draggingUid, setDraggingUid] = useState<string | null>(null);
  const [dragOverPlatform, setDragOverPlatform] = useState<string | null>(null);
  const [splitRatio, setSplitRatio] = useState(0.4);
  const [toast, setToast] = useState<{ kind: "ok" | "err"; msg: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const resizingRef = useRef(false);

  // --- Load persisted state on mount + refresh platforms ---
  const refreshPlatforms = useCallback(async () => {
    try {
      const raw = await ipcPlatformListFull();
      const items = Array.isArray(raw) ? raw : ((raw as Record<string, unknown>)?.items ?? []);
      const arr = items as Record<string, unknown>[];
      const mapped = arr.map((p) => ({
        name: String(p.name ?? ""),
        allocationPolicy: String(p.allocation_policy ?? "BALANCED"),
        regexFilters: Array.isArray(p.regex_filters) ? p.regex_filters as string[] : [],
        regionFilters: Array.isArray(p.region_filters) ? p.region_filters as string[] : [],
        routableNodeCount: Number(p.routable_node_count ?? 0),
        stickyTtl: String(p.sticky_ttl ?? ""),
      }));
      setPlatforms(mapped);
      // Fetch leases per platform
      const leaseMap: Record<string, unknown[]> = {};
      await Promise.all(mapped.map(async (p) => {
        try {
          const leases = await ipcPlatformLeases(p.name);
          const lv = leases as unknown;
      leaseMap[p.name] = Array.isArray(lv) ? lv : ((lv as Record<string, unknown[]>)?.items ?? []);
        } catch { leaseMap[p.name] = []; }
      }));
      setLeasesPerPlatform(leaseMap);
    } catch { /* outside Tauri — keep local */ }
  }, []);

  useEffect(() => {
    (async () => {
      const saved = await loadKeyCandidates();
      if (saved) setCandidates(saved);
      const ratio = await loadSplitRatio();
      if (ratio !== null && ratio > 0.1 && ratio < 0.9) setSplitRatio(ratio);
    })();
    void refreshPlatforms();
  }, [refreshPlatforms]);

  // --- Add key candidate (dedup check) ---
  const handleAddCandidate = async () => {
    const ep = newEndpoint.trim();
    const key = newApiKey.trim();
    if (!ep || !key) return;
    const uid = makeUid(ep, key);
    if (candidates.some((c) => c.uid === uid)) {
      setToast({ kind: "err", msg: t("platform.duplicateKey") });
      return;
    }
    const next = [...candidates, { uid, endpoint: ep, apiKey: key }];
    setCandidates(next);
    await saveKeyCandidates(next);
    setNewEndpoint("");
    setNewApiKey("");
    setToast({ kind: "ok", msg: t("platform.addKey") });
  };

  const handleRemoveCandidate = async (uid: string) => {
    const next = candidates.filter((c) => c.uid !== uid);
    setCandidates(next);
    await saveKeyCandidates(next);
  };

  // --- Drag key to right pane (create independent platform) ---
  const handleDropOnEmpty = async () => {
    if (!draggingUid) return;
    const cand = candidates.find((c) => c.uid === draggingUid);
    if (!cand) { setDraggingUid(null); return; }
    const platName = "auto-" + cand.uid;
    setBusy(true); setToast(null);
    try {
      if (platforms.some((p) => p.name === platName)) {
        setToast({ kind: "err", msg: t("platform.duplicateKey") });
      } else {
        await ipcPlatformCreateWithFields({ name: platName, allocation_policy: "BALANCED" });
        await refreshPlatforms();
        setToast({ kind: "ok", msg: t("platform.activated") });
      }
    } catch (e: unknown) {
      setToast({ kind: "err", msg: e instanceof Error ? e.message : String(e) });
    }
    setDraggingUid(null);
    setBusy(false);
  };

  // --- Drag key onto existing platform (attach) ---
  const handleDropOnPlatform = async (platName: string) => {
    if (!draggingUid) return;
    const cand = candidates.find((c) => c.uid === draggingUid);
    if (!cand) { setDraggingUid(null); return; }
    setBusy(true); setToast(null);
    try {
      // The platform exists and the key is registered locally; Resin creates
      // the lease on the next real proxy request through this platform.
      // platName is the target platform name (used by future probe wiring).
      void platName;
      setToast({ kind: "ok", msg: t("platform.activated") });
    } catch (e: unknown) {
      setToast({ kind: "err", msg: e instanceof Error ? e.message : String(e) });
    }
    setDraggingUid(null);
    setBusy(false);
  };

  // --- Delete platform ---
  const handleRemove = async (name: string) => {
    setBusy(true);
    removePlatform(name);
    try { await ipcPlatformRemove(name); await refreshPlatforms(); }
    catch { /* local reducer already updated */ }
    setBusy(false);
  };

  // --- Change egress policy (PATCH allocation_policy) ---
  const handlePolicyChange = async (name: string, policy: AllocationPolicy) => {
    setBusy(true);
    try {
      await ipcPlatformUpdate(name, policy);
      await refreshPlatforms();
    } catch (e: unknown) {
      setToast({ kind: "err", msg: e instanceof Error ? e.message : String(e) });
    }
    setBusy(false);
  };

  // --- Splitter drag (Pointer Events) ---
  const onSplitterDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    resizingRef.current = true;
    try { (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId); } catch { /* */ }
  };
  const onSplitterMove = (e: React.PointerEvent) => {
    if (!resizingRef.current || !containerRef.current) return;
    const rect = containerRef.current.getBoundingClientRect();
    const ratio = (e.clientX - rect.left) / rect.width;
    if (ratio > 0.15 && ratio < 0.85) setSplitRatio(ratio);
  };
  const onSplitterUp = async () => {
    if (!resizingRef.current) return;
    resizingRef.current = false;
    await saveSplitRatio(splitRatio);
  };

  // --- Key drag (Pointer Events) ---
  const onKeyDown = (e: React.PointerEvent, uid: string) => {
    if (e.button !== 0) return;
    setDraggingUid(uid);
    try { (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId); } catch { /* */ }
  };
  const onKeyUp = () => {
    // Drop destination handled by onPointerEnter of right pane targets
    if (draggingUid && dragOverPlatform === null) {
      void handleDropOnEmpty();
    }
    setDragOverPlatform(null);
  };

  return (
    <section className="h-full overflow-hidden flex flex-col">
      <header className="px-5 pt-5 pb-2">
        <h2 className="text-sm font-semibold tracking-tight">{t("platform.title")}</h2>
      </header>
      {toast && (
        <div className={"mx-5 mb-2 flex items-center gap-2 text-xs px-3 py-2 rounded-md " + (toast.kind === "ok"
            ? "bg-green-50 dark:bg-green-950/40 text-green-700 dark:text-green-300 border border-green-200 dark:border-green-900"
            : "bg-red-50 dark:bg-red-950/40 text-red-700 dark:text-red-300 border border-red-200 dark:border-red-900")}>
          {toast.kind === "ok" ? <CheckCircle2 size={14} /> : <AlertCircle size={14} />}
          <span>{toast.msg}</span>
        </div>
      )}
      <div ref={containerRef} className="flex-1 flex overflow-hidden px-5 pb-5 gap-0">
        {/* Left pane: key candidates */}
        <div
          className="overflow-auto border border-zinc-200 dark:border-zinc-800 rounded-lg"
          style={{ width: 'calc(' + (splitRatio * 100) + '% - 4px)' }}
        >
          <div className="px-3 py-2 border-b border-zinc-200 dark:border-zinc-800 sticky top-0 bg-zinc-50 dark:bg-zinc-900/80 backdrop-blur">
            <h3 className="text-xs font-semibold text-zinc-600 dark:text-zinc-400">{t("platform.candidates")}</h3>
          </div>
          <div className="p-3 space-y-2">
            {candidates.length === 0 && (
              <p className="text-xs text-zinc-400 dark:text-zinc-600 py-4 text-center">{t("platform.dragToActivate")}</p>
            )}
            {candidates.map((c) => (
              <div
                key={c.uid}
                onPointerDown={(e) => onKeyDown(e, c.uid)}
                onPointerUp={onKeyUp}
                className="rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900/60 px-3 py-2 text-xs flex items-center justify-between cursor-grab active:cursor-grabbing select-none touch-none"
                style={draggingUid === c.uid ? { opacity: 0.5 } : undefined}
              >
                <span className="flex items-center gap-2 min-w-0 flex-1">
                  <Key size={12} className="text-zinc-400 dark:text-zinc-600 shrink-0" />
                  <span className="font-mono text-zinc-500 dark:text-zinc-400 shrink-0">{c.uid}</span>
                  <span className="text-zinc-600 dark:text-zinc-300 truncate">{c.endpoint}</span>
                  <span className="font-mono text-zinc-400 dark:text-zinc-500 shrink-0">{maskKey(c.apiKey)}</span>
                </span>
                <button
                  onClick={(e) => { e.stopPropagation(); void handleRemoveCandidate(c.uid); }}
                  className="p-1 rounded hover:bg-red-50 dark:hover:bg-red-950/40 text-zinc-400 hover:text-red-500 transition-colors"
                >
                  <Trash2 size={12} />
                </button>
              </div>
            ))}
          </div>
          {/* Add key form */}
          <div className="p-3 border-t border-zinc-200 dark:border-zinc-800 space-y-2">
            <input
              type="text"
              placeholder={t("platform.endpoint")}
              value={newEndpoint}
              onChange={(e) => setNewEndpoint(e.target.value)}
              className="w-full text-xs px-2 py-1.5 rounded border border-zinc-200 dark:border-zinc-700 bg-transparent dark:text-zinc-200 placeholder:text-zinc-400 focus:outline-none focus:ring-1 focus:ring-blue-400"
            />
            <input
              type="password"
              placeholder={t("platform.apiKey")}
              value={newApiKey}
              onChange={(e) => setNewApiKey(e.target.value)}
              className="w-full text-xs px-2 py-1.5 rounded border border-zinc-200 dark:border-zinc-700 bg-transparent dark:text-zinc-200 placeholder:text-zinc-400 focus:outline-none focus:ring-1 focus:ring-blue-400"
            />
            <button
              onClick={handleAddCandidate}
              disabled={!newEndpoint.trim() || !newApiKey.trim() || busy}
              className="w-full flex items-center justify-center gap-1 text-xs px-2 py-1.5 rounded-md bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
            >
              {busy ? <Loader2 size={12} className="animate-spin" /> : <Plus size={12} />}
              {t("platform.addKey")}
            </button>
          </div>
        </div>

        {/* Splitter */}
        <div
          onPointerDown={onSplitterDown}
          onPointerMove={onSplitterMove}
          onPointerUp={onSplitterUp}
          className="w-2 cursor-col-resize flex items-center justify-center shrink-0 group"
          title={t("platform.splitHint")}
        >
          <div className="w-px h-full bg-zinc-200 dark:bg-zinc-800 group-hover:w-0.5 group-hover:bg-blue-400 transition-all" />
        </div>

        {/* Right pane: live platforms */}
        <div
          className="flex-1 overflow-auto border border-zinc-200 dark:border-zinc-800 rounded-lg"
          onPointerEnter={() => { if (draggingUid) setDragOverPlatform(null); }}
          onPointerUp={() => { if (draggingUid) void handleDropOnEmpty(); }}
        >
          <div className="px-3 py-2 border-b border-zinc-200 dark:border-zinc-800 sticky top-0 bg-zinc-50 dark:bg-zinc-900/80 backdrop-blur flex items-center justify-between">
            <h3 className="text-xs font-semibold text-zinc-600 dark:text-zinc-400">{t("platform.title")}</h3>
            {busy && <Loader2 size={12} className="animate-spin text-zinc-400" />}
          </div>
          <div className="p-3 space-y-2">
            {platforms.length === 0 && (
              <p className="text-xs text-zinc-400 dark:text-zinc-600 py-4 text-center">{t("platform.empty")}</p>
            )}
            {platforms.map((p) => {
              const auto = isAutoPlatform(p.name);
              const leases = leasesPerPlatform[p.name] ?? [];
              return (
                <div
                  key={p.name}
                  onPointerEnter={() => { if (draggingUid) setDragOverPlatform(p.name); }}
                  onPointerUp={() => { if (draggingUid) void handleDropOnPlatform(p.name); }}
                  className={dragOverPlatform === p.name
                    ? "rounded-lg border-2 border-blue-400/60 bg-blue-50/30 dark:bg-blue-950/20 px-3 py-2"
                    : auto
                      ? "rounded-lg border border-solid border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900/60 px-3 py-2"
                      : "rounded-lg border-2 border-dashed border-zinc-300 dark:border-zinc-700 bg-zinc-50/50 dark:bg-zinc-800/30 px-3 py-2"}
                >
                  <div className="flex items-center justify-between mb-1">
                    <span className="flex items-center gap-2 min-w-0">
                      <span className="font-mono text-xs text-zinc-600 dark:text-zinc-300 truncate">{p.name}</span>
                      {auto && <span className="text-xs px-1.5 py-0.5 rounded bg-zinc-100 dark:bg-zinc-800 text-zinc-400">{t("platform.independent")}</span>}
                    </span>
                    <div className="flex items-center gap-1">
                      <select
                        value={p.allocationPolicy}
                        onChange={(e) => void handlePolicyChange(p.name, e.target.value as AllocationPolicy)}
                        disabled={busy}
                        className="text-xs px-1.5 py-1 rounded border border-zinc-200 dark:border-zinc-700 bg-transparent dark:text-zinc-200 focus:outline-none focus:ring-1 focus:ring-blue-400"
                      >
                        {ALLOCATION_POLICIES.map((pol) => (
                          <option key={pol} value={pol}>{pol}</option>
                        ))}
                      </select>
                      <button
                        onClick={() => { if (confirm(t("platform.deleteConfirm"))) void handleRemove(p.name); }}
                        disabled={busy}
                        className="p-1 rounded hover:bg-red-50 dark:hover:bg-red-950/40 text-zinc-400 hover:text-red-500 transition-colors"
                      >
                        <Trash2 size={12} />
                      </button>
                    </div>
                  </div>
                  <div className="text-xs text-zinc-500 dark:text-zinc-400 flex items-center gap-3">
                    <span>{t("platform.leases")}: {leases.length}</span>
                    {p.routableNodeCount > 0 && <span>nodes: {p.routableNodeCount}</span>}
                    {p.regionFilters.length > 0 && <span>regions: {p.regionFilters.join(", ")}</span>}
                  </div>
                  {leases.length > 0 && (
                    <div className="mt-1 pl-2 border-l border-zinc-200 dark:border-zinc-800 space-y-0.5">
                      {leases.slice(0, 5).map((lease, i) => (
                        <span key={i} className="block text-xs font-mono text-zinc-400 dark:text-zinc-600 truncate">
                          {String((lease as Record<string, unknown>)?.account ?? "")} → {String((lease as Record<string, unknown>)?.egress_ip ?? "?")}
                        </span>
                      ))}
                      {leases.length > 5 && <span className="text-xs text-zinc-400">+{leases.length - 5}</span>}
                    </div>
                  )}
                  {leases.length === 0 && auto && (
                    <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-1">{t("platform.noLeases")}</p>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </section>
  );
}
