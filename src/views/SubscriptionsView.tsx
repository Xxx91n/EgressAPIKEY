import { useTranslation } from "react-i18next";
import { useState } from "react";
import { useAppStore } from "../store/appStore";

export function SubscriptionsView() {
  const { t } = useTranslation();
  const subscriptions = useAppStore((s) => s.subscriptions);
  const lanes = useAppStore((s) => s.laneCount);
  const addSubscription = useAppStore((s) => s.addSubscription);
  const [url, setUrl] = useState("");

  return (
    <section className="max-w-2xl space-y-4">
      <header>
        <h2 className="font-medium">{t("subscription.title")}</h2>
      </header>
      <div className="flex gap-2">
        <input
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder={t("subscription.url")}
          className="flex-1 border border-zinc-300 dark:border-zinc-700 rounded px-2 py-1 bg-white dark:bg-zinc-900"
        />
        <button
          onClick={() => {
            // P3 stub: real import calls resin-core compile_subscription via Tauri IPC.
            if (url.trim()) {
              addSubscription(url.trim(), 0, lanes);
              setUrl("");
            }
          }}
          className="px-3 py-1 rounded bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900 text-sm"
        >
          {t("subscription.import")}
        </button>
      </div>
      {subscriptions.map((s) => (
        <div
          key={s.id}
          className="border border-zinc-200 dark:border-zinc-800 rounded px-3 py-2 text-sm"
        >
          {s.url} - {t("subscription.imported", { count: s.nodeCount, lanes: s.lanes })}
        </div>
      ))}
    </section>
  );
}
