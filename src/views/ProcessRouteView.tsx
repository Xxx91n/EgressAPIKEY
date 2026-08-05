import { useTranslation } from "react-i18next";
import { useEffect, useState, useCallback } from "react";
import { Route, Plus, Trash2, AlertCircle, CheckCircle2, Loader2, Inbox, ArrowRight } from "lucide-react";
import { useAppStore } from "../store/appStore";
import { ipcProcessRouteAdd, ipcProcessRouteRemove, ipcProcessRouteList } from "../lib/ipc";

/// ProcessRouteView: per-process -> lane routing table.
/// Issue 3+10: rules are PERSISTED server-side via tauri-plugin-store and
/// the Rust process_route_add REFUSES lane conflicts (a target lane already
/// bound to another process) BEFORE recording. We forward the IPC error to a
/// visible toast; outside Tauri the optimistic local appStore update keeps
/// the dev preview working. The Resin sidecar proxy owns per-request auth
/// separately; here we only own the routing-rule registry.
export function ProcessRouteView() {
  const { t } = useTranslation();
  const localRoutes = useAppStore((s) => s.processRoutes);
  const [targetPort, setTargetPort] = useState(17990);
  const setRoutes = useAppStore((s) => s.setProcessRoutes);
  const [process, setProcess] = useState("");
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState<{ kind: "ok" | "err"; msg: string } | null>(null);

  const inputCls =
    "rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-3 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500/40";

  const refreshFromBackend = useCallback(async () => {
    try {
      const rules = await ipcProcessRouteList();
      setRoutes(rules.map((r) => ({ id: r.process, process: r.process, targetPort: r.target_port })));
    } catch {
      // outside Tauri or backend not started — keep existing local routes
    }
  }, [setRoutes]);

  useEffect(() => { void refreshFromBackend(); }, [refreshFromBackend]);

  const handleAdd = async () => {
    const p = process.trim();
    if (!p) return;
    const tgt = Math.max(1024, Math.min(65535, Math.trunc(targetPort)));
    setBusy(true); setToast(null);
    try {
      await ipcProcessRouteAdd(p, tgt);
      setToast({ kind: "ok", msg: t("processRoute.backendSaved") });
      await refreshFromBackend();
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setToast({ kind: "err", msg: t("processRoute.conflict", { port: tgt, process: extractBoundProcess(msg) ?? "?" }) });
    } finally {
      setProcess(""); setTargetPort(17990); setBusy(false);
    }
  };

  const handleRemove = async (proc: string) => {
    setBusy(true); setToast(null);
    try {
      await ipcProcessRouteRemove(proc);
      await refreshFromBackend();
    } catch { /* dev fallback: trim local */ setRoutes(localRoutes.filter((r) => r.process !== proc)); }
    finally { setBusy(false); }
  };

  return (
    <section className="w-full max-w-none px-6 space-y-4">
      <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900/60">
        <div className="flex items-center gap-2 px-4 py-3 border-b border-zinc-200 dark:border-zinc-800">
          <Route size={16} className="text-zinc-500 dark:text-zinc-400" strokeWidth={1.75} />
          <h2 className="text-sm font-semibold tracking-tight">{t("processRoute.title")}</h2>
        </div>
        <div className="p-4 space-y-3">
          <div className="flex gap-2 items-end">
            <div className="flex-1 space-y-1.5">
              <label className="block text-xs font-medium text-zinc-600 dark:text-zinc-300">{t("processRoute.process")}</label>
              <input
                value={process}
                onChange={(e) => setProcess(e.target.value)}
                placeholder={t("processRoute.process")}
                className={inputCls + " w-full"}
              />
            </div>
            <div className="space-y-1.5">
              <label className="block text-xs font-medium text-zinc-600 dark:text-zinc-300">{t("processRoute.targetPort")}</label>
              <input
                type="number"
                min={1024}
                max={65535}
                value={targetPort}
                onChange={(e) => setTargetPort(Number(e.target.value))}
                aria-label={t("processRoute.targetPort")}
                className={inputCls + " w-24"}
              />
            </div>
            <button
              disabled={!process.trim() || busy}
              onClick={handleAdd}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-blue-600 hover:bg-blue-700 disabled:bg-zinc-300 dark:disabled:bg-zinc-700 disabled:cursor-not-allowed disabled:opacity-60 text-white text-sm font-medium transition-colors"
            >
              {busy ? <Loader2 size={14} className="animate-spin" /> : <Plus size={14} strokeWidth={2} />}
              {t("processRoute.add")}
            </button>
          </div>
          {toast && (
            <div className={`flex items-center gap-2 text-xs px-3 py-2 rounded-md ${
              toast.kind === "ok"
                ? "bg-green-50 dark:bg-green-950/40 text-green-700 dark:text-green-300 border border-green-200 dark:border-green-900"
                : "bg-red-50 dark:bg-red-950/40 text-red-700 dark:text-red-300 border border-red-200 dark:border-red-900"
            }`}>
              {toast.kind === "ok" ? <CheckCircle2 size={14} /> : <AlertCircle size={14} />}
              <span>{toast.msg}</span>
            </div>
          )}
        </div>
      </div>

      <div className="rounded-lg border border-blue-200 dark:border-blue-900/40 bg-blue-50 dark:bg-blue-950/30 px-4 py-2 text-xs text-blue-600 dark:text-blue-400 flex items-start gap-2">
        <AlertCircle size={14} className="mt-0.5 shrink-0" strokeWidth={1.75} />
        <span className="leading-relaxed">{t("processRoute.proxyNote")}</span>
      </div>

      {localRoutes.length === 0 ? (
        <div className="rounded-lg border border-dashed border-zinc-300 dark:border-zinc-700 py-8 flex flex-col items-center gap-2 text-zinc-400 dark:text-zinc-500">
          <Inbox size={20} strokeWidth={1.5} />
          <span className="text-xs">{t("processRoute.empty")}</span>
        </div>
      ) : (
        <ul className="space-y-2">
          {localRoutes.map((r) => (
            <li
              key={r.process}
              className="rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900/60 px-4 py-3 text-sm flex items-center justify-between"
            >
              <span className="font-mono text-xs text-zinc-600 dark:text-zinc-300 truncate max-w-[60%]">{r.process}</span>
              <span className="flex items-center gap-2 text-zinc-500 dark:text-zinc-400">
                <ArrowRight size={12} />
                <span className="font-mono">:{r.targetPort}</span>
                <button
                  onClick={() => handleRemove(r.process)}
                  disabled={busy}
                  aria-label={t("common.delete")}
                  className="text-zinc-400 hover:text-red-600 dark:hover:text-red-400 p-1 disabled:opacity-40"
                >
                  <Trash2 size={14} />
                </button>
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function extractBoundProcess(errMsg: string): string | null {
  const m = errMsg.match(/bound to process '([^']+)'/);
  return m ? m[1] : null;
}
