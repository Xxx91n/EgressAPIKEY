import { useTranslation } from "react-i18next";
import { useState } from "react";
import { Route, Plus, Trash2, ArrowRight, Inbox } from "lucide-react";
import { useAppStore } from "../store/appStore";

export function ProcessRouteView() {
  const { t } = useTranslation();
  const routes = useAppStore((s) => s.processRoutes);
  const laneCount = useAppStore((s) => s.laneCount);
  const addRoute = useAppStore((s) => s.addProcessRoute);
  const removeRoute = useAppStore((s) => s.removeProcessRoute);
  const [process, setProcess] = useState("");
  const [target, setTarget] = useState(0);

  const inputCls =
    "rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-3 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500/40";

  return (
    <section className="max-w-2xl space-y-4">
      <div className="flex items-center gap-2">
        <Route size={16} className="text-zinc-500" strokeWidth={1.75} />
        <h2 className="text-sm font-semibold tracking-tight">{t("processRoute.title")}</h2>
      </div>

      <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900/60 p-4">
        <div className="flex gap-2 items-end">
          <div className="flex-1 space-y-1.5">
            <label className="block text-xs font-medium text-zinc-600 dark:text-zinc-300">
              {t("processRoute.process")}
            </label>
            <input
              value={process}
              onChange={(e) => setProcess(e.target.value)}
              placeholder={t("processRoute.process")}
              className={inputCls + " w-full"}
            />
          </div>
          <div className="space-y-1.5">
            <label className="block text-xs font-medium text-zinc-600 dark:text-zinc-300">
              {t("processRoute.target")}
            </label>
            <input
              type="number"
              min={0}
              max={Math.max(0, laneCount - 1)}
              value={target}
              onChange={(e) => setTarget(Number(e.target.value))}
              aria-label={t("processRoute.target")}
              className={inputCls + " w-24"}
            />
          </div>
          <button
            onClick={() => {
              if (process.trim()) {
                const tgt = Math.max(0, Math.min(laneCount - 1, Math.trunc(target)));
                addRoute(process.trim(), tgt);
                setProcess("");
                setTarget(0);
              }
            }}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium transition-colors"
          >
            <Plus size={14} strokeWidth={2} />
            {t("processRoute.add")}
          </button>
        </div>
      </div>

      {routes.length === 0 ? (
        <div className="rounded-lg border border-dashed border-zinc-300 dark:border-zinc-700 py-8 flex flex-col items-center gap-2 text-zinc-400 dark:text-zinc-500">
          <Inbox size={20} strokeWidth={1.5} />
          <span className="text-xs">{t("processRoute.empty")}</span>
        </div>
      ) : (
        <ul className="rounded-lg border border-zinc-200 dark:border-zinc-800 divide-y divide-zinc-200 dark:divide-zinc-800 overflow-hidden">
          {routes.map((r) => (
            <li
              key={r.id}
              className="flex items-center justify-between px-4 py-2.5 text-sm bg-white dark:bg-zinc-900/60"
            >
              <span className="flex items-center gap-2 font-mono text-xs">
                {r.process}
                <ArrowRight size={12} className="text-zinc-400" strokeWidth={1.75} />
                <span className="text-zinc-600 dark:text-zinc-300">
                  {t("topology.lane", { index: r.targetLane })}
                </span>
              </span>
              <button
                onClick={() => removeRoute(r.id)}
                className="inline-flex items-center justify-center w-7 h-7 rounded-md text-zinc-400 hover:text-red-600 hover:bg-red-500/10 transition-colors"
                aria-label={t("common.delete")}
              >
                <Trash2 size={14} strokeWidth={1.75} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
