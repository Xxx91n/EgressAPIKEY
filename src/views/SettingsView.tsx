import { useTranslation } from "react-i18next";
import { useState } from "react";
import { useAppStore } from "../store/appStore";

export function SettingsView() {
  const { t, i18n } = useTranslation();
  const laneCount = useAppStore((s) => s.laneCount);
  const setLaneCount = useAppStore((s) => s.setLaneCount);
  const locale = useAppStore((s) => s.locale);
  const setLocale = useAppStore((s) => s.setLocale);
  const [lanes, setLanes] = useState(laneCount);

  return (
    <section className="max-w-xl space-y-6">
      <div>
        <h2 className="font-medium">{t("settings.general")}</h2>
      </div>
      <div className="space-y-2">
        <label className="block text-sm font-medium">{t("settings.language")}</label>
        <select
          value={locale}
          onChange={(e) => {
            const next = e.target.value as "en" | "zh" | "ja" | "es" | "fr";
            setLocale(next);
            void i18n.changeLanguage(next);
          }}
          className="border border-zinc-300 dark:border-zinc-700 rounded px-2 py-1 bg-white dark:bg-zinc-900"
        >
          <option value="en">English</option>
          <option value="zh">中文</option>
          <option value="ja">日本語</option>
          <option value="es">Español</option>
          <option value="fr">Français</option>
        </select>
      </div>
      <div className="space-y-2">
        <label className="block text-sm font-medium">{t("settings.lanes")}</label>
        <input
          type="number"
          min={1}
          max={50}
          value={lanes}
          onChange={(e) => setLanes(Number(e.target.value))}
          className="border border-zinc-300 dark:border-zinc-700 rounded px-2 py-1 bg-white dark:bg-zinc-900 w-24"
        />
        <button
          onClick={() => setLaneCount(lanes)}
          className="ml-2 px-3 py-1 rounded bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900 text-sm"
        >
          {t("settings.save")}
        </button>
        <p className="text-xs text-zinc-500 dark:text-zinc-400">1..50</p>
      </div>
    </section>
  );
}
