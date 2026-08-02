import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { Globe, Activity, Server, Save, Check, FolderOpen, ScrollText, CloudUpload, Loader2, Download, Upload } from "lucide-react";
import { openPath } from "@tauri-apps/plugin-opener";
import { ipcBackupCreate, ipcBackupUpload, ipcConfigExport, ipcConfigImport, ipcInterceptorPort } from "../lib/ipc";
import { useAppStore, type Locale, type Theme } from "../store/appStore";
import {
  saveLocale,
  saveTheme,
  saveLaneCount,
  loadGatewayBind,
  saveGatewayBind,
  loadMihomoApi,
  saveMihomoApi,
  loadWebdavConfig,
  saveWebdavConfig,
} from "../lib/settings";

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
  const [busy, setBusy] = useState(false);
  // Network settings (problem 6 parity): loaded from tauri-plugin-store on
  // mount, persisted via the Save button. The Rust shell reads these keys
  // (gatewayBind, mihomoApi) at startup into CoreConfig; mihomoApi flows to
  // MihomoController::new which refuses non-loopback URLs (§7.6).
  const [gatewayBind, setGatewayBind] = useState("127.0.0.1:7897");
  const [mihomoApi, setMihomoApi] = useState("http://127.0.0.1:9090");
  // WebDAV backup config (clash-verge-rev pattern)
  const [backupUrl, setBackupUrl] = useState("");
  const [backupUser, setBackupUser] = useState("");
  const [backupPass, setBackupPass] = useState("");
  const [backupBusy, setBackupBusy] = useState(false);
  const [backupMsg, setBackupMsg] = useState("");
  // A4-3: interceptor port — bind-then-bind state. Displayed so the user
  // knows what to fill in omniroute/litellm as base_url. Read-only here; the
  // Rust side owns the port and rebinds only on app restart.
  const [interceptPort, setInterceptPort] = useState<number>(0);
  const [configBusy, setConfigBusy] = useState(false);
  // A4-3: request the interceptor port once on mount so the Settings page can
  // surface it. Swallow errors (vitest, sidecar not running, IPC not registered).
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const p = await ipcInterceptorPort();
        if (p > 0 && !cancelled) setInterceptPort(p);
      } catch {
        // Outside Tauri / sidecar down: keep 0.
      }
    })();
    return () => { cancelled = true; };
  }, []);
  const [configMsg, setConfigMsg] = useState("");

  // Hydrate persisted network settings on mount (webview only; no-op in vitest).
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const [b, m] = await Promise.all([loadGatewayBind(), loadMihomoApi()]);
      if (cancelled) return;
      if (b) setGatewayBind(b);
      if (m) setMihomoApi(m);
    })();
    return () => { cancelled = true; };
  }, []);

   // Hydrate WebDAV backup config
  useEffect(() => {
    let c2 = false;
    void (async () => {
      const cfg = await loadWebdavConfig();
      if (c2 || !cfg) return;
      setBackupUrl(cfg.url);
      setBackupUser(cfg.username);
      setBackupPass(cfg.password);
    })();
    return () => { c2 = true; };
  }, []);

  const changeLocale = async (next: Locale) => {
    setLocale(next);
    await i18n.changeLanguage(next);
    await saveLocale(next);
    // Re-localise the OS tray AFTER the persisted lang is flushed so current_lang
    // reads the new value (race fix for tray i18n lag). Best-effort outside Tauri.
    await invoke("tray_refresh_labels").catch(() => {});
  };

  const changeTheme = async (next: Theme) => {
    setTheme(next);
    void saveTheme(next);
  };

  // Persist laneCount + network settings to tauri-plugin-store (problem 6 fix:
  // the old Save button only updated the in-memory store, so the count was lost
  // on quit). Client-side guards here are UX-only: trim, a `^https?://` shape
  // check on mihomoApi, and a length cap so we never persist a multi-MB string.
  // The Rust side is the real trust boundary (MihomoController::new refuses
  // non-loopback URLs, see crates/resin-core/src/mihomo.rs + AGENTS §7.6).
  const saveWebdav = async () => {
    await saveWebdavConfig(backupUrl.trim(), backupUser.trim(), backupPass);
    setBackupMsg(t("backup.success"));
    setTimeout(() => setBackupMsg(""), 2000);
  };

  const doBackup = async () => {
    if (!backupUrl.trim()) { setBackupMsg(t("backup.noConfig")); return; }
    setBackupBusy(true);
    setBackupMsg("");
    try {
      const zipPath = await ipcBackupCreate();
      await ipcBackupUpload(backupUrl.trim(), backupUser.trim(), backupPass, zipPath);
      setBackupMsg(t("backup.success"));
    } catch (e) {
      setBackupMsg(t("backup.failed") + ": " + String(e));
    } finally {
      setBackupBusy(false);
      setTimeout(() => setBackupMsg(""), 3000);
    }
  };

  const doConfigExport = async () => {
    setConfigBusy(true);
    setConfigMsg("");
    try {
      const config = await ipcConfigExport();
      const blob = new Blob([JSON.stringify(config, null, 2)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "ai-api-route-config.json";
      a.click();
      URL.revokeObjectURL(url);
      setConfigMsg(t("config.exported"));
    } catch (e) {
      setConfigMsg(String(e));
    } finally {
      setConfigBusy(false);
      setTimeout(() => setConfigMsg(""), 3000);
    }
  };

  const doConfigImport = async () => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".json,application/json";
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) return;
      setConfigBusy(true);
      setConfigMsg("");
      try {
        const text = await file.text();
        const config = JSON.parse(text);
        const result = await ipcConfigImport(config);
        setConfigMsg(t("config.imported", { platforms: result.platforms_created, subscriptions: result.subscriptions_created }));
      } catch (e) {
        setConfigMsg(t("config.importError") + ": " + String(e));
      } finally {
        setConfigBusy(false);
        setTimeout(() => setConfigMsg(""), 5000);
      }
    };
    input.click();
  };

  const saveAll = async () => {
    setBusy(true);
    try {
    const n = Math.max(1, Math.min(50, Math.trunc(lanes)));
    setLanes(n);
    setLaneCount(n);
    await saveLaneCount(n);

    const rawBind = gatewayBind.trim().slice(0, 2048);
    const bind = rawBind || "127.0.0.1:7897";
    const rawApi = mihomoApi.trim().slice(0, 2048);
    // Reject an mihomoApi that is not an http(s) URL shape (UX only; the Rust
    // loopback guard is still the authoritative check). On bad shape we keep
    // the canonical default so the saved store never holds garbage.
    const looksLikeUrl = /^https?:\/\//i.test(rawApi);
    const api = looksLikeUrl ? rawApi : "http://127.0.0.1:9090";
    setGatewayBind(bind);
    setMihomoApi(api);
    await Promise.all([saveGatewayBind(bind), saveMihomoApi(api)]);
    setSaved(true);
    setTimeout(() => setSaved(false), 1500);
    } finally { setBusy(false); }
  };

  const openDir = async (which: "config" | "log") => {
    try {
      const dir = await invoke<string>(which === "config" ? "get_config_dir" : "get_log_dir");
      if (dir) await openPath(dir);
    } catch {
      /* not in tauri (vitest) or path unresolved — no-op */
    }
  };

  const selectCls =
    "w-56 rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-2.5 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500/40";
  const inputCls =
    "w-full max-w-sm rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-2.5 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500/40";
  const btnCls =
    "inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-blue-600 hover:bg-blue-700 text-white text-sm";

  return (
    <section className="w-full max-w-none px-6 space-y-5">
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
            {/* Issue 6: unified Save moved to the sticky bottom bar so the user
                sees a single save action for the whole settings panel. */}
          </div>
        </Field>
      </SectionCard>

      <SectionCard icon={<Server size={16} strokeWidth={1.75} />} title={t("settings.network")}>
        <Field label={t("settings.gatewayBind")}>
          <input
            type="text"
            value={gatewayBind}
            onChange={(e) => setGatewayBind(e.target.value)}
            placeholder="127.0.0.1:7897"
            className="w-56 rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-2.5 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500/40"
          />
        </Field>
        <Field label={t("settings.mihomoApi")}>
          <input
            type="text"
            value={mihomoApi}
            onChange={(e) => setMihomoApi(e.target.value)}
            placeholder="http://127.0.0.1:9090"
            className="w-56 rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-2.5 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500/40"
          />
        </Field>
        {/* Issue 6: per-card Save removed; the unified sticky bottom bar
            calls saveAll() so there is one obvious commit action. */}
      </SectionCard>
      <SectionCard icon={<Server size={16} strokeWidth={1.75} />} title={t("settings.interceptPort")}>
        <div className="text-sm">
          <div className="font-mono text-blue-600 dark:text-blue-400">
            {interceptPort > 0 ? "http://127.0.0.1:" + interceptPort : "—"}
          </div>
          <p className="mt-1 text-xs text-zinc-500 dark:text-zinc-400 max-w-md">
            {t("settings.interceptPortHint")}
          </p>
        </div>
      </SectionCard>
      <SectionCard icon={<FolderOpen size={16} strokeWidth={1.75} />} title={t("settings.storage")}>
        <div className="flex flex-col gap-3 sm:flex-row sm:gap-3">
          <button
            onClick={() => void openDir("config")}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 hover:bg-zinc-50 dark:hover:bg-zinc-800 text-sm font-medium transition-colors"
          >
            <FolderOpen size={14} strokeWidth={1.75} />
            {t("settings.openConfigDir")}
          </button>
          <button
            onClick={() => void openDir("log")}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 hover:bg-zinc-50 dark:hover:bg-zinc-800 text-sm font-medium transition-colors"
          >
            <ScrollText size={14} strokeWidth={1.75} />
            {t("settings.openLogDir")}
          </button>
        </div>
      </SectionCard>
          <SectionCard icon={<CloudUpload size={16} strokeWidth={1.75} />} title={t("backup.title")}>
        <Field label={t("backup.webdavUrl")}>
          <input
            type="text"
            value={backupUrl}
            onChange={(e) => setBackupUrl(e.target.value)}
            placeholder="https://app.koofr.net/dav/Koofr"
            className={inputCls}
          />
        </Field>
        <div className="grid grid-cols-2 gap-3">
          <Field label={t("backup.webdavUser")}>
            <input
              type="text"
              value={backupUser}
              onChange={(e) => setBackupUser(e.target.value)}
              className={inputCls}
            />
          </Field>
          <Field label={t("backup.webdavPass")}>
            <input
              type="password"
              value={backupPass}
              onChange={(e) => setBackupPass(e.target.value)}
              className={inputCls}
            />
          </Field>
        </div>
        <div className="flex items-center gap-2 pt-1">
          <button onClick={() => void saveWebdav()} className={btnCls}>
            <Save size={14} strokeWidth={1.75} />
            {t("backup.save")}
          </button>
          <button onClick={() => void doBackup()} disabled={backupBusy} className={btnCls}>
            {backupBusy ? <Loader2 size={14} strokeWidth={1.75} className="animate-spin" /> : <CloudUpload size={14} strokeWidth={1.75} />}
            {t("backup.create")}
          </button>
          {backupMsg ? <Check size={14} className="text-green-500" /> : null}
          {backupMsg ? <span className="text-xs text-zinc-500">{backupMsg}</span> : null}
        </div>
      </SectionCard>
          <SectionCard icon={<Download size={16} strokeWidth={1.75} />} title={t("config.title")}>
        <div className="flex items-center gap-2">
          <button onClick={() => void doConfigExport()} disabled={configBusy} className={btnCls}>
            <Download size={14} strokeWidth={1.75} />
            {t("config.exportBtn")}
          </button>
          <button onClick={() => void doConfigImport()} disabled={configBusy} className={btnCls}>
            <Upload size={14} strokeWidth={1.75} />
            {t("config.importBtn")}
          </button>
          {configBusy ? <Loader2 size={14} className="animate-spin text-zinc-400" /> : null}
          {configMsg ? <span className="text-xs text-zinc-500">{configMsg}</span> : null}
        </div>
      </SectionCard>
      <div className="sticky bottom-0 left-0 right-0 mt-4 px-4 py-3 bg-white/85 dark:bg-zinc-900/80 backdrop-blur border-t border-zinc-200 dark:border-zinc-800 flex items-center justify-end gap-2">
        <span className="text-xs text-zinc-500 dark:text-zinc-400">{t("settings.unifiedHelp")}</span>
        <button
          onClick={() => void saveAll()}
          disabled={busy}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium transition-colors disabled:opacity-40"
        >
          {saved ? <Check size={14} strokeWidth={2.5} /> : <Save size={14} strokeWidth={2} />}
          {t("settings.save")}
        </button>
      </div>
    </section>

      
  );
}
