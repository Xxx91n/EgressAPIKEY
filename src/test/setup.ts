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

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(args[0] as string, args[1] as Record<string, unknown> | undefined),
}));

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
        "subscription.importSuccess": "Subscription imported",
        "subscription.rename": "Rename subscription",
        "subscription.renamePrompt": "New name for \"{{name}}\"",
        "subscription.renameOk": "Renamed to \"{{name}}\"",
        "subscription.renameMissing": "Subscription not found on the gateway. Refresh the list and try again.",
        "subscription.renameUrlMissing": "Cannot rename: original URL not cached. Re-import the subscription with the new name instead.",
        "subscription.resetOrder": "Reset sort",
        "subscription.duplicate": "Subscription already exists. Choose a different name.",
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
        "strategy.title": "Egress Strategy", "strategy.apply": "Apply", "strategy.applyOk": "Strategy applied", "strategy.applyPartial": "Strategy applied with warnings", "strategy.noPlatforms": "No platforms configured", "strategy.aClass": "Node Selection", "strategy.bClass": "Egress Selection", "strategy.manual": "Manual", "strategy.region": "By Region", "strategy.quality": "Top-N Quality", "strategy.subscription": "By Subscription", "strategy.balanced": "Balanced", "strategy.preferLowLatency": "Prefer low latency", "strategy.preferIdleIp": "Prefer idle IP", "strategy.regionsHint": "US,SG,JP", "strategy.subsHint": "sub1,sub2", "strategy.topN": "Max nodes", "strategy.random": "Random", "strategy.sequential": "Sequential", "strategy.latency": "Prefer lowest latency", "strategy.bQuality": "IP quality score", "strategy.bandwidth": "Prefer highest bandwidth", "strategy.protocolWeight": "Protocol-weighted", "strategy.manualSelect": "Select nodes manually", "strategy.regionSelect": "Select by region", "strategy.subscriptionSelect": "Select by subscription", "strategy.qualityPreview": "Top-3 nodes", "strategy.aClassTitle": "Node Selection", "strategy.bClassTitle": "Egress Selection",
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