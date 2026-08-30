import { create } from "zustand";
import { useShallow } from "zustand/react/shallow";
import { saveView, saveProcessRoutes } from "../lib/settings";

/// One Platform + its accounts (Resin Platform/Account model).
export interface Platform {
  name: string;
  accounts: Account[];
  // Phase R2: full Resin platform schema for the topology canvas.
  regexFilters: string[] | null;
  regionFilters: string[] | null;
  allocationPolicy: string;
  routableNodeCount: number;
  stickyTtl: string;
}

export interface Account {
  id: string;
  platform: string;
  lane: number;
  exitIp: string | null;
  active: boolean;
}

/// Phase R2: a proxy node from Resin GET /api/v1/nodes (the "C category").
export interface NodeInfo {
  nodeHash: string;
  displayTag: string;
  enabled: boolean;
  hasOutbound: boolean;
  failureCount: number;
  region: string | null;
  tags: { subscriptionName: string; tag: string }[];
}

/// A per-process routing rule (process-route entry).
export interface ProcessRoute {
  id: string;
  process: string;
  targetPort: number;
}

/// A subscription import.
export interface Subscription {
  id: string;
  url: string;
  nodeCount: number;
}

/// View selection between the three primary desktop views.
/// Supported UI locales. MUST stay in lockstep with src/locales/ directories
/// and scripts/i18n-check.cjs ALL list.
export type Locale = "en" | "zh" | "ja" | "es" | "fr" | "de" | "ko" | "ru" | "pt" | "ar" | "it" | "nl" | "pl" | "tr" | "vi" | "th" | "id" | "hi";

/// Colour-scheme preference. "system" follows prefers-color-scheme at runtime.
export type Theme = "light" | "dark" | "system";

export type View = "topology" | "platforms" | "nodes" | "settings" | "processRoute" | "subscriptions" | "effectiveConfig" | "diagnostics";

export interface SubFormDraft {
  name: string;
  url: string;
}

export interface AppState {
  view: View;
  subFormDraft: SubFormDraft;
  platforms: Platform[];
  nodes: NodeInfo[];
  processRoutes: ProcessRoute[];
  subscriptions: Subscription[];
  locale: Locale;
  theme: Theme;

  setView: (v: View) => void;
  setSubFormDraft: (d: SubFormDraft) => void;
  setPlatforms: (p: Platform[]) => void;
  setNodes: (n: NodeInfo[]) => void;
  addPlatform: (name: string) => void;
  removePlatform: (name: string) => void;
  addAccount: (platform: string, id: string, lane: number) => void;
  bindExitIp: (platform: string, account: string, ip: string) => void;
  addProcessRoute: (process: string, targetPort: number) => void;
  removeProcessRoute: (id: string) => void;
  setProcessRoutes: (routes: ProcessRoute[]) => void;
  addSubscription: (url: string, nodeCount: number) => void;
  setLocale: (l: Locale) => void;
  setTheme: (t: Theme) => void;
}

const uid = () => Math.random().toString(36).slice(2, 10);

export const useAppStore = create<AppState>((set) => ({
  view: "topology",
  subFormDraft: { name: "", url: "" },
  platforms: [],
  nodes: [],
  processRoutes: [],
  subscriptions: [],
  locale: "en",
  theme: "system",

  setView: (view) => { set({ view }); void saveView(view); },
  setSubFormDraft: (subFormDraft) => set({ subFormDraft }),
  setPlatforms: (platforms) => set({ platforms }),
  setNodes: (nodes) => set({ nodes }),
  addPlatform: (name) =>
    set((s) =>
      s.platforms.some((p) => p.name === name)
        ? s
        : { platforms: [...s.platforms, { name, accounts: [], regexFilters: null, regionFilters: null, allocationPolicy: "BALANCED", routableNodeCount: 0, stickyTtl: "168h0m0s" }] }
    ),
  removePlatform: (name) =>
    set((s) => ({ platforms: s.platforms.filter((p) => p.name !== name) })),
  addAccount: (platform, id, lane) =>
    set((s) => ({
      platforms: s.platforms.map((p) =>
        p.name === platform
          ? {
              ...p,
              accounts: [...p.accounts, { id, platform, lane, exitIp: null, active: true }],
            }
          : p
      ),
    })),
  bindExitIp: (platform, account, ip) =>
    set((s) => ({
      platforms: s.platforms.map((p) =>
        p.name === platform
          ? {
              ...p,
              accounts: p.accounts.map((a) =>
                a.id === account ? { ...a, exitIp: ip } : a
              ),
            }
          : p
      ),
    })),
  addProcessRoute: (process, targetPort) =>
    set((s) => ({
      processRoutes: [...s.processRoutes, { id: uid(), process, targetPort }],
    })),
  removeProcessRoute: (id) => {
    const next = useAppStore.getState().processRoutes.filter((r) => r.id !== id);
    set({ processRoutes: next });
    void saveProcessRoutes(next);
  },
  addSubscription: (url, nodeCount) =>
    set((s) => ({
      subscriptions: [...s.subscriptions, { id: uid(), url, nodeCount }],
    })),
  setProcessRoutes: (routes) => { set({ processRoutes: routes }); void saveProcessRoutes(routes); },
  setLocale: (locale) => set({ locale }),
  setTheme: (theme) => set({ theme }),
}));

/// T14-4: re-export useShallow for ergonomic multi-field selection
/// Usage: const { field1, field2 } = useAppStore(useShallow((s) => ({ field1: s.field1, field2: s.field2 })))
export { useShallow };
