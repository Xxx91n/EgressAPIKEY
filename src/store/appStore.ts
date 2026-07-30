import { create } from "zustand";
import { saveView, saveProcessRoutes } from "../lib/settings";

/// One lane in the topology canvas. Matches resin-core lane state.
export interface LaneState {
  index: number;
  exitIp: string | null;
  busy: boolean;
  account: string | null;
  authority: string | null;
  // Issue 4+7: each lane carries the platform + key-hash it currently
  // serves, plus an SSE-locked flag. The Resin sidecar computes the
  // lane<->exit-IP mapping (token-hash->lane native); the desktop shell
  // only mirrors the live lease view here, so these default falsy and
  // only populate when the IPC gateway_snapshot active_leases includes
  // platform/token/exit_ip fields (Resin is the source of truth).
  platform: string | null;
  keyHash: string | null;
  sseLocked: boolean;
}

/// One Platform + its accounts (Resin Platform/Account model).
export interface Platform {
  name: string;
  accounts: Account[];
}

export interface Account {
  id: string;
  platform: string;
  lane: number;
  exitIp: string | null;
  active: boolean;
}

/// A per-process routing rule (process-route entry).
export interface ProcessRoute {
  id: string;
  process: string;
  targetLane: number;
}

/// A subscription import.
export interface Subscription {
  id: string;
  url: string;
  nodeCount: number;
  lanes: number;
}

/// View selection between the three primary desktop views.
/// Supported UI locales. MUST stay in lockstep with src/locales/ directories
/// and scripts/i18n-check.cjs ALL list.
export type Locale = "en" | "zh" | "ja" | "es" | "fr" | "de" | "ko" | "ru" | "pt" | "ar" | "it" | "nl" | "pl" | "tr" | "vi" | "th" | "id" | "hi";

/// Colour-scheme preference. "system" follows prefers-color-scheme at runtime.
export type Theme = "light" | "dark" | "system";

export type View = "topology" | "platforms" | "settings" | "processRoute" | "subscriptions";

export interface SubFormDraft {
  name: string;
  url: string;
}

export interface AppState {
  view: View;
  subFormDraft: SubFormDraft;
  lanes: LaneState[];
  platforms: Platform[];
  processRoutes: ProcessRoute[];
  subscriptions: Subscription[];
  laneCount: number;
  locale: Locale;
  theme: Theme;

  setView: (v: View) => void;
  setSubFormDraft: (d: SubFormDraft) => void;
  setLanes: (l: LaneState[]) => void;
  setPlatforms: (p: Platform[]) => void;
  addPlatform: (name: string) => void;
  removePlatform: (name: string) => void;
  addAccount: (platform: string, id: string, lane: number) => void;
  bindExitIp: (platform: string, account: string, ip: string) => void;
  addProcessRoute: (process: string, targetLane: number) => void;
  removeProcessRoute: (id: string) => void;
  setProcessRoutes: (routes: ProcessRoute[]) => void;
  addSubscription: (url: string, nodeCount: number, lanes: number) => void;
  setLaneCount: (n: number) => void;
  setLocale: (l: Locale) => void;
  setTheme: (t: Theme) => void;
}

const uid = () => Math.random().toString(36).slice(2, 10);

export const useAppStore = create<AppState>((set) => ({
  view: "topology",
  subFormDraft: { name: "", url: "" },
  lanes: [
    { index: 0, exitIp: null, busy: false, account: null, authority: null, platform: null, keyHash: null, sseLocked: false },
    { index: 1, exitIp: null, busy: false, account: null, authority: null, platform: null, keyHash: null, sseLocked: false },
  ],
  platforms: [],
  processRoutes: [],
  subscriptions: [],
  laneCount: 10,
  locale: "en",
  theme: "system",

  setView: (view) => { set({ view }); void saveView(view); },
  setSubFormDraft: (subFormDraft) => set({ subFormDraft }),
  setLanes: (lanes) => set({ lanes }),
  setPlatforms: (platforms) => set({ platforms }),
  addPlatform: (name) =>
    set((s) =>
      s.platforms.some((p) => p.name === name)
        ? s
        : { platforms: [...s.platforms, { name, accounts: [] }] }
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
  addProcessRoute: (process, targetLane) =>
    set((s) => ({
      processRoutes: [...s.processRoutes, { id: uid(), process, targetLane }],
    })),
  removeProcessRoute: (id) => {
    const next = useAppStore.getState().processRoutes.filter((r) => r.id !== id);
    set({ processRoutes: next });
    void saveProcessRoutes(next);
  },
  addSubscription: (url, nodeCount, lanes) =>
    set((s) => ({
      subscriptions: [...s.subscriptions, { id: uid(), url, nodeCount, lanes }],
    })),
  setProcessRoutes: (routes) => { set({ processRoutes: routes }); void saveProcessRoutes(routes); },
  setLaneCount: (n) => set({ laneCount: Math.max(1, Math.min(50, n)) }),
  setLocale: (locale) => set({ locale }),
  setTheme: (theme) => set({ theme }),
}));
