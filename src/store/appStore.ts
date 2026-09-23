import { create } from "zustand";
import { useShallow } from "zustand/react/shallow";
import { saveView } from "../lib/settings";
// type-only import — erased at build, no runtime cycle.
import type { SubscriptionPhaseRow } from "../lib/ipc";
import { listen } from "@tauri-apps/api/event";
import { ipcAuthoritativeSnapshot, type AuthoritativeSnapshot } from "../lib/ipc";

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
  /** per-subscription establish-phase STATUS rows,
   *  mirrored from the authoritative snapshot stream (App-level pull +
   *  SubscriptionsView refresh feed the same field; data identical). */
  subscriptionPhases: SubscriptionPhaseRow[];
  locale: Locale;
  theme: Theme;
  /// the globally-subscribed authoritative snapshot.
  /// Display truth only (ADR-0051); null until the first fetch lands and on
  /// fetch failure (the "no pill" contract, not a fake Unknown).
  convergeSnapshot: AuthoritativeSnapshot | null;
  /// Whether the global converge subscription is wired (subscribeToConverge).
  convergeSubscribed: boolean;
  /// Checkpoint D: polling is paused while the document is hidden.
  convergePausedByVisibility: boolean;

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
  setSubscriptionPhases: (rows: SubscriptionPhaseRow[]) => void;
  setLocale: (l: Locale) => void;
  setTheme: (t: Theme) => void;
  refreshConvergeSnapshot: () => Promise<void>;
  subscribeToConverge: () => () => void;
}

const uid = () => Math.random().toString(36).slice(2, 10);

export const useAppStore = create<AppState>((set) => ({
  view: "topology",
  subFormDraft: { name: "", url: "" },
  platforms: [],
  nodes: [],
  processRoutes: [],
  subscriptions: [],
  subscriptionPhases: [],
  locale: "en",
  theme: "system",
  convergeSnapshot: null,
  convergeSubscribed: false,
  convergePausedByVisibility: false,

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
// ADR-0055: view cache only — persistence goes through the
  // process_route_* IPC commands (single write entry, L2 whitebox).
  removeProcessRoute: (id) => {
    set({ processRoutes: useAppStore.getState().processRoutes.filter((r) => r.id !== id) });
  },
  addSubscription: (url, nodeCount) =>
    set((s) => ({
      subscriptions: [...s.subscriptions, { id: uid(), url, nodeCount }],
    })),
  setProcessRoutes: (routes) => set({ processRoutes: routes }),
  setSubscriptionPhases: (subscriptionPhases) => set({ subscriptionPhases }),
  setLocale: (locale) => set({ locale }),
  setTheme: (theme) => set({ theme }),
  refreshConvergeSnapshot: () => convergeRefresh(),
  subscribeToConverge: () => convergeSubscribe(),
}));

/// re-export useShallow for ergonomic multi-field selection
/// Usage: const { field1, field2 } = useAppStore(useShallow((s) => ({ field1: s.field1, field2: s.field2 })))
export { useShallow };

// ─── the global converge loop ──────────────────────
// One subscription owns the authoritative-snapshot cadence for the whole app:
//   - checkpoint A: 5s foreground polling (default); once the phase is
//     Converged AND lastApplyAt is more than CONVERGE_SETTLE_SECONDS (60s)
//     old, the cadence backs off to 30s — a settled green world does not
//     need a 5s heartbeat, but a fresh green apply is still watched closely;
//   - checkpoint B: an immediate refresh whenever the sidecar-status event
//     fires (the existing Resin G4 IPC retarget channel from sidecar.rs,
//     reused verbatim — no new channel; the string payload is untrusted and
//     unused, the event is a trigger only, AGENTS §7.6 discipline);
//   - checkpoint D: polling pauses while the document is hidden (visibility
//     API) and a resume refetches immediately before re-arming.
// The snapshot is display truth (ADR-0051): nothing here writes config, and
// every write still flows through the ADR-0036 / ADR-0042 entries.

/// Foreground polling cadence (checkpoint A: default 5s).
export const CONVERGE_POLL_INTERVAL_MS = 5_000;
/// Backoff cadence while Converged is settled (checkpoint A: 30s).
export const CONVERGE_POLL_CONVERGED_MS = 30_000;
/// A Converged phase only earns the slow cadence once the last green apply
/// is at least this old (checkpoint A: lastApplyAt > 60s).
export const CONVERGE_SETTLE_SECONDS = 60;

/// Event channel owned by src-tauri/src/sidecar.rs (G3 health poll / G4).
export const SIDECAR_STATUS_EVENT = "sidecar-status";
/// ADR-0069 D3: emitted by the Rust port family when an L2 intent was
/// persisted but the L3 (Resin) mutation was rejected, so the drift becomes
/// visible immediately instead of at the next poll. Payload-less trigger only.
export const SNAPSHOT_REFRESH_EVENT = "snapshot://refresh";

/// Pure cadence selector (checkpoint A). Injected nowSec keeps it testable
/// without fake timers. Converged + settled apply → 30s; everything else
/// (including a fresh Converged apply within the settle window) stays 5s.
export function convergePollIntervalMs(
  snap: AuthoritativeSnapshot | null,
  nowSec: number
): number {
  if (
    snap !== null &&
    snap.convergePhase === "Converged" &&
    typeof snap.lastApplyAt === "number" &&
    nowSec - snap.lastApplyAt > CONVERGE_SETTLE_SECONDS
  ) {
    return CONVERGE_POLL_CONVERGED_MS;
  }
  return CONVERGE_POLL_INTERVAL_MS;
}

// Module-scope controller state: timers and event unlisteners are process
// resources, not render state — the observable parts are store fields.
let convergeTimer: ReturnType<typeof setTimeout> | null = null;
let convergeUnlistenSidecar: (() => void) | null = null;
let convergeUnlistenSnapshotRefresh: (() => void) | null = null;
let convergeOnVisibility: (() => void) | null = null;
let convergeInFlight = false;
/// r12-wave-d D4c probe for the React-Query register line (arm (a)): a
/// monotonic dev-only counter that records a snapshot-trigger mark landing
/// while the previous ipcAuthoritativeSnapshot invoke is still unresolved.
/// One measured overlap fires the line; two observation cycles at zero
/// return it to suspended. import.meta.env.DEV dead-code-eliminates the
/// whole instrument in production builds.
const convergeDevMarks = import.meta.env.DEV
  ? { lastMarkAt: 0, overlaps: 0 }
  : null;
function convergeDevMark(path: string): void {
  if (convergeDevMarks === null) return;
  const now = performance.now();
  if (convergeInFlight) {
    convergeDevMarks.overlaps += 1;
    console.debug(
      `[converge] ${path} snapshot trigger overlapped an in-flight pull (overlaps=${convergeDevMarks.overlaps}, mark delta ${(now - convergeDevMarks.lastMarkAt).toFixed(0)}ms)`,
    );
  }
  convergeDevMarks.lastMarkAt = now;
}
/// Rises on every subscribe/unsubscribe so a late-resolving listen() from a
/// discarded subscription cannot leak its unlisten into the next one
/// (React StrictMode double-mounts effects in dev).
let convergeGeneration = 0;

function convergeClearTimer(): void {
  if (convergeTimer !== null) {
    clearTimeout(convergeTimer);
    convergeTimer = null;
  }
}

function convergeSchedule(): void {
  if (convergeTimer !== null) return;
  // Only the global subscription arms the cadence: one-shot refreshes (the
  // App navigation path, tests) must not start polling on their own.
  if (!useAppStore.getState().convergeSubscribed) return;
  if (useAppStore.getState().convergePausedByVisibility) return;
  const delay = convergePollIntervalMs(
    useAppStore.getState().convergeSnapshot,
    Date.now() / 1000
  );
  convergeTimer = setTimeout(() => {
    convergeTimer = null;
    void useAppStore.getState().refreshConvergeSnapshot();
  }, delay);
}

async function convergeRefresh(): Promise<void> {
  if (convergeInFlight) return;
  convergeInFlight = true;
  try {
    const snap = await ipcAuthoritativeSnapshot();
    useAppStore.setState({ convergeSnapshot: snap });
  } catch {
    // contract: a fetch failure degrades to no pill (null), never
    // to a misleading Unknown pill. The loop keeps running; the next poll
    // (or sidecar-status boost) may catch the sidecar coming back.
    useAppStore.setState({ convergeSnapshot: null });
  } finally {
    convergeInFlight = false;
  }
  convergeSchedule();
}

function convergeSubscribe(): () => void {
  if (useAppStore.getState().convergeSubscribed) return convergeUnsubscribe;
  const gen = ++convergeGeneration;
  useAppStore.setState({
    convergeSubscribed: true,
    convergePausedByVisibility:
      typeof document === "undefined" ? false : document.visibilityState === "hidden",
  });
  // Checkpoint B: sidecar lifecycle transitions (healthy/unhealthy/
  // terminated/restarting from the G3 health poll) refetch immediately.
  void listen(SIDECAR_STATUS_EVENT, () => {
    convergeDevMark("sidecar-status");
    void useAppStore.getState().refreshConvergeSnapshot();
  })
    .then((unlisten) => {
      if (gen === convergeGeneration) convergeUnlistenSidecar = unlisten;
      else unlisten();
    })
    .catch(() => { /* outside Tauri: polling still runs */ });
  // ADR-0069 D3: a port command that persisted the L2 intent but was rejected
  // by Resin asks for an immediate re-pull, so the resulting drift shows at
  // once instead of at the next 5s/30s poll. Same trigger-only discipline as
  // checkpoint B (the payload is untrusted and unused).
  void listen(SNAPSHOT_REFRESH_EVENT, () => {
    convergeDevMark("snapshot-refresh");
    void useAppStore.getState().refreshConvergeSnapshot();
  })
    .then((unlisten) => {
      if (gen === convergeGeneration) convergeUnlistenSnapshotRefresh = unlisten;
      else unlisten();
    })
    .catch(() => { /* outside Tauri: polling still runs */ });
  // Checkpoint D: hidden clears the timer; a resume refetches once and lets
  // the refresh re-arm the cadence from the (possibly changed) phase.
  convergeOnVisibility = () => {
    const hidden =
      typeof document === "undefined" ? false : document.visibilityState === "hidden";
    if (useAppStore.getState().convergePausedByVisibility === hidden) return;
    useAppStore.setState({ convergePausedByVisibility: hidden });
    if (hidden) {
      convergeClearTimer();
    } else {
      void useAppStore.getState().refreshConvergeSnapshot();
    }
  };
  document.addEventListener("visibilitychange", convergeOnVisibility);
  void useAppStore.getState().refreshConvergeSnapshot();
  return convergeUnsubscribe;
}

function convergeUnsubscribe(): void {
  convergeGeneration++;
  convergeClearTimer();
  if (convergeOnVisibility !== null) {
    document.removeEventListener("visibilitychange", convergeOnVisibility);
    convergeOnVisibility = null;
  }
  if (convergeUnlistenSidecar !== null) {
    try {
      convergeUnlistenSidecar();
    } catch {
      /* ignore double-unlisten */
    }
    convergeUnlistenSidecar = null;
  }
  if (convergeUnlistenSnapshotRefresh !== null) {
    try {
      convergeUnlistenSnapshotRefresh();
    } catch {
      /* ignore double-unlisten */
    }
    convergeUnlistenSnapshotRefresh = null;
  }
  useAppStore.setState({ convergeSubscribed: false, convergePausedByVisibility: false });
}
