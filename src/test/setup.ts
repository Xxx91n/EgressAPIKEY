import { vi, beforeEach, beforeAll } from "vitest";
import "@testing-library/jest-dom/vitest";
import i18next from "i18next";
import { initReactI18next } from "react-i18next";

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
        "subscription.imported": "Imported {{count}} nodes into {{lanes}} lanes",
        "subscription.dragHint": "Drag rows to reorder",
        "subscription.importSuccess": "Subscription imported",
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
        "settings.general": "General",
        "settings.language": "Language",
        "settings.lanes": "Lanes",
        "settings.gatewayBind": "Gateway bind address",
        "settings.mihomoApi": "mihomo API URL",
        "settings.save": "Save",
        "settings.network": "Network",
        "settings.advanced": "Advanced",
        "settings.storage": "Storage",
        "settings.openConfigDir": "Open config directory",
        "settings.openLogDir": "Open log directory",
        "settings.unifiedHelp": "Save all changes",
        "common.delete": "Delete",
        "common.cancel": "Cancel",
        "common.confirm": "Confirm",
        "common.edit": "Edit",
        "theme.label": "Theme", "theme.light": "Light", "theme.dark": "Dark", "theme.system": "System",
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