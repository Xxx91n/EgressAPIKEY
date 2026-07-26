import { useTranslation } from "react-i18next";
import { useState } from "react";
import { useAppStore } from "../store/appStore";

export function ProcessRouteView() {
  const { t } = useTranslation();
  const routes = useAppStore((s) => s.processRoutes);
  const laneCount = useAppStore((s) => s.laneCount);
  const addRoute = useAppStore((s) => s.addProcessRoute);
  const removeRoute = useAppStore((s) => s.removeProcessRoute);
  const [process, setProcess] = useState("");
  const [target, setTarget] = useState(0);

  return (
    <section className="max-w-2xl space-y-4">
      <header>
        <h2 className="font-medium">{t("processRoute.title")}</h2>
      </header>
      <div className="flex gap-2">
        <input
          value={process}
          onChange={(e) => setProcess(e.target.value)}
          placeholder={t("processRoute.process")}
          className="flex-1 border border-zinc-300 dark:border-zinc-700 rounded px-2 py-1 bg-white dark:bg-zinc-900"
        />
        <input
          type="number"
          min={0}
          max={laneCount - 1}
          value={target}
          onChange={(e) => setTarget(Number(e.target.value))}
          aria-label={t("processRoute.target")}
          className="w-24 border border-zinc-300 dark:border-zinc-700 rounded px-2 py-1 bg-white dark:bg-zinc-900"
        />
        <button
          onClick={() => {
            if (process.trim()) {
              addRoute(process.trim(), target);
              setProcess("");
            }
          }}
          className="px-3 py-1 rounded bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900 text-sm"
        >
          {t("processRoute.add")}
        </button>
      </div>
      {routes.length === 0 ? (
        <p className="text-sm text-zinc-500 dark:text-zinc-400">{t("processRoute.empty")}</p>
      ) : (
        <ul className="divide-y divide-zinc-200 dark:divide-zinc-800 border border-zinc-200 dark:border-zinc-800 rounded">
          {routes.map((r) => (
            <li key={r.id} className="flex items-center justify-between px-3 py-2 text-sm">
              <span>
                {r.process} {">"} {t("topology.lane", { index: r.targetLane })}
              </span>
              <button
                onClick={() => removeRoute(r.id)}
                className="text-xs text-red-600 hover:underline"
              >
                {t("common.delete")}
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
