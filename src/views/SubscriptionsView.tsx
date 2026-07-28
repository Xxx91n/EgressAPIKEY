import { useTranslation } from "react-i18next";
import { useState } from "react";
import { Rss, Download, Inbox } from "lucide-react";
import { useAppStore } from "../store/appStore";

export function SubscriptionsView() {
  const { t } = useTranslation();
  const subscriptions = useAppStore((s) => s.subscriptions);
  const lanes = useAppStore((s) => s.laneCount);
  const addSubscription = useAppStore((s) => s.addSubscription);
  const [url, setUrl] = useState("");

  const inputCls =
    "flex-1 rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-3 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500/40";

  return (
    <section className="max-w-2xl space-y-4">
      <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900/60">
        <div className="flex items-center gap-2 px-4 py-3 border-b border-zinc-200 dark:border-zinc-800">
          <Rss size={16} className="text-zinc-500 dark:text-zinc-400" strokeWidth={1.75} />
          <h2 className="text-sm font-semibold tracking-tight">{t("subscription.title")}</h2>
        </div>
        <div className="p-4">
        <div className="flex gap-2">
          <input
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder={t("subscription.url")}
            className={inputCls}
          />
          <button
            onClick={() => {
              // P3 stub: real import calls resin-core compile_subscription via Tauri IPC.
              if (url.trim()) {
                addSubscription(url.trim(), 0, lanes);
                setUrl("");
              }
            }}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium transition-colors"
          >
            <Download size={14} strokeWidth={2} />
            {t("subscription.import")}
          </button>
        </div>
        </div>
      </div>

      {subscriptions.length === 0 ? (
        <div className="rounded-lg border border-dashed border-zinc-300 dark:border-zinc-700 py-8 flex flex-col items-center gap-2 text-zinc-400 dark:text-zinc-500">
          <Inbox size={20} strokeWidth={1.5} />
          <span className="text-xs">{t("platform.empty")}</span>
        </div>
      ) : (
        <ul className="space-y-2">
          {subscriptions.map((s) => (
            <li
              key={s.id}
              className="rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900/60 px-4 py-3 text-sm flex items-center justify-between"
            >
              <span className="font-mono text-xs text-zinc-600 dark:text-zinc-300 truncate max-w-[60%]">
                {s.url}
              </span>
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
