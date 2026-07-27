import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";
import { useAppStore, type Locale, type Theme } from "../store/appStore";
import { saveLocale, saveTheme } from "../lib/settings";

const LOCALES: Locale[] = ["en", "zh", "ja", "es", "fr", "de", "ko", "ru", "pt", "ar"];

/// Native endonym for each locale, shown in the language <select>.
const LOCALE_ENDONYM: Record<Locale, string> = {
  en: "English",
  zh: "中文",
  ja: "日本語",
  es: "Español",
  fr: "Français",
  de: "Deutsch",
  ko: "한국어",
  ru: "Русский",
  pt: "Português",
  ar: "العربية",
};

const THEMES: Theme[] = ["light", "dark", "system"];

export function SettingsView() {
  const { t, i18n } = useTranslation();
  const laneCount = useAppStore((s) => s.laneCount);
  const setLaneCount = useAppStore((s) => s.setLaneCount);
  const locale = useAppStore((s) => s.locale);
  const setLocale = useAppStore((s) => s.setLocale);
  const theme = useAppStore((s) => s.theme);
  const setTheme = useAppStore((s) => s.setTheme);
  const [lanes, setLanes] = useState(laneCount);

  const changeLocale = async (next: Locale) => {
    setLocale(next);
    await i18n.changeLanguage(next);
    void saveLocale(next);
    // Re-localise the OS tray menu; best-effort, ignored outside Tauri.
    void invoke("tray_refresh_labels").catch(() => {});
  };

  const changeTheme = async (next: Theme) => {
    setTheme(next);
    void saveTheme(next);
  };

  return (
    <section className="max-w-xl space-y-6">
      <div>
        <h2 className="font-medium">{t("settings.general")}</h2>
      </div>
      <div className="space-y-2">
        <label className="block text-sm font-medium">{t("settings.language")}</label>
        <select
          value={locale}
          onChange={(e) => void changeLocale(e.target.value as Locale)}
          className="border border-zinc-300 dark:border-zinc-700 rounded px-2 py-1 bg-white dark:bg-zinc-900"
        >
          {LOCALES.map((lc) => (
            <option key={lc} value={lc}>
              {LOCALE_ENDONYM[lc]}
            </option>
          ))}
        </select>
      </div>
      <div className="space-y-2">
        <label className="block text-sm font-medium">{t("theme.label")}</label>
        <select
          value={theme}
          onChange={(e) => void changeTheme(e.target.value as Theme)}
          className="border border-zinc-300 dark:border-zinc-700 rounded px-2 py-1 bg-white dark:bg-zinc-900"
        >
          {THEMES.map((th) => (
            <option key={th} value={th}>
              {t(`theme.${th}`)}
            </option>
          ))}
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
