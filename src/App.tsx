import { useTranslation } from "react-i18next";
import { useAppStore } from "./store/appStore";
import { TopologyView } from "./views/TopologyView";
import { SettingsView } from "./views/SettingsView";
import { ProcessRouteView } from "./views/ProcessRouteView";
import { SubscriptionsView } from "./views/SubscriptionsView";

const NAV_ITEMS = [
  { key: "topology", icon: "\u{1F6F0}" },
  { key: "settings", icon: "\u2699" },
  { key: "processRoute", icon: "\u{1F9ED}" },
  { key: "subscriptions", icon: "\u{1F4E1}" },
] as const;

function NavBar() {
  const { t } = useTranslation();
  const view = useAppStore((s) => s.view);
  const setView = useAppStore((s) => s.setView);
  return (
    <nav className="flex gap-2 border-b border-zinc-200 dark:border-zinc-800 px-3 py-2 bg-white dark:bg-zinc-950">
      {NAV_ITEMS.map((item) => (
        <button
          key={item.key}
          onClick={() => setView(item.key)}
          aria-pressed={view === item.key}
          title={t(`nav.${item.key}`)}
          className={
            "px-3 py-1.5 rounded text-sm " +
            (view === item.key
              ? "bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900"
              : "text-zinc-700 hover:bg-zinc-100 dark:text-zinc-300 dark:hover:bg-zinc-800")
          }
        >
          <span aria-hidden className="mr-1">
            {item.icon}
          </span>
          {t(`nav.${item.key}`)}
        </button>
      ))}
    </nav>
  );
}

export default function App() {
  const { t } = useTranslation();
  const view = useAppStore((s) => s.view);
  return (
    <div className="h-full flex flex-col bg-zinc-50 dark:bg-zinc-900 text-zinc-900 dark:text-zinc-100">
      <header className="px-4 py-3 border-b border-zinc-200 dark:border-zinc-800">
        <h1 className="text-lg font-semibold">{t("app.title")}</h1>
        <p className="text-xs text-zinc-500 dark:text-zinc-400">{t("app.tagline")}</p>
      </header>
      <NavBar />
      <main className="flex-1 overflow-auto p-4">
        {view === "topology" && <TopologyView />}
        {view === "settings" && <SettingsView />}
        {view === "processRoute" && <ProcessRouteView />}
        {view === "subscriptions" && <SubscriptionsView />}
      </main>
    </div>
  );
}
