import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState } from "react";
import { Globe, Activity, Server, Save, Check, FolderOpen, ScrollText, CloudUpload, Loader2, Download, Upload } from "lucide-react";
import { openPath } from "@tauri-apps/plugin-opener";
import { ipcBackupCreate, ipcBackupUpload, ipcConfigExport, ipcConfigImport, ipcWhiteboxPath, ipcWhiteboxReload, ipcWhiteboxGet, ipcWhiteboxSaveNetwork, type NetworkConfig } from "../lib/ipc";
import { useAppStore, type Locale, type Theme } from "../store/appStore";
import { translateError } from "../lib/i18n-error";
import { ipcGetSidecarStatus, type SidecarStatus } from "../lib/ipc";
import {
  saveLocale,
  saveTheme,
  loadWebdavConfig,
  saveWebdavConfig,
  loadIpReputationConfig,
  saveIpReputationConfig,
  type IpReputationConfig,
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
  const locale = useAppStore((s) => s.locale);
  const setLocale = useAppStore((s) => s.setLocale);
  const theme = useAppStore((s) => s.theme);
  const setTheme = useAppStore((s) => s.setTheme);
  const [saved, setSaved] = useState(false);
  const [busy, setBusy] = useState(false);
  // Network settings (problem 6 parity): loaded from tauri-plugin-store on
  // mount, persisted via the Save button. The Rust shell reads these keys
  // T3-A2: gatewayBind/mihomoApi were dead fields (main.rs L173 dropped cfg).
  // MihomoController::new which refuses non-loopback URLs (§7.6).
  // T3-A2: gatewayBind/mihomoApi are dead fields (main.rs L173 drops cfg).
  // Replaced with read-only Resin sidecar actual port display.
  const [sidecarStatus, setSidecarStatus] = useState<SidecarStatus | null>(null);
  // T6-3: Network-layer whitebox config (DNS + idle + probe + bypass).
  const [netCfg, setNetCfg] = useState<NetworkConfig>({});
  const [netBusy, setNetBusy] = useState(false);
  const [netMsg, setNetMsg] = useState("");
  // T6-7: Network diagnostics panel state.
  const [reputationConfig, setReputationConfig] = useState<IpReputationConfig>({ provider: "", ipQualityScoreApiKey: "", abuseIpDbApiKey: "" });
  // C2-8: dirty-state tracking — baseline snapshot vs current form values.
  // idiomatic enterprise pattern (minimal baseline+JSON.stringify diff, no RHF dep).
  const [baseline, setBaseline] = useState({ reputationConfig: { provider: "", ipQualityScoreApiKey: "", abuseIpDbApiKey: "" } });
  // WebDAV backup config (clash-verge-rev pattern)
  const [backupUrl, setBackupUrl] = useState("");
  const [whiteboxPath, setWhiteboxPath] = useState("");
  const [whiteboxBusy, setWhiteboxBusy] = useState(false);
  const [backupUser, setBackupUser] = useState("");
  const [backupPass, setBackupPass] = useState("");
  const [backupBusy, setBackupBusy] = useState(false);
  const [backupMsg, setBackupMsg] = useState("");
  const [configBusy, setConfigBusy] = useState(false);
  // surface it. Swallow errors (vitest, sidecar not running, IPC not registered).
  const [configMsg, setConfigMsg] = useState("");

  useEffect(() => {
    void ipcWhiteboxPath().then(setWhiteboxPath).catch(() => setWhiteboxPath(""));
  }, []);

  async function reloadWhitebox() {
    setWhiteboxBusy(true);
    try {
      const n = await ipcWhiteboxReload();
      setConfigMsg(t("settings.whiteboxReloaded", { count: n }));
      const p = await ipcWhiteboxPath().catch(() => whiteboxPath);
      setWhiteboxPath(p);
    } catch (e) {
      setConfigMsg(translateError(e, t));
    } finally {
      setWhiteboxBusy(false);
    }
  }

  // Hydrate persisted network settings on mount (webview only; no-op in vitest).
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const [status, reputation] = await Promise.all([ipcGetSidecarStatus().catch(() => null), loadIpReputationConfig()]);
    if (cancelled) return;
    if (status) setSidecarStatus(status);
    setReputationConfig(reputation);
    setBaseline({ reputationConfig: reputation });
    // T6-3: also load network-layer config from whitebox
    void ipcWhiteboxGet().then((wb) => setNetCfg(wb.network ?? {})).catch(() => {});
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

  // Persist network + backup config to tauri-plugin-store.
  // Client-side guards are UX-only; Rust side is the trust boundary
  // (MihomoController::new refuses non-loopback URLs, AGENTS §7.6).
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
      setBackupMsg(t("backup.failed") + ": " + translateError(e, t));
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
      a.download = "egressapikey-config.json";
      a.click();
      URL.revokeObjectURL(url);
      setConfigMsg(t("config.exported"));
    } catch (e) {
      setConfigMsg(translateError(e, t));
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
        setConfigMsg(t("config.importError") + ": " + translateError(e, t));
      } finally {
        setConfigBusy(false);
        setTimeout(() => setConfigMsg(""), 5000);
      }
    };
   input.click();
 };

 // C2-8: isDirty = baseline vs current form snapshot. showSaveBar gates the sticky bar.
 const isDirty = useMemo(() => JSON.stringify({ reputationConfig }) !== JSON.stringify(baseline), [reputationConfig, baseline]);
 const showSaveBar = isDirty || busy || saved;

 // T6-3: Save network-layer config to whitebox JSON.
  const saveNetwork = async () => {
    setNetBusy(true);
    setNetMsg("");
    try {
      await ipcWhiteboxSaveNetwork(netCfg);
      setNetMsg(t("networkLayer.saved"));
      setTimeout(() => setNetMsg(""), 2000);
    } catch (e) {
      setNetMsg(translateError(e, t));
    } finally {
      setNetBusy(false);
    }
  };

  const resetNetwork = () => {
    setNetCfg({});
    setNetMsg(t("networkLayer.reset"));
    setTimeout(() => setNetMsg(""), 2000);
  };



 const saveAll = async () => {
   setBusy(true);
    try {
    // T3-A2: gatewayBind/mihomoApi removed — sidecar port is auto-assigned.
    await saveIpReputationConfig(reputationConfig);
    setBaseline({ reputationConfig });
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

      <SectionCard icon={<Server size={16} strokeWidth={1.75} />} title={t("settings.network")}>
        <Field label={t("settings.resinPort")}>
          <p className="text-sm text-zinc-700 dark:text-zinc-300" data-testid="sidecar-port">
            {sidecarStatus ? sidecarStatus.api_port : t("settings.sidecarLoading")}
          </p>
        </Field>
        <Field label={t("settings.resinStatus")}>
          <p className="text-sm text-zinc-700 dark:text-zinc-300" data-testid="sidecar-mode">
            {sidecarStatus ? sidecarStatus.mode : t("settings.sidecarLoading")}
          </p>
        </Field>
        <p className="text-xs text-zinc-500 dark:text-zinc-400">
          {t("settings.sidecarAutoPort")}
        </p>
      </SectionCard>
      <SectionCard icon={<Server size={16} strokeWidth={1.75} />} title={t("networkLayer.title")}>
        <Field label={t("networkLayer.dnsUpstreams")} hint={t("networkLayer.dnsHint")}>
          <textarea
            data-testid="net-dns-upstreams"
            aria-label={t("networkLayer.dnsUpstreams")}
            value={(netCfg.dns_upstreams ?? []).join("\n")}
            onChange={(e) => setNetCfg((c) => ({ ...c, dns_upstreams: e.target.value.split("\n").map((x) => x.trim()).filter(Boolean) }))}
            rows={3}
            className={inputCls + " max-w-none font-mono text-xs"}
            placeholder="https://1.1.1.1/dns-query"
          />
        </Field>
        <div className="grid grid-cols-2 gap-3">
          <Field label={t("networkLayer.maxIdleConns")}>
            <input type="number" min={1} data-testid="net-max-idle-conns" aria-label={t("networkLayer.maxIdleConns")} value={netCfg.max_idle_conns ?? ""} onChange={(e) => setNetCfg((c) => ({ ...c, max_idle_conns: e.target.value ? Number(e.target.value) : undefined }))} className={inputCls} />
          </Field>
          <Field label={t("networkLayer.maxIdleConnsPerHost")}>
            <input type="number" min={1} data-testid="net-max-idle-conns-per-host" aria-label={t("networkLayer.maxIdleConnsPerHost")} value={netCfg.max_idle_conns_per_host ?? ""} onChange={(e) => setNetCfg((c) => ({ ...c, max_idle_conns_per_host: e.target.value ? Number(e.target.value) : undefined }))} className={inputCls} />
          </Field>
        </div>
        <div className="grid grid-cols-3 gap-3">
          <Field label={t("networkLayer.idleConnTimeout")}>
            <input type="number" min={1} data-testid="net-idle-conn-timeout" aria-label={t("networkLayer.idleConnTimeout")} value={netCfg.idle_conn_timeout_secs ?? ""} onChange={(e) => setNetCfg((c) => ({ ...c, idle_conn_timeout_secs: e.target.value ? Number(e.target.value) : undefined }))} className={inputCls} />
          </Field>
          <Field label={t("networkLayer.probeTimeout")}>
            <input type="number" min={1} data-testid="net-probe-timeout" aria-label={t("networkLayer.probeTimeout")} value={netCfg.probe_timeout_secs ?? ""} onChange={(e) => setNetCfg((c) => ({ ...c, probe_timeout_secs: e.target.value ? Number(e.target.value) : undefined }))} className={inputCls} />
          </Field>
          <Field label={t("networkLayer.probeConcurrency")}>
            <input type="number" min={1} max={10000} data-testid="net-probe-concurrency" aria-label={t("networkLayer.probeConcurrency")} value={netCfg.probe_concurrency ?? ""} onChange={(e) => setNetCfg((c) => ({ ...c, probe_concurrency: e.target.value ? Number(e.target.value) : undefined }))} className={inputCls} />
          </Field>
        </div>
        <Field label={t("networkLayer.proxyBypass")} hint={t("networkLayer.bypassHint")}>
          <textarea
            data-testid="net-proxy-bypass"
            aria-label={t("networkLayer.proxyBypass")}
            value={(netCfg.proxy_bypass ?? []).join("\n")}
            onChange={(e) => setNetCfg((c) => ({ ...c, proxy_bypass: e.target.value.split("\n").map((x) => x.trim()).filter(Boolean) }))}
            rows={2}
            className={inputCls + " max-w-none font-mono text-xs"}
            placeholder="*.local,127.0.0.1"
          />
        </Field>
        <div className="flex items-center gap-2 pt-1">
          <button data-testid="net-save-btn" onClick={() => void saveNetwork()} disabled={netBusy} className={btnCls}>
            {netBusy ? <Loader2 size={14} className="animate-spin" /> : <Save size={14} strokeWidth={1.75} />}
            {t("networkLayer.save")}
          </button>
          <button data-testid="net-reset-btn" onClick={resetNetwork} className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 hover:bg-zinc-50 dark:hover:bg-zinc-800 text-sm font-medium transition-colors">
            {t("networkLayer.resetBtn")}
          </button>
          {netMsg ? <span className="text-xs text-zinc-500" data-testid="net-msg">{netMsg}</span> : null}
        </div>
      </SectionCard>
      <SectionCard icon={<Activity size={16} strokeWidth={1.75} />} title={t("settings.ipReputation")}>
        <Field label={t("settings.ipReputationProvider")} hint={t("settings.ipReputationHelp")}>
          <select aria-label={t("settings.ipReputationProvider")} value={reputationConfig.provider} onChange={(e) => setReputationConfig((current) => ({ ...current, provider: e.target.value as IpReputationConfig["provider"] }))} className={inputCls}>
            <option value="">{t("settings.ipReputationDisabled")}</option>
            <option value="ip_quality_score">{t("settings.ipQualityScore")}</option>
            <option value="abuse_ip_db">{t("settings.abuseIpDb")}</option>
            <option value="ip_api">{t("settings.ipApi")}</option>
          </select>
        </Field>
        {reputationConfig.provider === "ip_quality_score" ? <Field label={t("settings.ipReputationKey")}><input aria-label={t("settings.ipReputationKey")} type="password" autoComplete="off" value={reputationConfig.ipQualityScoreApiKey} onChange={(e) => setReputationConfig((current) => ({ ...current, ipQualityScoreApiKey: e.target.value }))} className={inputCls} /></Field> : null}
        {reputationConfig.provider === "abuse_ip_db" ? <Field label={t("settings.ipReputationKey")}><input aria-label={t("settings.ipReputationKey")} type="password" autoComplete="off" value={reputationConfig.abuseIpDbApiKey} onChange={(e) => setReputationConfig((current) => ({ ...current, abuseIpDbApiKey: e.target.value }))} className={inputCls} /></Field> : null}
        {reputationConfig.provider === "ip_api" ? <p className="text-xs text-amber-700 dark:text-amber-300">{t("settings.ipApiWarning")}</p> : null}
      </SectionCard>

      <SectionCard icon={<FolderOpen size={16} strokeWidth={1.75} />} title={t("settings.storage")}>
        <div className="flex flex-col gap-3 sm:flex-row sm:flex-wrap sm:gap-3">
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
          <button
            data-testid="settings-whitebox-reload"
            onClick={() => void reloadWhitebox()}
            disabled={whiteboxBusy}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 hover:bg-zinc-50 dark:hover:bg-zinc-800 text-sm font-medium transition-colors disabled:opacity-50"
          >
            {whiteboxBusy ? <Loader2 size={14} className="animate-spin" /> : <Download size={14} strokeWidth={1.75} />}
            {t("settings.whiteboxReload")}
          </button>
        </div>
        {whiteboxPath ? (
          <p className="mt-2 text-xs text-zinc-500 dark:text-zinc-400 break-all" data-testid="settings-whitebox-path">
            {t("settings.whiteboxPath")}: {whiteboxPath}
          </p>
        ) : null}
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
      {showSaveBar && (
      <div className="sticky bottom-0 left-0 right-0 mt-4 px-4 py-3 bg-white/85 dark:bg-zinc-900/80 backdrop-blur border-t border-zinc-200 dark:border-zinc-800 flex items-center justify-end gap-2" data-testid="settings-save-bar">
        <span className="text-xs text-zinc-500 dark:text-zinc-400">{t("settings.unifiedHelp")}</span>
        <button
          data-testid="settings-save-button" onClick={() => void saveAll()}
          disabled={busy || (!isDirty && !saved)}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium transition-colors disabled:opacity-40"
        >
          {busy ? <Loader2 size={14} className="animate-spin" /> : saved ? <Check size={14} strokeWidth={2.5} /> : <Save size={14} strokeWidth={2} />}
          {t("settings.save")}
        </button>
      </div>
      )}
    </section>

      
  );
}
