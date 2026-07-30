import { useTranslation } from "react-i18next";
import { useEffect, useState, useCallback } from "react";
import { Rss, Download, Inbox, Trash2, Loader2, GripVertical, CheckCircle2, AlertCircle } from "lucide-react";
import { useAppStore } from "../store/appStore";
import {
  ipcSubscriptionAdd, ipcSubscriptionList, ipcSubscriptionRemove, ipcNodePoolSnapshot,
  type SubscriptionSnapshotEntry,
} from "../lib/ipc";
import { createDragState, beginDrag, enterTarget, finishDrag } from "../lib/subDrag";

/// SubscriptionsView is the Resin subscription import surface. Under Path
/// A, add/list/remove forward to the live Resin sidecar admin REST:
///   subscription_add -> POST /api/v1/subscriptions (source_type=local)
///     The shell fetches the Clash YAML itself (clash-family UA, Resin's
///     default UA gets 403 from many providers), converts flow-style
///     proxies to block-style, and posts local content. Resin parses the
///     nodes on its 30s update_interval tick.
///   subscription_list -> GET /api/v1/subscriptions  (project name + node_count)
///   subscription_remove -> DELETE /api/v1/subscriptions/{id}
/// B2: the form draft (name+url) is persisted to appStore so navigating
///   away+back keeps the input contents instead of the previous collapse.
/// B3: rows support drag-to-reorder (env-manager profileDrag pattern, zero
///   extra deps) with a window-level mouseup listener so releasing outside
///   the row still commits the drag.
/// Outside Tauri (Vite dev preview), every IPC call throws gracefully and
/// we keep the local appStore as a fallback list so the screen never blanks.
export function SubscriptionsView() {
  const { t } = useTranslation();
  const lanes = useAppStore((s) => s.laneCount);
  const localAdd = useAppStore((s) => s.addSubscription);
  const localSubs = useAppStore((s) => s.subscriptions);
  const subFormDraft = useAppStore((s) => s.subFormDraft);
  const setSubFormDraft = useAppStore((s) => s.setSubFormDraft);
  const [name, setName] = useState(subFormDraft.name);
  const [url, setUrl] = useState(subFormDraft.url);
  const [busy, setBusy] = useState(false);
  const [live, setLive] = useState<SubscriptionSnapshotEntry[]>([]);
  const [toast, setToast] = useState<{ kind: "ok" | "err"; msg: string } | null>(null);
  const [drag] = useState(createDragState);

  // B2: persist form draft so navigating away+back keeps input contents.
  useEffect(() => { setSubFormDraft({ name, url }); }, [name, url, setSubFormDraft]);

  // B3: window-level mouseup so releasing outside the row still commits the drag.
  useEffect(() => {
    if (!drag.isDragging) return;
    const onUp = () => setLive((lst) => finishDrag(drag, lst));
    window.addEventListener("mouseup", onUp, { once: true });
    return () => window.removeEventListener("mouseup", onUp);
  }, [drag, drag.isDragging, drag.dragIndex, drag.dragOverIndex]);

  const inputCls =
    "flex-1 rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-3 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500/40";

  const refresh = useCallback(async () => {
    try { setLive(await ipcSubscriptionList()); } catch { /* keep last */ }
  }, []);
  useEffect(() => { void refresh(); }, [refresh]);

  // P13 B4: Resin parses local subscription content on its 30s
  // update_interval tick. Poll up to 5 times at 3s so the user sees the
  // real node_count within ~15s instead of staying at 0.
  const refreshWithRetry = useCallback(async () => {
    for (let i = 0; i < 5; i++) {
      try {
        const lst = await ipcSubscriptionList();
        setLive(lst);
        const total = lst.reduce((s: number, x: SubscriptionSnapshotEntry) => s + x.node_count, 0);
        if (total > 0) return;
      } catch { return; }
      await new Promise((r) => setTimeout(r, 3000));
    }
    try { setLive(await ipcSubscriptionList()); } catch {}
  }, []);

  const handleAdd = async () => {
    const n = name.trim() || url.trim().replace(/[^A-Za-z0-9_-]/g, "_").slice(0, 48);
    const u = url.trim();
    if (!n || !u) return;
    setBusy(true); setToast(null);
    localAdd(u, 0, lanes);
    try {
      await ipcSubscriptionAdd(n, u);
      // B4: retry-refresh catches the async Resin subscription parse.
      await new Promise((r) => setTimeout(r, 1500));
      await refreshWithRetry();
      try {
        const pool = await ipcNodePoolSnapshot();
        const total = Number(pool?.total_nodes ?? 0);
        // B1: i18n interpolation, not hardcoded English.
        setToast({ kind: "ok", msg: t("subscription.importSuccess", { total }) });
      } catch { /* node pool optional */ }
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setToast({ kind: "err", msg });
    }
    setName(""); setUrl(""); setBusy(false);
  };

  const handleRemove = async (subName: string) => {
    setBusy(true); setToast(null);
    try { await ipcSubscriptionRemove(subName); await refresh(); } catch { /* local */ }
    setBusy(false);
  };

  return (
    <section className="max-w-2xl space-y-4">
      <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900/60">
        <div className="flex items-center gap-2 px-4 py-3 border-b border-zinc-200 dark:border-zinc-800">
          <Rss size={16} className="text-zinc-500 dark:text-zinc-400" strokeWidth={1.75} />
          <h2 className="text-sm font-semibold tracking-tight">{t("subscription.title")}</h2>
        </div>
        <div className="p-4 space-y-3">
          <div className="grid grid-cols-1 gap-1.5">
            <label className="text-xs text-zinc-500 dark:text-zinc-400">{t("subscription.name")}</label>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t("subscription.name")}
              className={inputCls}
              aria-label={t("subscription.name")}
            />
            <span className="text-xs text-zinc-400 dark:text-zinc-500">{t("subscription.description")}</span>
          </div>
          <div className="grid grid-cols-1 gap-1.5">
            <label className="text-xs text-zinc-500 dark:text-zinc-400">{t("subscription.url")}</label>
            <div className="flex gap-2 items-start">
              <input
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder={t("subscription.url")}
                className={inputCls + " flex-1"}
                aria-label={t("subscription.url")}
              />
              <button
                onClick={handleAdd}
                disabled={busy || !url.trim()}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium transition-colors disabled:opacity-40 shrink-0"
              >
                {busy ? <Loader2 size={14} className="animate-spin" /> : <Download size={14} strokeWidth={2} />}
                {t("subscription.import")}
              </button>
            </div>
          </div>
          {toast && (
            <div className={"flex items-center gap-2 text-xs px-3 py-2 rounded-md " + (toast.kind === "ok"
                ? "bg-green-50 dark:bg-green-950/40 text-green-700 dark:text-green-300 border border-green-200 dark:border-green-900"
                : "bg-red-50 dark:bg-red-950/40 text-red-700 dark:text-red-300 border border-red-200 dark:border-red-900")}>
              {toast.kind === "ok" ? <CheckCircle2 size={14} /> : <AlertCircle size={14} />}
              <span>{toast.msg}</span>
            </div>
          )}
        </div>
      </div>

      {live.length === 0 && localSubs.length === 0 ? (
        <div className="rounded-lg border border-dashed border-zinc-300 dark:border-zinc-700 py-8 flex flex-col items-center gap-2 text-zinc-400 dark:text-zinc-500">
          <Inbox size={20} strokeWidth={1.5} />
          <span className="text-xs">{t("platform.empty")}</span>
        </div>
      ) : (
        <ul className="space-y-2">
          <li className="text-xs text-zinc-400 dark:text-zinc-500 px-1">{t("subscription.dragHint")}</li>
          {live.map((s, i) => (
            <li
              key={s.name}
              onMouseDown={(e) => beginDrag(drag, i, e)}
              onMouseEnter={() => enterTarget(drag, i)}
              className={"rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900/60 px-4 py-3 text-sm flex items-center justify-between cursor-move transition-opacity " + (drag.isDragging && drag.dragIndex === i ? "opacity-50" : "")}
            >
              <span className="flex items-center gap-2 min-w-0 flex-1">
                <GripVertical size={14} className="text-zinc-400 dark:text-zinc-600 shrink-0" />
                <span className="font-mono text-xs text-zinc-600 dark:text-zinc-300 truncate max-w-[60%]">{s.name}</span>
              </span>
              <span className="text-xs text-zinc-500 dark:text-zinc-400 flex items-center gap-2">
                {t("subscription.imported", { count: s.node_count, lanes })}
                <button
                  onClick={() => handleRemove(s.name)}
                  disabled={busy}
                  aria-label={t("common.delete")}
                  className="text-zinc-400 hover:text-red-600 dark:hover:text-red-400 p-1 disabled:opacity-40"
                >
                  <Trash2 size={14} />
                </button>
              </span>
            </li>
          ))}
          {live.length === 0 && localSubs.map((s) => (
            <li
              key={s.id}
              className="rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900/60 px-4 py-3 text-sm flex items-center justify-between"
            >
              <span className="font-mono text-xs text-zinc-600 dark:text-zinc-300 truncate max-w-[60%]">{s.url}</span>
              <span className="text-xs text-zinc-500 dark:text-zinc-400">
                {t("subscription.imported", { count: s.nodeCount, lanes: s.lanes })}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
