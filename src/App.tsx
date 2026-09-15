import * as React from "react";
import { useEffect, useState, lazy, Suspense } from "react";
import { useTranslation } from "react-i18next";
import { ClipboardCheck, Navigation, Network, Settings as SettingsIcon, Route, FolderTree, RadioTower, Server, Stethoscope } from "lucide-react";
import { useAppStore, type Locale, type Theme } from "./store/appStore";
const TopologyView = lazy(() => import("./views/TopologyView").then(m => ({ default: m.TopologyView })));
// one-level effective-config view, code-split like the topology canvas.
const EffectiveConfigView = lazy(() => import("./views/EffectiveConfigView").then(m => ({ default: m.EffectiveConfigView })));
import { SettingsView } from "./views/SettingsView";
import { ProcessRouteView } from "./views/ProcessRouteView";
import { SubscriptionsView } from "./views/SubscriptionsView";
import { PlatformsView } from "./views/PlatformsView";
import { NodesView } from "./views/NodesView";
import { DiagnosticsView } from "./views/DiagnosticsView";
import { useTheme } from "./lib/useTheme";
import { LogPanel } from "./components/LogPanel";
// the TopConvergeStatus pill in the
// header and the SideRailConvergeDot in the nav share one snapshot owned by
// App.tsx — see snapshot fetch useEffect below.
import { TopConvergeStatus } from "./components/TopConvergeStatus";
import { SideRailConvergeDot } from "./components/SideRailConvergeDot";
import { type ConvergePhase } from "./lib/ipc";
import { loadLocale, loadTheme, loadView } from "./lib/settings";
import { listen } from "@tauri-apps/api/event";


// Error Boundary — catches unexpected throws in render/useEffect (e.g. Tauri
// APIs called in a plain browser during headless mode). Without this, any uncaught
// error unmounts the entire React tree → white screen.
class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { hasError: boolean; error: Error | null }
> {
  constructor(props: { children: React.ReactNode }) {
    super(props);
    this.state = { hasError: false, error: null };
  }
  static getDerivedStateFromError(error: Error) {
    return { hasError: true, error };
  }
  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("[ErrorBoundary]", error, info?.componentStack);
  }
  render() {
    if (this.state.hasError) {
      return (
        <div className="h-full flex flex-col items-center justify-center bg-white dark:bg-zinc-950 text-zinc-900 dark:text-zinc-100 p-4">
          <p className="text-sm font-medium text-red-500 mb-2">Something went wrong</p>
          <pre className="text-xs text-zinc-500 max-w-md overflow-auto">{this.state.error?.message ?? "Unknown error"}</pre>
          <button
            onClick={() => { this.setState({ hasError: false, error: null }); window.location.reload(); }}
            className="mt-4 px-3 py-1.5 text-xs rounded-md bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900"
          >Reload</button>
        </div>
      );
    }
    return this.props.children;
  }
}

// Side rail nav: icon + label, desktop-tool density. Lucide vector icons
// (not emoji) per ui-ux-pro-max: scalable, theme-aware, consistent stroke.
const NAV_ITEMS = [
  { key: "topology", icon: Network },
  { key: "platforms", icon: FolderTree },
  { key: "processRoute", icon: Route },
  { key: "subscriptions", icon: RadioTower },
  { key: "nodes", icon: Server },
  // (spec Decision 1): effective config sits BEFORE diagnostics.
  { key: "effectiveConfig", icon: ClipboardCheck },
  { key: "diagnostics", icon: Stethoscope },
  { key: "settings", icon: SettingsIcon },
] as const;

function SideRail({ railConverge, railError }: { railConverge?: ConvergePhase; railError?: string }) {
  const { t } = useTranslation();
  const view = useAppStore((s) => s.view);
  const setView = useAppStore((s) => s.setView);

  return (
    <nav className="w-14 flex flex-col items-center gap-1 border-r border-zinc-200 dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-950 py-3 shrink-0">
      {/* (D-C2.1, checkpoint C): single 4px
          convergence dot at the top of the rail. Converged renders
          `display: none`; never opacity. Per-entry Health lives inside
          EffectiveConfigView (ArgoCD #22059 — Sync vs Health separate). */}
      <div className="w-9 h-1 flex items-center justify-center mb-1">
        {railConverge ? (
          <SideRailConvergeDot convergePhase={railConverge} lastApplyError={railError} />
        ) : null}
      </div>
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
  const setView = useAppStore((s) => s.setView);
  useTheme();
  // Bug #1 fix: the store defaults view="topology". Without a gate, the first
  // paint renders the topology (or its lazy fallback) before loadView() resolves
  // the persisted view and setView() swaps it — the user sees a topology flash.
  // Hold a minimal loader until the persisted view has been read.
  const [bootstrapped, setBootstrapped] = useState(false);
  // (checkpoint A): one snapshot,
  // passed down to TopConvergeStatus (header pill) and SideRailConvergeDot
// (rail dot). The children never reach for IPC. the
  // snapshot is owned by the appStore global converge subscription — the
  // poll cadence, sidecar-status boost, and visibility pause live there.
  const snap = useAppStore((s) => s.convergeSnapshot);
  const refreshConvergeSnapshot = useAppStore((s) => s.refreshConvergeSnapshot);
  const subscribeToConverge = useAppStore((s) => s.subscribeToConverge);

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
      const savedView = await loadView();
      if (savedView && !cancelled) {
        setView(savedView as any);
      }
// ADR-0055: process routes are rehydrated by
      // ProcessRouteView from the L2 whitebox via process_route_list IPC —
      // the L1 bootstrap read is gone.
      if (!cancelled) setBootstrapped(true);
    })();
    return () => { cancelled = true; };
  }, [setLocale, setTheme, setView, i18n]);

  // 07: refresh immediately on navigation (the existing
  // EffectiveConfigView also does its own fetch — every path converges on
  // the same IPC and the same authoritative snapshot); the global poll
  // cadence in the appStore subscription keeps it fresh afterwards.
  useEffect(() => {
    void refreshConvergeSnapshot();
  }, [view, refreshConvergeSnapshot]);

  // global converge subscription — 5s foreground polling
  // (30s once Converged settles >60s), immediate refresh on sidecar-status
  // events (the existing G4 retarget channel), paused while hidden.
  useEffect(() => subscribeToConverge(), [subscribeToConverge]);

  // tray "Converge status" menu item emits this event;
  // the listener navigates to EffectiveConfigView (the Rust side already
  // shows + focuses the window before emitting).
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen("tray://converge-status", () => {
      setView("effectiveConfig");
    })
      .then((fn) => { unlisten = fn; })
      .catch(() => { /* outside Tauri: no-op */ });
    return () => { if (unlisten) unlisten(); };
  }, [setView]);

  if (!bootstrapped) {
    return (
      <div className="h-full flex items-center justify-center bg-white dark:bg-zinc-950">
        <span className="text-sm text-zinc-400">Loading…</span>
      </div>
    );
  }
  return (
    <ErrorBoundary>
    <div className="h-full flex bg-white dark:bg-zinc-950 text-zinc-900 dark:text-zinc-100" onContextMenu={(e) => e.preventDefault()}>
      <SideRail
        railConverge={snap?.convergePhase}
        railError={snap?.lastApplyError}
      />
      <div className="flex-1 flex flex-col min-w-0">
        <header className="px-5 py-3 border-b border-zinc-200 dark:border-zinc-800 flex items-center gap-2">
          <Navigation size={16} className="text-zinc-400" strokeWidth={1.75} />
          <h1 className="text-sm font-semibold tracking-tight">{t("app.title")}</h1>
          <span className="text-xs text-zinc-400 dark:text-zinc-500 hidden sm:inline">· {t("app.tagline")}</span>
          {/* (D-C2.1): the global Sync pill.
              ArgoCD #22059 — Sync vs Health are kept SEPARATE: this pill is
              the top-level ConvergePhase only; per-entry drift badges live
              inside EffectiveConfigView and are NOT merged here. */}
          {snap ? (
            <span className="ml-auto">
              <TopConvergeStatus
                convergePhase={snap.convergePhase}
                generation={snap.strategyGeneration}
                appliedGeneration={snap.strategyAppliedGeneration}
                lastApplyAt={snap.lastApplyAt}
                lastApplyError={snap.lastApplyError}
                onOpenDetail={() => setView("effectiveConfig")}
              />
            </span>
          ) : null}
        </header>
        <main className="flex-1 overflow-auto">
          {view === "topology" && <Suspense fallback={<div className="flex-1 flex items-center justify-center text-sm text-zinc-400">Loading...</div>}><TopologyView /></Suspense>}
          {view === "platforms" && <PlatformsView />}
          {view === "settings" && <SettingsView />}
          {view === "processRoute" && <ProcessRouteView />}
          {view === "subscriptions" && <SubscriptionsView />}
          {view === "nodes" && <NodesView />}
          {view === "effectiveConfig" && <Suspense fallback={<div className="flex-1 flex items-center justify-center text-sm text-zinc-400">Loading...</div>}><EffectiveConfigView /></Suspense>}
          {view === "diagnostics" && <DiagnosticsView />}
        </main>
        <LogPanel />
      </div>
    </div>
    </ErrorBoundary>
  );
}
