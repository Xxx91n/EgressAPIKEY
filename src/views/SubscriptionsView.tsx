import { useTranslation } from "react-i18next";
import { useEffect, useState, useCallback, useRef } from "react";
import { Rss, Download, Inbox, Trash2, Loader2, GripVertical, CheckCircle2, AlertCircle, Pencil, ArrowDownUp } from "lucide-react";
import { useAppStore } from "../store/appStore";
import {
  ipcSubscriptionAdd, ipcSubscriptionList, ipcSubscriptionRemove, ipcNodePoolSnapshot,
  type SubscriptionSnapshotEntry,
} from "../lib/ipc";
import { loadSubOrder, saveSubOrder } from "../lib/settings";

/// SubscriptionsView - Resin subscription import surface (P20 rewrite).
///
/// Fixes vs the P19 version:
///  - Item 3: duplicate-name detection BEFORE import. Resin POST /api/v1/subscriptions
///    is name-keyed - posting the same name silently overwrites the prior sub.
///    We fetch the live list, and if the chosen name already exists we refuse to
///    POST and surface a toast so the user can rename. Coexistence by design.
///  - Item 4: reset-order no longer duplicates. The old applyOrder merged stale
///    localOrder entries that no longer exist on the server with the fresh server
///    list, causing "ghost" rows after reset. The new reset drops localOrder AND
///    re-renders from the server list only (`setLive(serverList)`), bypassing
///    applyOrder entirely, so there is no stale-name carryover.
///  - Item 6: drag-to-reorder uses the browser-native HTML5 draggable attribute
///    (dragstart / dragover / drop events). The old hand-rolled subDrag mutated a
///    useState object's isDragging flag without triggering a React re-render, so
///    the window mouseup listener that was supposed to fire finishDrag() never
///    re-registered after the first drag - dragging "did nothing". Native drag
///    has no such race: the browser owns the drag lifecycle and emits dragstart
///    once + drop once, both on the DOM elements the user actually grabs.
///
/// Outside Tauri (Vite dev preview), every IPC call throws gracefully and we
/// keep the local appStore as a fallback list so the screen never blanks.
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
  // P20 item 6: HTML5 drag tracks the dragged index in a ref so re-renders do
  // not lose the in-flight drag; the browser owns the drag session lifetime.
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null);
  // P20 item 6: local order override. Loaded once on mount, persisted on every
  // reorder. Reset (item 4) clears this to [] and re-renders from server only.
  const [localOrder, setLocalOrder] = useState<string[]>([]);

  // B2: persist form draft so navigating away+back keeps input contents.
  useEffect(() => { setSubFormDraft({ name, url }); }, [name, url, setSubFormDraft]);

  // P20 item 6: load the persisted local order once on mount.
  useEffect(() => {
    void (async () => {
      const saved = await loadSubOrder().catch(() => null);
      if (saved && saved.length > 0) setLocalOrder(saved);
    })();
  }, []);

  /// Re-sort a fresh list by the user's local order. Items in the server list
  /// that the user has never reordered (new subs, or order cleared) fall through
  /// to the end, keeping the server-defined order. P20 item 4: only names that
  /// are STILL in the server list are kept in the output - a stale localOrder
  /// entry that Resin no longer returns is dropped, so reset never duplicates.
  const applyOrder = useCallback((lst: SubscriptionSnapshotEntry[], order: string[]): SubscriptionSnapshotEntry[] => {
    if (order.length === 0) return lst;
    const indexed = new Map<string, SubscriptionSnapshotEntry>();
    for (const x of lst) { const k = (x.name || "").trim(); if (k) indexed.set(k, x); }
    const out: SubscriptionSnapshotEntry[] = [];
    for (const want of order) {
      const k = (want || "").trim();
      const item = indexed.get(k);
      if (item) { out.push(item); indexed.delete(k); }
    }
    // Append remaining server entries (new subs the user has not ordered)
    // in their original order so the new sub is visible below the layout.
    for (const x of indexed.values()) out.push(x);
    return out;
  }, []);

  const inputCls =
    "flex-1 rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-3 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500/40";

  const refresh = useCallback(async () => {
    try {
      const lst = await ipcSubscriptionList();
      // P20 item 4: server fetch is the source of truth - applyOrder only
      // re-sorts names that exist in BOTH the order array and the server list.
      setLive(applyOrder(lst, localOrder));
    } catch { /* keep last */ }
  }, [applyOrder, localOrder]);
  useEffect(() => { void refresh(); }, [refresh]);

  // P13 B4: Resin parses local subscription content on its 30s update_interval
  // tick. Poll up to 5 times at 3s so the user sees the real node_count within
  // ~15s instead of staying at 0.
  const refreshWithRetry = useCallback(async () => {
    for (let i = 0; i < 5; i++) {
      try {
        const lst = await ipcSubscriptionList();
        const ordered = applyOrder(lst, localOrder);
        setLive(ordered);
        const total = ordered.reduce((s: number, x: SubscriptionSnapshotEntry) => s + x.node_count, 0);
        if (total > 0) return;
      } catch { return; }
      await new Promise((r) => setTimeout(r, 3000));
    }
    try { setLive(applyOrder(await ipcSubscriptionList(), localOrder)); } catch {}
  }, [applyOrder, localOrder]);

  // P20 item 3: check the live list for a duplicate name BEFORE posting to
  // Resin. Resin POST /api/v1/subscriptions is name-keyed - a second POST with
  // an existing name silently overwrites the prior subscription's nodes. By
  // detecting first we refuse the import and surface a toast so the user
  // renames; distinct subscriptions coexist instead of clobbering each other.
  const handleAdd = async () => {
    const n = name.trim() || url.trim().replace(/[^A-Za-z0-9_-]/g, "_").slice(0, 48);
    const u = url.trim();
    if (!n || !u) return;
    setBusy(true); setToast(null);
    // Item 3: duplicate-name guard. Best-effort - if the list fetch fails
    // (outside Tauri) we proceed so the optimistic path still works in dev.
    try {
      const existing = await ipcSubscriptionList();
      if (existing.some((s) => s.name === n)) {
        setToast({ kind: "err", msg: t("subscription.duplicate", { name: n }) });
        setBusy(false);
        return;
      }
    } catch { /* dev: allow */ }
    localAdd(u, 0, lanes);
    try {
      await ipcSubscriptionAdd(n, u);
      // Push the new sub name into the local order so it stays at the tail
      // of the hand-sorted list on subsequent refreshes.
      const newOrder = [...localOrder, n];
      setLocalOrder(newOrder);
      void saveSubOrder(newOrder).catch(() => {});
      await new Promise((r) => setTimeout(r, 1500));
      await refreshWithRetry();
      try {
        const pool = await ipcNodePoolSnapshot();
        const total = Number(pool?.total_nodes ?? 0);
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
    try {
      await ipcSubscriptionRemove(subName);
      const newOrder = localOrder.filter((n) => n !== subName);
      setLocalOrder(newOrder);
      void saveSubOrder(newOrder).catch(() => {});
      await refresh();
    } catch { /* local */ }
    setBusy(false);
  };

  /// P19 item 6: rename via delete + recreate. Resin has no PATCH
  /// /api/v1/subscriptions, so we delete the old sub and create a new one
  /// with the new name. Item 3 applies: we also reject a rename target that
  /// collides with another existing sub's name.
  const handleRename = async (oldName: string) => {
    let renamed = "";
    try {
      renamed = window.prompt(t("subscription.renamePrompt", { name: oldName }), oldName) || "";
    } catch { return; }
    renamed = (renamed || "").trim();
    if (!renamed || renamed === oldName) return;
    setBusy(true); setToast(null);
    const existing = live.find((x) => x.name === oldName);
    if (!existing) {
      setToast({ kind: "err", msg: t("subscription.renameMissing") });
      setBusy(false);
      return;
    }
    // P20 item 3: reject a rename that would collide with another live sub.
    if (live.some((s) => s.name === renamed)) {
      setToast({ kind: "err", msg: t("subscription.duplicate", { name: renamed }) });
      setBusy(false);
      return;
    }
    const cached = localSubs.find((s) => s.url && s.url.length > 0);
    const sourceUrl = cached?.url ?? "";
    if (!sourceUrl) {
      setToast({ kind: "err", msg: t("subscription.renameUrlMissing") });
      setBusy(false);
      return;
    }
    try {
      await ipcSubscriptionRemove(oldName);
      await ipcSubscriptionAdd(renamed, sourceUrl);
      const idx = localOrder.indexOf(oldName);
      const newOrder = idx >= 0
        ? localOrder.map((n) => (n === oldName ? renamed : n))
        : [...localOrder, renamed];
      setLocalOrder(newOrder);
      void saveSubOrder(newOrder).catch(() => {});
      await refreshWithRetry();
      setToast({ kind: "ok", msg: t("subscription.renameOk", { name: renamed }) });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setToast({ kind: "err", msg });
    } finally {
      setBusy(false);
    }
  };

  /// P20 item 4: clear the persisted order and re-render from the server list
  /// ONLY. The old code called refresh() which applyOrder'd against the (now
  /// empty) localOrder, but also re-merged stale names that were once in
  /// localOrder - leading to duplicate / leftover rows after reset. The new
  /// path bypasses applyOrder entirely: serverList goes straight to setLive.
  const handleResetOrder = async () => {
    setLocalOrder([]);
    void saveSubOrder([]).catch(() => {});
    try {
      const serverList = await ipcSubscriptionList();
      setLive(serverList);
    } catch { /* keep */ }
  };

  // ---- P21 item 1: Pointer Events drag (WebView2-stable; HTML5 DnD showed
  // a "禁止符号" because onDragStart never set e.dataTransfer.effectAllowed/
  // setData — WebView2 suppresses the drag session entirely without them, and
  // Tauri's webview intercepts text/plain drags for native window drag.
  // Pointer Events are the same model env-manager's ProfilePage.svelte uses
  // (onpointerdown / onpointerenter / onpointerup) — it works everywhere with
  // zero deps and no image/dataTransfer ceremony.
  // We stash the source index + drag ref in refs (survive re-renders), a
  // hasDragged flag distinguishes a real drag from a click, and a window
  // pointerup listener is registered once via useEffect so releasing the
  // mouse outside any row still finishes the drag cleanly.
  const dragSrc = useRef<number | null>(null);
  const dragMoved = useRef(false);

  useEffect(() => {
    const onUp = () => {
      if (dragSrc.current !== null && dragMoved.current) {
        // released outside a valid drop target — cancel, keep original order
        dragSrc.current = null;
        dragMoved.current = false;
        setDragOverIndex(null);
      }
    };
    window.addEventListener("pointerup", onUp);
    return () => window.removeEventListener("pointerup", onUp);
  }, []);

  const onPointerDownRow = (e: React.PointerEvent, i: number) => {
    // Only left button starts a drag; right/middle are clicks.
    if (e.button !== 0) return;
    dragSrc.current = i;
    dragMoved.current = false;
    // Capture pointer events so onPointerEnter continues to fire while moving
    // fast even if the pointer leaves the row briefly.
    try { (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId); } catch { /* noop */ }
  };

  const onPointerEnterRow = (i: number) => {
    if (dragSrc.current === null) return;
    if (dragSrc.current !== i) {
      dragMoved.current = true;
      setDragOverIndex(i);
      // Instant swap (env-manager style): move the dragged item to the hovered
      // position on every enter, so the list reorders live under the cursor
      // and the result is the same whether the user drops or just keeps going.
      setLive((lst) => {
        const from = dragSrc.current ?? -1;
        if (from < 0 || from === i || from >= lst.length) return lst;
        const next = [...lst];
        const [moved] = next.splice(from, 1);
        next.splice(i, 0, moved);
        dragSrc.current = i;
        const newOrder = next.map((x) => x.name);
        setLocalOrder(newOrder);
        void saveSubOrder(newOrder).catch(() => {});
        return next;
      });
    }
  };

  const onPointerUpRow = (_i: number) => {
    // The live-swap already committed the new order on enter; this just clears
    // state. If the user never moved (click without drag), dragMoved is false
    // and the order is unchanged.
    dragSrc.current = null;
    dragMoved.current = false;
    setDragOverIndex(null);
  };

  return (
    <section className="w-full max-w-none px-6 space-y-4">
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
          <li className="text-xs text-zinc-400 dark:text-zinc-500 px-1 flex items-center justify-between">
            <span>{t("subscription.dragHint")}</span>
            {localOrder.length > 0 && (
              <button
                onClick={handleResetOrder}
                disabled={busy}
                aria-label={t("subscription.resetOrder")}
                title={t("subscription.resetOrder")}
                className="inline-flex items-center gap-1 text-xs text-zinc-400 hover:text-blue-600 dark:hover:text-blue-400 disabled:opacity-40"
              >
                <ArrowDownUp size={12} />
                {t("subscription.resetOrder")}
              </button>
            )}
          </li>
          {live.map((s, i) => (
            <li
              key={s.name}
              onPointerDown={(e) => onPointerDownRow(e, i)}
              onPointerEnter={() => onPointerEnterRow(i)}
              onPointerUp={() => onPointerUpRow(i)}
              className={"rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900/60 px-4 py-3 text-sm flex items-center justify-between cursor-grab active:cursor-grabbing select-none touch-none transition-opacity " + (dragOverIndex === i ? "ring-2 ring-blue-400/50 " : "") + (dragSrc.current === i && dragMoved.current ? "opacity-60 " : "")}
            >
              <span className="flex items-center gap-2 min-w-0 flex-1">
                <GripVertical size={14} className="text-zinc-400 dark:text-zinc-600 shrink-0" />
                <span className="font-mono text-xs text-zinc-600 dark:text-zinc-300 truncate max-w-[60%]">{s.name}</span>
              </span>
              <span className="text-xs text-zinc-500 dark:text-zinc-400 flex items-center gap-2">
                {t("subscription.imported", { count: s.node_count, lanes })}
                <button
                  onClick={() => handleRename(s.name)}
                  disabled={busy}
                  aria-label={t("subscription.rename")}
                  title={t("subscription.rename")}
                  className="text-zinc-400 hover:text-blue-600 dark:hover:text-blue-400 p-1 disabled:opacity-40"
                >
                  <Pencil size={13} />
                </button>
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
