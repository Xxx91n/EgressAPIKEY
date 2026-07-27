import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";
import { Globe, Activity, Server, Save, Check } from "lucide-react";
import { useAppStore, type Locale, type Theme } from "../store/appStore";
import { saveLocale, saveTheme, saveLaneCount } from "../lib/settings";

const LOCALES: Locale[] = ["en", "zh", "es", "fr", "de", "ja", "ko", "ru", "pt", "it", "nl", "pl", "tr", "ar", "vi", "th", "id", "hi"];

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
  it: "Italiano",
  nl: "Nederlands",
  pl: "Polski",
  tr: "Türkçe",
  vi: "Tiếng Việt",
  th: "ไทย",
  id: "Bahasa Indonesia",
  hi: "हिन्दी",
};

const THEMES: Theme[] = ["light", "dark", "system"];

function SectionCard({
  icon,
  title,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900/60">
      <div className="flex items-center gap-2 px-4 py-3 border-b border-zinc-200 dark:border-zinc-800">
        <span className="text-zinc-500 dark:text-zinc-400">{icon}</span>
        <h3 className="text-sm font-semibold tracking-tight">{title}</h3>
      </div>
      <div className="p-4 space-y-4">{children}</div>
    </div>
  );
}

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="space-y-1.5">
      <label className="block text-xs font-medium text-zinc-600 dark:text-zinc-300">{label}</label>
      {children}
      {hint ? <p className="text-[11px] text-zinc-400 dark:text-zinc-500">{hint}</p> : null}
    </div>
  );
}

export function SettingsView() {
  const { t, i18n } = useTranslation();
  const laneCount = useAppStore((s) => s.laneCount);
  const setLaneCount = useAppStore((s) => s.setLaneCount);
  const locale = useAppStore((s) => s.locale);
  const setLocale = useAppStore((s) => s.setLocale);
  const theme = useAppStore((s) => s.theme);
  const setTheme = useAppStore((s) => s.setTheme);
  const [lanes, setLanes] = useState(laneCount);
  const [saved, setSaved] = useState(false);

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

  // Persist laneCount to tauri-plugin-store (problem 6 fix: the old Save
  // button only updated the in-memory store, so the count was lost on quit).
  const saveAll = async () => {
    const n = Math.max(1, Math.min(50, Math.trunc(lanes)));
    setLanes(n);
    setLaneCount(n);
    await saveLaneCount(n);
    setSaved(true);
    setTimeout(() => setSaved(false), 1500);
  };

  const selectCls =
    "w-56 rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-2.5 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500/40";

  return (
    <section className="max-w-2xl space-y-5">
      <SectionCard icon={<Globe size={16} strokeWidth={1.75} />} title={t("settings.general")}>
        <Field label={t("settings.language")}>
          <select
            value={locale}
            onChange={(e) => void changeLocale(e.target.value as Locale)}
            className={selectCls}
          >
            {LOCALES.map((lc) => (
              <option key={lc} value={lc}>
                {LOCALE_ENDONYM[lc]}
              </option>
            ))}
          </select>
        </Field>
        <Field label={t("theme.label")}>
          <select
            value={theme}
            onChange={(e) => void changeTheme(e.target.value as Theme)}
            className={selectCls}
          >
            {THEMES.map((th) => (
              <option key={th} value={th}>
                {t(`theme.${th}`)}
              </option>
            ))}
          </select>
        </Field>
      </SectionCard>

      <SectionCard icon={<Activity size={16} strokeWidth={1.75} />} title={t("settings.advanced")}>
        <Field label={t("settings.lanes")} hint="1..50">
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={1}
              max={50}
              value={lanes}
              onChange={(e) => setLanes(Number(e.target.value))}
              className="w-24 rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-2.5 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500/40"
            />
            <button
              onClick={() => void saveAll()}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium transition-colors"
            >
              {saved ? <Check size={14} strokeWidth={2.5} /> : <Save size={14} strokeWidth={2} />}
              {t("settings.save")}
            </button>
          </div>
        </Field>
      </SectionCard>

      <SectionCard icon={<Server size={16} strokeWidth={1.75} />} title={t("settings.network")}>
        <Field label={t("settings.gatewayBind")} hint="127.0.0.1:10086">
          <input
            type="text"
            placeholder="127.0.0.1:10086"
            className="w-56 rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-2.5 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500/40"
          />
        </Field>
        <Field label={t("settings.mihomoApi")} hint="http://127.0.0.1:9090">
          <input
            type="text"
            placeholder="http://127.0.0.1:9090"
            className="w-56 rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-2.5 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500/40"
          />
        </Field>
      </SectionCard>
    </section>
  );
}
