import { useEffect, useState, lazy, Suspense } from "react";
import { useTranslation } from "react-i18next";
import { Navigation, Network, Settings as SettingsIcon, Route, FolderTree, RadioTower } from "lucide-react";
import { useAppStore, type Locale, type Theme } from "./store/appStore";
const TopologyView = lazy(() => import("./views/TopologyView").then(m => ({ default: m.TopologyView })));
import { SettingsView } from "./views/SettingsView";
import { ProcessRouteView } from "./views/ProcessRouteView";
import { SubscriptionsView } from "./views/SubscriptionsView";
import { PlatformsView } from "./views/PlatformsView";
import { useTheme } from "./lib/useTheme";
import { LogPanel } from "./components/LogPanel";
import { loadLocale, loadTheme, loadLaneCount, loadView, loadProcessRoutes } from "./lib/settings";

// Side rail nav: icon + label, desktop-tool density. Lucide vector icons
// (not emoji) per ui-ux-pro-max: scalable, theme-aware, consistent stroke.
const NAV_ITEMS = [
  { key: "topology", icon: Network },
  { key: "platforms", icon: FolderTree },
  { key: "processRoute", icon: Route },
  { key: "subscriptions", icon: RadioTower },
  { key: "settings", icon: SettingsIcon },
] as const;

function SideRail() {
  const { t } = useTranslation();
  const view = useAppStore((s) => s.view);
  const setView = useAppStore((s) => s.setView);

  return (
    <nav className="w-14 flex flex-col items-center gap-1 border-r border-zinc-200 dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-950 py-3 shrink-0">
      {NAV_ITEMS.map((item) => {
        const Icon = item.icon;
        const active = view === item.key;
        return (
          <button
            key={item.key}
            onClick={() => setView(item.key)}
            aria-pressed={active}
            aria-label={t(`nav.${item.key}`)}
            title={t(`nav.${item.key}`)}
            className={
              "w-9 h-9 flex items-center justify-center rounded-md transition-colors " +
              (active
                ? "bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900"
                : "text-zinc-500 hover:text-zinc-900 hover:bg-zinc-200/60 dark:text-zinc-400 dark:hover:text-zinc-100 dark:hover:bg-zinc-800")
            }
          >
            <Icon size={18} strokeWidth={1.75} />
          </button>
        );
      })}
    </nav>
  );
}

export default function App() {
  const { t, i18n } = useTranslation();
  const view = useAppStore((s) => s.view);
  const setLocale = useAppStore((s) => s.setLocale);
  const setTheme = useAppStore((s) => s.setTheme);
  const setLaneCount = useAppStore((s) => s.setLaneCount);
  const setView = useAppStore((s) => s.setView);
  const setProcessRoutes = useAppStore((s) => s.setProcessRoutes);
  useTheme();
  // Bug #1 fix: the store defaults view="topology". Without a gate, the first
  // paint renders the topology (or its lazy fallback) before loadView() resolves
  // the persisted view and setView() swaps it — the user sees a topology flash.
  // Hold a minimal loader until the persisted view has been read.
  const [bootstrapped, setBootstrapped] = useState(false);

  // Bootstrap persisted prefs once on mount (no-op outside Tauri/vitest).
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const savedLocale = await loadLocale();
      if (savedLocale && !cancelled) {
        setLocale(savedLocale as Locale);
        void i18n.changeLanguage(savedLocale);
      }
      const savedTheme = await loadTheme();
      if (savedTheme && !cancelled) {
        setTheme(savedTheme as Theme);
      }
      const savedLaneCount = await loadLaneCount();
      if (savedLaneCount && !cancelled) {
        setLaneCount(savedLaneCount);
      }
      const savedView = await loadView();
      if (savedView && !cancelled) {
        setView(savedView as any);
      }
      const savedRoutes = await loadProcessRoutes();
      if (savedRoutes && !cancelled) {
        setProcessRoutes(savedRoutes as any);
      }
      if (!cancelled) setBootstrapped(true);
    })();
    return () => { cancelled = true; };
  }, [setLocale, setTheme, setLaneCount, setView, setProcessRoutes, i18n]);

  if (!bootstrapped) {
    return (
      <div className="h-full flex items-center justify-center bg-white dark:bg-zinc-950">
        <span className="text-sm text-zinc-400">Loading…</span>
      </div>
    );
  }
  return (
    <div className="h-full flex bg-white dark:bg-zinc-950 text-zinc-900 dark:text-zinc-100">
      <SideRail />
      <div className="flex-1 flex flex-col min-w-0">
        <header className="px-5 py-3 border-b border-zinc-200 dark:border-zinc-800 flex items-center gap-2">
          <Navigation size={16} className="text-zinc-400" strokeWidth={1.75} />
          <h1 className="text-sm font-semibold tracking-tight">{t("app.title")}</h1>
          <span className="text-xs text-zinc-400 dark:text-zinc-500 hidden sm:inline">· {t("app.tagline")}</span>
        </header>
        <main className="flex-1 overflow-auto">
          {view === "topology" && <Suspense fallback={<div className="flex-1 flex items-center justify-center text-sm text-zinc-400">Loading...</div>}><TopologyView /></Suspense>}
          {view === "platforms" && <PlatformsView />}
          {view === "settings" && <SettingsView />}
          {view === "processRoute" && <ProcessRouteView />}
          {view === "subscriptions" && <SubscriptionsView />}
        </main>
        <LogPanel />
      </div>
    </div>
  );
}
