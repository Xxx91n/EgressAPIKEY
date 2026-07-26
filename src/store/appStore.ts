import { create } from "zustand";

/// One lane in the topology canvas. Matches resin-core lane state.
export interface LaneState {
  index: number;
  exitIp: string | null;
  busy: boolean;
  account: string | null;
  authority: string | null;
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
export type View = "topology" | "settings" | "processRoute" | "subscriptions";

export interface AppState {
  view: View;
  lanes: LaneState[];
  platforms: Platform[];
  processRoutes: ProcessRoute[];
  subscriptions: Subscription[];
  laneCount: number;
  locale: "en" | "zh";

  setView: (v: View) => void;
  setLanes: (l: LaneState[]) => void;
  setPlatforms: (p: Platform[]) => void;
  addPlatform: (name: string) => void;
  removePlatform: (name: string) => void;
  addAccount: (platform: string, id: string, lane: number) => void;
  bindExitIp: (platform: string, account: string, ip: string) => void;
  addProcessRoute: (process: string, targetLane: number) => void;
  removeProcessRoute: (id: string) => void;
  addSubscription: (url: string, nodeCount: number, lanes: number) => void;
  setLaneCount: (n: number) => void;
  setLocale: (l: "en" | "zh") => void;
}

const uid = () => Math.random().toString(36).slice(2, 10);

export const useAppStore = create<AppState>((set) => ({
  view: "topology",
  lanes: [
    { index: 0, exitIp: null, busy: false, account: null, authority: null },
    { index: 1, exitIp: null, busy: false, account: null, authority: null },
  ],
  platforms: [],
  processRoutes: [],
  subscriptions: [],
  laneCount: 10,
  locale: "en",

  setView: (view) => set({ view }),
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
  removeProcessRoute: (id) =>
    set((s) => ({ processRoutes: s.processRoutes.filter((r) => r.id !== id) })),
  addSubscription: (url, nodeCount, lanes) =>
    set((s) => ({
      subscriptions: [...s.subscriptions, { id: uid(), url, nodeCount, lanes }],
    })),
  setLaneCount: (n) => set({ laneCount: Math.max(1, Math.min(50, n)) }),
  setLocale: (locale) => set({ locale }),
}));
