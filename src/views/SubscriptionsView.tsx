import { useTranslation } from "react-i18next";
import { useEffect, useState, useCallback, useRef } from "react";
import { Rss, Download, Inbox, Trash2, Loader2, GripVertical, CheckCircle2, AlertCircle, Pencil, ArrowDownUp, ChevronDown } from "lucide-react";
import { useAppStore } from "../store/appStore";
import {
  ipcSubscriptionAdd, ipcSubscriptionList, ipcSubscriptionRemove, ipcNodePoolSnapshot,
  type SubscriptionSnapshotEntry,
} from "../lib/ipc";
import { translateError } from "../lib/i18n-error";
import { loadSubOrder, saveSubOrder } from "../lib/settings";
import { usePoll } from "../hooks/usePoll";

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
  const localAdd = useAppStore((s) => s.addSubscription);
  const localSubs = useAppStore((s) => s.subscriptions);
  const subFormDraft = useAppStore((s) => s.subFormDraft);
  const setSubFormDraft = useAppStore((s) => s.setSubFormDraft);
  const [name, setName] = useState(subFormDraft.name);
  const [url, setUrl] = useState(subFormDraft.url);
  const [busy, setBusy] = useState(false);
  const [live, setLive] = useState<SubscriptionSnapshotEntry[]>([]);
  const [toast, setToast] = useState<{ kind: "ok" | "err"; msg: string; withErrorHint?: boolean } | null>(null);
  // P20 item 6: HTML5 drag tracks the dragged index in a ref so re-renders do
  // not lose the in-flight drag; the browser owns the drag session lifetime.
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null);
  // P20 item 6: local order override. Loaded once on mount, persisted on every
  // reorder. Reset (item 4) clears this to [] and re-renders from server only.
  const [localOrder, setLocalOrder] = useState<string[]>([]);

  // pre-selects "auto-create the same-name platform" ONLY when no
  // a_class=subscription platform already consumes the sub (issue: "若
  // a_class=subscription 已有则不重复").
  /** a_class=subscription platform names offered as bind chips. */
  /** Existing consumers of the sub (chips pre-checked; never re-created). */
  /** Current user selection (multi-select). */

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

  // T03 (round5): one-line error summary for toasts - Resin last_error can be
  // a multi-line downloader dump; a toast only fits the first meaningful line.
  const summarizeError = useCallback((raw: string): string => {
    const first = (raw || "").split("\n").flatMap((l) => l.split("\r")).map((l) => l.trim()).filter(Boolean)[0] || "";
    return first.length > 120 ? first.slice(0, 117) + "..." : first;
  }, []);

  // T03 (round5): edge-triggered 30s poll. Resin's 30s update_interval tick
  // writes subscription.last_error without any push channel to the shell, so
  // the view polls subscription_list every 30s (independent from the 5s
  // snapshot poll) and compares (last_error, last_checked) per subscription
  // against the previous poll. A transition into failure toasts a red banner,
  // a transition back toasts recovery; steady states stay silent. uses the
  // shared usePoll hook (visibility-paused) - not a second interval loop.
  const prevSubStateRef = useRef<Map<string, { last_error: string; last_checked: string }> | null>(null);
  const pollEdge = useCallback(async (signal: AbortSignal) => {
    let lst: SubscriptionSnapshotEntry[];
    try {
      lst = await ipcSubscriptionList();
    } catch { return; }
    if (signal.aborted) return;
    // Server list is the source of truth (same contract as refresh()); the
    // re-sort by localOrder keeps a just-dragged order intact. This also
    // clears a stale red banner in the same tick the recovery toast fires.
    setLive(applyOrder(lst, localOrder));
    const prev = prevSubStateRef.current;
    const next = new Map<string, { last_error: string; last_checked: string }>();
    for (const s of lst) next.set(s.name, { last_error: s.last_error || "", last_checked: s.last_checked || "" });
    if (prev !== null) {
      for (const [name, st] of next) {
        const before = prev.get(name);
        if (!before) continue; // brand-new subscription: import toast already covered it
        const wasErr = before.last_error !== "";
        const nowErr = st.last_error !== "";
        if (!wasErr && nowErr) {
          setToast({ kind: "err", msg: t("subscription.fetchFailed", { name, error: summarizeError(st.last_error) }) });
        } else if (wasErr && !nowErr) {
          setToast({ kind: "ok", msg: t("subscription.fetchRecovered", { name }) });
        }
      }
    }
    prevSubStateRef.current = next;
  }, [applyOrder, localOrder, summarizeError, t]);
  usePoll(pollEdge, { intervalMs: 30_000, fireImmediately: false });

  // P13 B4: Resin parses local subscription content on its 30s update_interval
  // tick. Poll up to 5 times at 3s so the user sees the real node_count within
  // ~15s instead of staying at 0.
  // T03 (round5): exit early not only on nodes but also on a surfaced
  // last_error - "imported but Resin cannot pull it" must be reported by the
  // import toast instead of silently settling for 0 nodes.
  const refreshWithRetry = useCallback(async (): Promise<SubscriptionSnapshotEntry[]> => {
    let last: SubscriptionSnapshotEntry[] = [];
    for (let i = 0; i < 5; i++) {
      try {
        const lst = await ipcSubscriptionList();
        const ordered = applyOrder(lst, localOrder);
        last = ordered;
        setLive(ordered);
        const total = ordered.reduce((s: number, x: SubscriptionSnapshotEntry) => s + x.node_count, 0);
        if (total > 0) return ordered;
        const failed = ordered.find((x) => (x.last_error || "").trim() !== "");
        if (failed) {
          setToast({ kind: "err", msg: t("subscription.importFetchFailed", { name: failed.name, error: summarizeError(failed.last_error) }) });
          return ordered;
        }
      } catch { return last; }
      await new Promise((r) => setTimeout(r, 3000));
    }
    try {
      const ordered = applyOrder(await ipcSubscriptionList(), localOrder);
      last = ordered;
      setLive(ordered);
    } catch { /* keep last */ }
    return last;
  }, [applyOrder, localOrder, summarizeError, t]);

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
    localAdd(u, 0);
    try {
      await ipcSubscriptionAdd(n, u);
      // Push the new sub name into the local order so it stays at the tail
      // of the hand-sorted list on subsequent refreshes.
      const newOrder = [...localOrder, n];
      setLocalOrder(newOrder);
      void saveSubOrder(newOrder).catch((e) => console.warn("[SubscriptionsView] saveSubOrder failed", e));
      await new Promise((r) => setTimeout(r, 50));
      const finalList = await refreshWithRetry();
      try {
        const pool = await ipcNodePoolSnapshot();
        const total = Number(pool?.total_nodes ?? 0);
        const failedSub = finalList.find((x) => (x.last_error || "").trim() !== "");
        setToast({
          kind: "ok",
          msg: failedSub
            ? t("subscription.importSuccessWithHint", { total })
            : t("subscription.importSuccess", { total }),
          withErrorHint: failedSub !== undefined,
        });
      } catch { /* node pool optional */ }
    } catch (e: unknown) {
      const msg = translateError(e, t);
      setToast({ kind: "err", msg });
    } finally {
      try { setName(""); setUrl(""); setBusy(false); } catch { /* unmounted */ }
    }
  };

  const handleRemove = async (subName: string) => {
    setBusy(true); setToast(null);
    try {
      await ipcSubscriptionRemove(subName);
      const newOrder = localOrder.filter((n) => n !== subName);
      setLocalOrder(newOrder);
      void saveSubOrder(newOrder).catch((e) => console.warn("[SubscriptionsView] saveSubOrder failed", e));
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
      void saveSubOrder(newOrder).catch((e) => console.warn("[SubscriptionsView] saveSubOrder failed", e));
      await refreshWithRetry();
      setToast({ kind: "ok", msg: t("subscription.renameOk", { name: renamed }) });
    } catch (e: unknown) {
      const msg = translateError(e, t);
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
    void saveSubOrder([]).catch((e) => console.warn("[SubscriptionsView] saveSubOrder failed", e));
    try {
      const serverList = await ipcSubscriptionList();
      setLive(serverList);
    } catch { /* keep */ }
  };

  const dragSrc = useRef<number | null>(null);
  const dragMoved = useRef(false);

  const onPointerDownRow = (e: React.PointerEvent, i: number) => {
    // Only left button starts a drag; right/middle are clicks.
    if (e.button !== 0) return;
    dragSrc.current = i;
    dragMoved.current = false;
    // Capture pointer events so onPointerEnter continues to fire while moving
    // fast even if the pointer leaves the row briefly.
    // T6-Bug3: removed setPointerCapture — it locked pointer events on the source row
    // and prevented onPointerEnter from firing on other rows during drag.
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
        void saveSubOrder(newOrder).catch((e) => console.warn("[SubscriptionsView] saveSubOrder failed", e));
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
              {toast.kind === "ok" && toast.withErrorHint && (
                <span
                  data-testid="toast-error-chip"
                  className="ml-auto shrink-0 inline-flex items-center gap-1 rounded-full bg-red-100 dark:bg-red-900/60 text-red-700 dark:text-red-300 px-2 py-0.5 text-[11px] font-medium"
                >
                  <AlertCircle size={11} />
                  {t("subscription.fetchErrorChip")}
                </span>
              )}
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
              className={"rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900/60 px-4 py-3 text-sm cursor-grab active:cursor-grabbing select-none touch-none transition-opacity " + (dragOverIndex === i ? "ring-2 ring-blue-400/50 " : "") + (dragSrc.current === i && dragMoved.current ? "opacity-60 " : "")}
            >
              <div className="flex items-center justify-between">
                <span className="flex items-center gap-2 min-w-0 flex-1">
                  <GripVertical size={14} className="text-zinc-400 dark:text-zinc-600 shrink-0" />
                  <span className="font-mono text-xs text-zinc-600 dark:text-zinc-300 truncate max-w-[60%]">{s.name}</span>
                </span>
                <span className="text-xs text-zinc-500 dark:text-zinc-400 flex items-center gap-2">
                  {t("subscription.imported", { count: s.node_count })}
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
              </div>
              <SubscriptionRowHint s={s} t={t} />
            </li>
          ))}
          {live.length === 0 && localSubs.map((s) => (
            <li
              key={s.id}
              className="rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900/60 px-4 py-3 text-sm flex items-center justify-between"
            >
              <span className="font-mono text-xs text-zinc-600 dark:text-zinc-300 truncate max-w-[60%]">{s.url}</span>
              <span className="text-xs text-zinc-500 dark:text-zinc-400">
                {t("subscription.imported", { count: s.nodeCount })}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
/// Item 2 / Option B: surface Resin last_error / last_checked / healthy
/// so "imported ok but 0 nodes" is no longer a mute zero.
/// T03 (round5): the former 11px red text line is promoted to a full red
/// banner (bg-red-50 / dark:bg-red-950/30 + red-300 border) so a fetch
/// failure is impossible to miss, with a collapse toggle for the raw
/// message, a green check when the sub has recovered since its last
/// check (last_error empty + last_checked present), and the last-check
/// timestamp kept as the "error since / checked at" anchor. The collapsed
/// default keeps rows compact; the red border + one-line summary stay
/// visible either way, so hiding the detail never hides the failure.
function SubscriptionRowHint({ s, t }: { s: SubscriptionSnapshotEntry; t: ReturnType<typeof useTranslation>["t"] }) {
  const [open, setOpen] = useState(false);
  const hasError = (s.last_error || "").trim() !== "";
  const lc = (s.last_checked || "").replace(/\.\d+Z$/, "Z");
  const lcShort = lc ? lc.replace("T", " ").slice(0, 19) : "";
  if (!hasError) {
    // Healthy row hint: green check (recovered-or-never-failed) + healthy
    // count + last_checked, exactly the old compact line.
    return (
      <div className="mt-1 px-1 text-[11px] flex flex-wrap items-center gap-x-3 gap-y-0.5 leading-tight">
        <span className="shrink-0 inline-flex items-center gap-1 text-emerald-600 dark:text-emerald-400">
          {lcShort && <CheckCircle2 size={11} strokeWidth={2} aria-label={t("subscription.lastSuccess")} />}
          {t("subscription.healthy")} {s.healthy_node_count}
        </span>
        {lcShort && (
          <span className="text-zinc-400 dark:text-zinc-500 font-mono">{lcShort}</span>
        )}
      </div>
    );
  }
  return (
    <div
      data-testid="sub-error-banner"
      className="mt-1 rounded-md border border-red-300 bg-red-50 dark:bg-red-950/30 dark:border-red-900 px-2 py-1.5 text-[11px] leading-tight"
    >
      <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
        <AlertCircle size={12} className="shrink-0 text-red-600 dark:text-red-400" />
        <span className="text-red-700 dark:text-red-300 break-all min-w-0 flex-1">
          {open ? s.last_error : t("subscription.fetchErrorBanner", { error: s.last_error.trim().split("\n")[0].slice(0, 80) })}
        </span>
        <button
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
          aria-label={open ? t("subscription.hideError") : t("subscription.showError")}
          className="shrink-0 inline-flex items-center gap-0.5 text-red-600 dark:text-red-400 hover:text-red-700 dark:hover:text-red-300 p-0.5"
        >
          <ChevronDown size={12} className={"transition-transform " + (open ? "rotate-180" : "")} />
          {open ? t("subscription.hideError") : t("subscription.showError")}
        </button>
      </div>
      <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px]">
        <span className="shrink-0 text-red-700 dark:text-red-300">
          {t("subscription.healthy")} {s.healthy_node_count}
        </span>
        {lcShort && (
          <span className="text-red-400 dark:text-red-500/80 font-mono">{lcShort}</span>
        )}
      </div>
    </div>
  );
}
