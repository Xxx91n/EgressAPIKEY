import { vi, beforeEach, beforeAll } from "vitest";
import "@testing-library/jest-dom/vitest";
import i18next from "i18next";
import { initReactI18next } from "react-i18next";
// ReactFlow needs ResizeObserver; jsdom does not provide it.
class ResizeObserverPolyfill {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as unknown as { ResizeObserver: typeof ResizeObserverPolyfill }).ResizeObserver = ResizeObserverPolyfill;


/// Shared vitest setup (loaded via vitest.config.ts setupFiles).
/// Mocks the Tauri runtime so any view that uses ipc.ts runs in jsdom.
/// Default invoke resolves undefined so optimistic store paths execute;
/// specs override with invokeMock.mockResolvedValueOnce(...) per call.

const invokeMock = vi.fn<(cmd: string, args?: Record<string, unknown>) => Promise<unknown>>();

// T18: stub Tauri Channel<T> — Channel class defined INSIDE the vi.mock factory
// so it survives vitest vi.mock hoisting.
vi.mock("@tauri-apps/api/core", () => {
  class ChannelStub<T = unknown> {
    onmessage: ((msg: T) => void) | null = null;
    constructor() { (globalThis as unknown as { __lastChannel: ChannelStub<T> }).__lastChannel = this; }
    __emit(msg: T) { if (this.onmessage) this.onmessage(msg); }
    __close() { this.onmessage = null; }
  }
  return {
    invoke: (...args: unknown[]) => invokeMock(args[0] as string, args[1] as Record<string, unknown> | undefined),
    Channel: ChannelStub,
  };
});

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openPath: vi.fn(async () => {}),
}));

vi.mock("@tauri-apps/plugin-store", () => ({
  LazyStore: vi.fn().mockImplementation(() => ({
    get: vi.fn(async () => null),
    set: vi.fn(async () => {}),
    save: vi.fn(async () => {}),
  })),
}));

beforeAll(async () => {
  // T17 dual-mode: stub Tauri internals so isTauri() returns true in jsdom
  // T17 dual-mode: stub Tauri internals so isTauri() returns true in jsdom.
  // vitest jsdom gives each test file its own Window; globalThis and window
  // are the same object, but we set both defensively in case a future
  // environment isolates them. Also set window.isTauri so the second probe
  // (c0a309f) succeeds even if __TAURI_INTERNALS__ is somehow undefined.
  (globalThis as unknown as { __TAURI_INTERNALS?: unknown }).__TAURI_INTERNALS = {};
  if (typeof window !== "undefined") {
    (window as unknown as { __TAURI_INTERNALS?: unknown }).__TAURI_INTERNALS = {};
    (window as unknown as { isTauri?: unknown }).isTauri = true;
  }
  await i18next.use(initReactI18next).init({
    resources: {
      en: { translation: {
        "platform.title": "Platforms",
        "platform.add": "Add platform",
        "platform.empty": "No platforms configured.",
        "platform.healthOk": "port alive (SOCKS5)",
        "platform.healthProtocolMismatch": "listener on different protocol",
        "platform.healthUnavailable": "port unreachable",
        "platform.copyCredentials": "Copy SOCKS5 credentials",
        "platform.copied": "copied",
        "platform.socks5Auth": "SOCKS5 user",
        "platform.passwordMasked": "password: (masked)",
        "platform.httpNoAuth": "HTTP proxy (no authentication)",
        "platform.httpAuth": "HTTP Auth",
        "platform.unbound": "Unbound",
        "platform.noAuthRequired": "No authentication required",
        "platform.entryPorts": "Entry ports",
        "platform.activated": "Activated",
        "platform.leases": "Active leases",
        "platform.routableNodes": "Routable nodes",
        "platform.portInvalid": "Port must be 1024-65535",
        "platform.portDuplicate": "Port already exists",
        "platform.portAddOk": "Entry port added",
        "platform.portRemoved": "Entry port removed",
        "platform.portEnabled": "Port enabled",
        "platform.portDisabled": "Port disabled",
        "platform.enablePort": "Enable port",
        "platform.disablePort": "Disable port",
        "platform.portBound": "Bound :{{port}} to {{platform}}",
        "platform.noPorts": "No entry ports yet",
        "platform.port": "Port",
        "platform.portLabel": "Label",
        "platform.requireAuth": "Require authentication",
        "platform.addPort": "Add entry port",
        "platform.addOk": "Platform added",
        "platform.createEmptyName": "Platform name cannot be empty",
        "platform.createSubmit": "Create",
        "platform.createTitle": "New platform",
        "platform.deleteConfirm": "Delete this platform?",
        "platform.splitHint": "Left: entry ports. Right: platforms.",
        "platform.name": "Name",
        "account.id": "Account id",
        "account.lane": "Lane",
        "account.exitIp": "Exit IP",
        "account.bindIp": "Bind exit IP",
        "account.add": "Add account",
        "subscription.title": "Subscriptions",
        "subscription.import": "Import subscription",
        "subscription.url": "Subscription URL",
        "subscription.name": "Subscription name",
        "subscription.description": "Optional label for this imported subscription",
        "subscription.imported": "Imported {{count}} nodes",
        "subscription.dragHint": "Drag rows to reorder",
        "subscription.importSuccess": "Imported {{total}} nodes — check the subscription list for any fetch errors",
        "subscription.rename": "Rename subscription",
        "subscription.renamePrompt": "New name for \"{{name}}\"",
        "subscription.renameOk": "Renamed to \"{{name}}\"",
        "subscription.renameMissing": "Subscription not found on the gateway. Refresh the list and try again.",
        "subscription.renameUrlMissing": "Cannot rename: original URL not cached. Re-import the subscription with the new name instead.",
        "subscription.resetOrder": "Reset sort",
        "subscription.duplicate": "Subscription already exists. Choose a different name.",
        "subscription.importSuccessWithHint": "Imported {{total}} nodes — a fetch error was detected, see the red banner below",
        "subscription.importFetchFailed": "Subscription fetch failed: {{name}} — {{error}}",
        "subscription.fetchFailed": "Subscription {{name}} fetch failed: {{error}}",
        "subscription.fetchRecovered": "Subscription {{name}} fetch recovered",
        "subscription.fetchErrorBanner": "Fetch error: {{error}}",
        "subscription.fetchErrorChip": "fetch error",
        "subscription.showError": "Show error",
        "subscription.hideError": "Hide error",
        "subscription.lastSuccess": "last check succeeded",
        // Round 5 T01 (F1/F4): bind step + reverse-lookup badge vocabulary.
        "subscription.bindStepTitle": "Bind \"{{name}}\" to platforms (optional, {{count}} nodes imported)",
        "subscription.bindHint": "Only platforms with a_class=Subscription consume this subscription.",
        "subscription.bindCreate": "Auto-create platform \"{{name}}\"",
        "subscription.bindCreateOff": "Click to drop the auto-created platform",
        "subscription.bindPortsHint": "Pick entry ports to bind (optional)",
        "subscription.bindConfirm": "Bind & apply",
        "subscription.bindSkip": "Skip for now",
        "subscription.bindNow": "Bind to a platform",
        "subscription.bindApplyFailed": "Bound \"{{platform}}\" in the whitebox, but strategy_apply failed",
        "subscription.importBound": "Imported and connected platform {{platform}} ({{ports}} ports bound), {{count}} nodes",
        "subscription.boundTo": "Bound: {{count}} platform(s)",
        "subscription.unbound": "Unbound",
        "subscription.danglingRef": "Dangling reference",
                "nodes.title": "Node Pool",
        "nodes.refresh": "Refresh",
        "nodes.total": "Total nodes",
        "nodes.healthy": "Healthy",
        "nodes.unhealthy": "unhealthy",
        "nodes.egressIps": "Egress IPs",
        "nodes.healthyEgress": "Healthy egress",
        "nodes.loading": "Loading nodes...",
        "nodes.empty": "No nodes loaded. Import a subscription first.",
        "nodes.egressPolicyNote": "Per-region egress policy is bound on the Topology canvas.",
        "nodes.protocolWeights": "Protocol weights for AI SSE",
        "nodes.protocolWeightDesc": "Best (1.0): http, socks5, vmess/vless-tcp, trojan-tls-tcp. Moderate (0.7): shadowsocks. Avoid (0.1): hysteria2, tuic, wireguard.",
        "nodes.reputationTitle": "IP reputation",
        "nodes.reputationSummary": "{{count}} live egress IPs checked",
        "nodes.reputationCached": "cached",
        "nodes.reputationNotConfigured": "Choose a provider and save its API key in Settings.",
        "nodes.reputationDisabled": "IP reputation checks are disabled.",
        "nodes.search": "Search nodes...",
        "nodes.expandAll": "Expand all",
        "nodes.collapseAll": "Collapse all",
        "nodes.nodeCount": "{{count}} nodes",
        "nodes.healthRate": "{{rate}}% healthy",
        "nodes.noMatch": "No nodes match search.",
        "nodes.untagged": "Ungrouped",
        "nodes.timeout": "timeout",
        "nodes.hideUnhealthy": "Hide unhealthy",
        "nodes.sortDefault": "Sort: default",
        "nodes.sortLatencyAsc": "Sort: latency ↑",
        "nodes.sortLatencyDesc": "Sort: latency ↓",
        "nodes.delayFilterHint": "Use delay>100 or delay=timeout",
        "nodes.refreshSub": "Refresh subscription",
        "nodes.refreshingSub": "Fetching…",
        "nodes.probeLatency": "Probe latency",
        "nodes.probeEgress": "Probe egress",
        "nodes.batchProbe": "Batch probe latency",
        "nodes.batchProbing": "Probing {{done}}/{{total}}…",
        "nodes.syncCache": "Re-sync backend node snapshot",
        "nodes.refreshSent": "Refresh request sent to backend",
        "nodes.refreshDone": "Refreshed, {{count}} nodes in pool",
        "nodes.refreshNoChange": "No node count change; retry might be needed",
        "nodes.lastRefresh": "Last refresh: {{time}}",
        "processRoute.title": "Process route",
        "processRoute.add": "Add routing rule",
        "processRoute.process": "Process name",
        "processRoute.target": "Target lane",
        "processRoute.empty": "No per-process routing rules.",
        "processRoute.conflict": "Lane {{lane}} already bound to process \"{{process}}\"",
        "processRoute.backendSaved": "Routing rule saved",
        "topology.live": "Lanes refresh live from the gateway",
        "topology.sidecarUnhealthy": "Sidecar unsafe: Resin proxy offline.",
        "topology.entry": "Entry: {{platform}}",
        "topology.leasesActive": "Active leases: {{count}}",
        "topology.lane": "Lane {{index}}",
        "topology.entryPort": "Entry proxy port",
        "topology.platformsCol": "Platforms",
        "topology.nodesCol": "IP channels",
        "topology.dragHint": "Drag from a platform to a node region to bind routing",
        "topology.region": "Region: {{region}}",
        "topology.healthy": "healthy",
        "topology.unhealthy": "unhealthy",
        "topology.noNodes": "No nodes loaded. Import a subscription first.",
        "topology.noPlatforms": "No platforms. Add one in the Platforms tab.",
        "topology.policy": "Policy: {{policy}}",
        "topology.routable": "Routable: {{count}}",
        "topology.filters": "Upstream: {{filters}}",
        "topology.aClassRegion": "Region: {{regions}}",
        "topology.resetCenter": "Reset to center",
        "topology.viewSubscription": "Subscriptions",
        "topology.viewRegion": "Regions",
        "topology.expandNodes": "Expand",
        "topology.collapseNodes": "Collapse",
        "topology.openConfig": "Open config",
        "topology.openPortsConfig": "Open ports config",
        "topology.openStrategyConfig": "Open strategy config",
        "topology.minimapHint": "Mini-map: pan the canvas overview. Node colors match node types.",
        "topology.portAlive": "Alive",
        "topology.portDegraded": "Degraded",
        "topology.portDead": "Dead",
        "topology.portRestarting": "Restarting",
        "topology.portAuthRequired": "Authentication required",
        "topology.portAuthNotRequired": "No authentication",
        "topology.portDisabled": "Disabled",
        "topology.zoomIn": "Zoom in",
        "topology.zoomOut": "Zoom out",
        "topology.fitView": "Fit view",
        "topology.lock": "Lock canvas",
        "topology.unlock": "Unlock canvas",
        "topology.aClassManualCount": "Manual ({{n}} nodes)", "topology.aClassManual": "Manual selection",
        "topology.aClassQuality": "Quality Top-{{n}}",
        "topology.aClassSubscription": "Subscription: {{subs}}",
        "settings.strategyConfig": "Strategy config",
        "settings.strategyConfigPath": "Path",
        "settings.strategyConfigReload": "Reload from disk",
        "settings.strategyReloaded": "Applied {{patched}} platforms, {{errors}} errors",
        "settings.nodeProbeTitle": "Node probe parameters",
        "settings.nodeProbeConcurrency": "Concurrency (1-50)",
        "settings.nodeProbeTimeout": "Timeout (ms)",
        "settings.nodeProbeBatchOnLoad": "Auto-probe on page load",
        "settings.resinProbeTitle": "Resin probe parameters",
        "settings.resinMaxFailures": "Max consecutive failures",
        "settings.resinLatencyInterval": "Max latency test interval",
        "settings.resinLatencyTestUrl": "Latency test URL",
        "settings.resinMaxEgressInterval": "Max egress test interval",
        "settings.resinLatencyAuthorities": "Latency authorities",
        "settings.resinP2cLatencyWindow": "P2C latency window",
        "settings.resinLatencyDecayWindow": "Latency decay window",
        "settings.general": "General",
        "settings.language": "Language",
                                "settings.save": "Save",
        "settings.network": "Network",
                "settings.storage": "Storage",
        "settings.openConfigDir": "Open config directory",
        "settings.openLogDir": "Open log directory",
        "settings.unifiedHelp": "Save all changes",
        "common.delete": "Delete",
        "common.cancel": "Cancel",
        "common.confirm": "Confirm",
        "common.edit": "Edit",
        "theme.label": "Theme", "theme.light": "Light", "theme.dark": "Dark", "theme.system": "System",
        "strategy.title": "Egress Strategy", "strategy.apply": "Apply", "strategy.applyOk": "Strategy applied", "strategy.applyPartial": "Strategy applied with warnings", "strategy.noPlatforms": "No platforms configured", "strategy.aClass": "Node Selection", "strategy.bClass": "Egress Selection", "strategy.manual": "Manual", "strategy.region": "By Region", "strategy.quality": "Top-N Quality", "strategy.subscription": "By Subscription", "strategy.balanced": "Balanced", "strategy.preferLowLatency": "Prefer low latency", "strategy.preferIdleIp": "Prefer idle IP", "strategy.regionsHint": "US,SG,JP", "strategy.subsHint": "sub1,sub2", "strategy.topN": "Max nodes", "strategy.random": "Random", "strategy.sequential": "Sequential", "strategy.latency": "Prefer lowest latency", "strategy.bQuality": "IP quality score", "strategy.bandwidth": "Prefer highest bandwidth", "strategy.protocolWeight": "Protocol-weighted", "strategy.manualSelect": "Select nodes manually", "strategy.regionSelect": "Select by region", "strategy.subscriptionSelect": "Select by subscription", "strategy.qualityPreview": "Top-3 nodes", "strategy.aClassTitle": "Node Selection", "strategy.bClassTitle": "Egress Selection", "strategy.bParamsRandom": "Random", "strategy.bParamsSequential": "Sequential (N={{n}})", "strategy.bParamsLatency": "Latency (<{{threshold}}ms)", "strategy.bParamsQuality": "Quality (≥{{score}})", "strategy.bParamsBandwidth": "Bandwidth (×{{weight}})", "strategy.bParamsProtocolWeight": "Protocol Weight",
        "app.title": "AI API Route",
        "nav.topology": "Topology", "nav.platforms": "Platforms", "nav.settings": "Settings",
        "nav.processRoute": "Process Route", "nav.subscriptions": "Subscriptions",
      } },
    },
    lng: "en",
    fallbackLng: "en",
    interpolation: { escapeValue: false },
  });
});

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

export { invokeMock };