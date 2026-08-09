import { describe, it, expect, vi, beforeEach } from "vitest";

/// Issue 1: closed-loop coverage for the IPC boundary. We mock
/// @tauri-apps/api/core's invoke and verify:
///   - IPC wrappers validate inputs (port range, name length, URL shape)
///   - Forwards the args to the right command name
///   - Surfaces errors verbatim from the backend
/// Vitest runs WITHOUT a real Tauri runtime, so invoke would reject anyway;
/// we provide a mockable surface so we can assert the dispatched command and
/// validate the TS-side guards beyond what the unit-test store covers.

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

// import AFTER the mock is registered.
import {
  ipcPlatformAdd, ipcPlatformRemove, ipcPlatformList,
  ipcSubscriptionAdd, ipcSubscriptionRemove, ipcSubscriptionList,
  ipcProcessRouteAdd, ipcProcessRouteRemove, ipcProcessRouteList,
  ipcPlatformUpdate, ipcNodeList,
  ipcPlatformCreateWithFields, ipcPlatformLeases,
  ipChannelList, ipChannelPolicySet, ipChannelCreate, ipChannelDelete,
  ipcPortList, ipcPortUpsert, ipcPortRemove, ipcPortRunning, ipcPortReload,
  ipcPortHealthCheck,
  ipcWhiteboxPath, ipcWhiteboxGet, ipcWhiteboxReload,
  ipcIpReputationSnapshot,
  ipcStrategyConfigGet, ipcStrategyConfigPut, ipcStrategyApply,
} from "./ipc";

describe("IPC wrappers (issue 1 closed-loops)", () => {
  beforeEach(() => { invokeMock.mockReset(); });

  it("platform_add forwards name and resolves on backend ok", async () => {
    invokeMock.mockResolvedValue(undefined);
    await ipcPlatformAdd("openai");
    expect(invokeMock).toHaveBeenCalledWith("platform_add", expect.objectContaining({ name: "openai" }));
  });

  it("platform_add rejects names with control chars (TS guard)", async () => {
    await expect(ipcPlatformAdd("bad\x00name")).rejects.toThrow(/platform invalid/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("platform_remove forwards name and returns backend bool", async () => {
    invokeMock.mockResolvedValue(true);
    const r = await ipcPlatformRemove("openai");
    expect(r).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("platform_remove", expect.objectContaining({ name: "openai" }));
  });

  it("platform_list returns the backend string array", async () => {
    invokeMock.mockResolvedValue(["a", "b"]);
    const r = await ipcPlatformList();
    expect(r).toEqual(["a", "b"]);
  });

  it("subscription_add validates URL shape (http prefix)", async () => {
    await expect(ipcSubscriptionAdd("n", "ftp://x")).rejects.toThrow(/http:/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("subscription_add forwards name + url to the backend", async () => {
    invokeMock.mockResolvedValue(undefined);
    await ipcSubscriptionAdd("n", "https://x.invalid/sub");
    expect(invokeMock).toHaveBeenCalledWith("subscription_add", expect.objectContaining({ name: "n", url: "https://x.invalid/sub" }));
  });

  it("subscription_remove forwards the name", async () => {
    invokeMock.mockResolvedValue(true);
    await ipcSubscriptionRemove("n");
    expect(invokeMock).toHaveBeenCalledWith("subscription_remove", expect.objectContaining({ name: "n" }));
  });

  it("subscription_list returns the expected typed shape", async () => {
    invokeMock.mockResolvedValue([{ name: "n", node_count: 7 }]);
    const r = await ipcSubscriptionList();
    expect(r).toEqual([{ name: "n", node_count: 7 }]);
  });

  it("process_route_add rejects port 50 (out of range: below 1024)", async () => {
    await expect(ipcProcessRouteAdd("ollama", 50)).rejects.toThrow(/out of range/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("process_route_add forwards process + targetPort", async () => {
    invokeMock.mockResolvedValue(undefined);
    await ipcProcessRouteAdd("ollama", 17990);
    expect(invokeMock).toHaveBeenCalledWith("process_route_add", expect.objectContaining({ process: "ollama", targetPort: 17990 }));
  });

  it("process_route_remove rejects empty process name", async () => {
    await expect(ipcProcessRouteRemove("")).rejects.toThrow(/process invalid/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("process_route_list forwards no args and returns typed array", async () => {
    invokeMock.mockResolvedValue([{ process: "ollama", target_port: 17990 }]);
    const r = await ipcProcessRouteList();
    expect(r).toEqual([{ process: "ollama", target_port: 17990 }]);
  });

  // Phase R1: platform_update + node_list closed-loop guards.
  it("platform_update rejects invalid allocation_policy before invoke", async () => {
    await expect(ipcPlatformUpdate("OpenAI", "RANDOM" as any)).rejects.toThrow(/allocation_policy must be one of/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("platform_update rejects too many regex_filters before invoke", async () => {
    const tooMany = Array(65).fill("api.openai.com");
    await expect(ipcPlatformUpdate("OpenAI", undefined, tooMany)).rejects.toThrow(/too many/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("platform_update forwards valid fields with null for omitted ones", async () => {
    invokeMock.mockResolvedValue({ ok: true });
    await ipcPlatformUpdate("OpenAI", "PREFER_LOW_LATENCY");
    expect(invokeMock).toHaveBeenCalledWith("platform_update", expect.objectContaining({
      name: "OpenAI",
      allocationPolicy: "PREFER_LOW_LATENCY",
      regexFilters: null,
      regionFilters: null,
      stickyTtl: null,
    }));
  });

  it("platform_update forwards regionFilters for B->C binding", async () => {
    invokeMock.mockResolvedValue({ ok: true });
    await ipcPlatformUpdate("OpenAI", undefined, undefined, ["hk", "us"]);
    expect(invokeMock).toHaveBeenCalledWith("platform_update", expect.objectContaining({
      name: "OpenAI",
      allocationPolicy: null,
      regexFilters: null,
      regionFilters: ["hk", "us"],
      stickyTtl: null,
    }));
  });

  it("platform_update rejects too many region_filters before invoke", async () => {
    const tooMany = Array(65).fill("hk");
    await expect(ipcPlatformUpdate("OpenAI", undefined, undefined, tooMany)).rejects.toThrow(/too many/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("node_list forwards no args", async () => {
    invokeMock.mockResolvedValue({ items: [] });
    const r = await ipcNodeList();
    expect(r).toEqual({ items: [] });
    expect(invokeMock).toHaveBeenCalledWith("node_list", expect.objectContaining({ __trace_id: expect.any(String) }));
  });

  // P21-B: platform_create_with_fields closed-loop guards
  it("platform_create_with_fields rejects non-object body", async () => {
    await expect(ipcPlatformCreateWithFields("not an object")).rejects.toThrow(/JSON object/);
    await expect(ipcPlatformCreateWithFields(null)).rejects.toThrow(/JSON object/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("platform_create_with_fields rejects missing name", async () => {
    await expect(ipcPlatformCreateWithFields({ allocation_policy: "BALANCED" })).rejects.toThrow(/missing 'name'/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("platform_create_with_fields rejects empty name", async () => {
    await expect(ipcPlatformCreateWithFields({ name: "" })).rejects.toThrow(/platform invalid/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("platform_create_with_fields forwards valid body", async () => {
    invokeMock.mockResolvedValue({ id: "abc-123" });
    const body = { name: "auto-deadbeef", allocation_policy: "BALANCED", regex_filters: [], region_filters: [] };
    await ipcPlatformCreateWithFields(body);
    expect(invokeMock).toHaveBeenCalledWith("platform_create_with_fields", expect.objectContaining({ body }));
  });

  // P21-B: platform_leases closed-loop guards
  it("platform_leases forwards name and returns value", async () => {
    invokeMock.mockResolvedValue({ items: [{ account: "acct1", egress_ip: "1.2.3.4" }] });
    const r = await ipcPlatformLeases("my-platform");
    expect(r).toEqual({ items: [{ account: "acct1", egress_ip: "1.2.3.4" }] });
    expect(invokeMock).toHaveBeenCalledWith("platform_leases", expect.objectContaining({ name: "my-platform" }));
  });

  it("platform_leases rejects empty name", async () => {
    await expect(ipcPlatformLeases("")).rejects.toThrow(/platform invalid/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  // P21-C: ip_channel_* wrapper tests — verify the semantic aliases forward
  // to the correct Resin-backed IPC commands.
  it("ip_channel_list forwards to node_list", async () => {
    invokeMock.mockResolvedValue({ items: [] });
    const r = await ipChannelList();
    expect(r).toEqual({ items: [] });
    expect(invokeMock).toHaveBeenCalledWith("node_list", expect.objectContaining({ __trace_id: expect.any(String) }));
  });

  it("ip_channel_policy_set maps BALANCED → platform_update", async () => {
    invokeMock.mockResolvedValue({ ok: true });
    await ipChannelPolicySet("my-egress", "BALANCED");
    expect(invokeMock).toHaveBeenCalledWith("platform_update", expect.objectContaining({
      name: "my-egress",
      allocationPolicy: "BALANCED",
      regexFilters: null,
      regionFilters: null,
      stickyTtl: null,
    }));
  });

  it("ip_channel_policy_set maps PREFER_LOW_LATENCY → platform_update", async () => {
    invokeMock.mockResolvedValue({ ok: true });
    await ipChannelPolicySet("fast-egress", "PREFER_LOW_LATENCY");
    expect(invokeMock).toHaveBeenCalledWith("platform_update", expect.objectContaining({
      name: "fast-egress",
      allocationPolicy: "PREFER_LOW_LATENCY",
      regexFilters: null,
      regionFilters: null,
      stickyTtl: null,
    }));
  });

  it("ip_channel_policy_set maps PREFER_IDLE_IP → platform_update", async () => {
    invokeMock.mockResolvedValue({ ok: true });
    await ipChannelPolicySet("idle-egress", "PREFER_IDLE_IP");
    expect(invokeMock).toHaveBeenCalledWith("platform_update", expect.objectContaining({
      name: "idle-egress",
      allocationPolicy: "PREFER_IDLE_IP",
      regexFilters: null,
      regionFilters: null,
      stickyTtl: null,
    }));
  });

  it("ip_channel_policy_set rejects invalid policy before invoke", async () => {
    await expect(ipChannelPolicySet("x", "INVALID" as any)).rejects.toThrow(/allocation_policy must be one of/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("ip_channel_create forwards to platform_create_with_fields", async () => {
    invokeMock.mockResolvedValue({ id: "new-id" });
    const body = { name: "auto-newuid1", allocation_policy: "BALANCED" };
    await ipChannelCreate(body);
    expect(invokeMock).toHaveBeenCalledWith("platform_create_with_fields", expect.objectContaining({ body }));
  });

  it("ip_channel_delete forwards to platform_remove", async () => {
    invokeMock.mockResolvedValue(true);
    const r = await ipChannelDelete("auto-newuid1");
    expect(r).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("platform_remove", expect.objectContaining({ name: "auto-newuid1" }));
  });

  it("ip_channel_delete rejects empty name", async () => {
    await expect(ipChannelDelete("")).rejects.toThrow(/ip_channel invalid/);
    expect(invokeMock).not.toHaveBeenCalled();
  });
});


describe("port IPC (P2 multi-port thin forwarder)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("ipcPortList forwards to port_list and normalizes non-array", async () => {
    invokeMock.mockResolvedValueOnce([{ port: 17990, protocol: "socks5", platform_name: "Default", account: "port-17990", label: "a", enabled: true }]);
    const rows = await ipcPortList();
    expect(rows).toHaveLength(1);
    expect(invokeMock).toHaveBeenCalledWith("port_list", expect.objectContaining({ __trace_id: expect.any(String) }));
    invokeMock.mockResolvedValueOnce(null);
    expect(await ipcPortList()).toEqual([]);
  });

  it("ipcPortUpsert validates range/protocol and forwards camelCase args", async () => {
    invokeMock.mockResolvedValue({ port: 17990, protocol: "socks5", platform_name: "OpenAI", account: "port-17990", label: "k", enabled: true });
    await ipcPortUpsert({ port: 17990, protocol: "SOCKS5", platform_name: "OpenAI", account: "port-17990", label: "k", enabled: true });
    expect(invokeMock).toHaveBeenCalledWith("port_upsert", expect.objectContaining({
      port: 17990,
      protocol: "socks5",
      platformName: "OpenAI",
      account: "port-17990",
      label: "k",
      enabled: true,
    }));
  });

  it("ipcPortUpsert rejects privileged port and bad protocol before invoke", async () => {
    await expect(ipcPortUpsert({ port: 80, protocol: "socks5", platform_name: "OpenAI" })).rejects.toThrow(/port out of range/);
    await expect(ipcPortUpsert({ port: 17990, protocol: "ftp", platform_name: "OpenAI" })).rejects.toThrow(/protocol must be socks5 or http/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("ipcIpReputationSnapshot forwards and normalizes malformed responses", async () => {
    invokeMock.mockResolvedValueOnce({ provider: "ip_api", status: "ok", entries: [{ ip: "1.1.1.1", score: 5 }] });
    await expect(ipcIpReputationSnapshot()).resolves.toMatchObject({ provider: "ip_api", status: "ok" });
    expect(invokeMock).toHaveBeenLastCalledWith("ip_reputation_snapshot", expect.objectContaining({ __trace_id: expect.any(String) }));
    invokeMock.mockResolvedValueOnce({ provider: "ip_api", status: "ok" });
    await expect(ipcIpReputationSnapshot()).resolves.toEqual({ provider: null, status: "disabled", entries: [] });
  });

  it("ipcIpReputationSnapshot forwards and normalizes malformed responses", async () => {
    invokeMock.mockResolvedValueOnce({ provider: "ip_api", status: "ok", entries: [{ ip: "1.1.1.1", score: 5 }] });
    await expect(ipcIpReputationSnapshot()).resolves.toMatchObject({ provider: "ip_api", status: "ok" });
    expect(invokeMock).toHaveBeenLastCalledWith("ip_reputation_snapshot", expect.objectContaining({ __trace_id: expect.any(String) }));
    invokeMock.mockResolvedValueOnce({ provider: "ip_api", status: "ok" });
    await expect(ipcIpReputationSnapshot()).resolves.toEqual({ provider: null, status: "disabled", entries: [] });
  });

  it("ipcPortRemove / ipcPortRunning / ipcPortReload forward", async () => {
    invokeMock.mockResolvedValueOnce(true);
    await expect(ipcPortRemove(17990)).resolves.toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("port_remove", expect.objectContaining({ port: 17990 }));

    invokeMock.mockResolvedValueOnce([17990, 17991]);
    await expect(ipcPortRunning()).resolves.toEqual([17990, 17991]);
    expect(invokeMock).toHaveBeenCalledWith("port_running", expect.objectContaining({ __trace_id: expect.any(String) }));

    invokeMock.mockResolvedValueOnce(2);
    await expect(ipcPortReload()).resolves.toBe(2);
    expect(invokeMock).toHaveBeenCalledWith("port_reload", expect.objectContaining({ __trace_id: expect.any(String) }));
  });

  it("ipcPortRemove rejects privileged port before invoke", async () => {
    await expect(ipcPortRemove(443)).rejects.toThrow(/port out of range/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("ipcWhiteboxPath/Get/Reload forward", async () => {
    invokeMock.mockResolvedValueOnce("C:/cfg/egressapikey-ports.json");
    await expect(ipcWhiteboxPath()).resolves.toBe("C:/cfg/egressapikey-ports.json");
    expect(invokeMock).toHaveBeenCalledWith("whitebox_path", expect.objectContaining({ __trace_id: expect.any(String) }));
    invokeMock.mockResolvedValueOnce({ version: 1, entry_ports: [{ port: 17990, protocol: "socks5", platform_name: "OpenAI", account: "port-17990", label: "", enabled: true }] });
    const cfg = await ipcWhiteboxGet();
    expect(cfg.version).toBe(1);
    expect(cfg.entry_ports).toHaveLength(1);
    expect(invokeMock).toHaveBeenCalledWith("whitebox_get", expect.objectContaining({ __trace_id: expect.any(String) }));
    invokeMock.mockResolvedValueOnce(3);
    await expect(ipcWhiteboxReload()).resolves.toBe(3);
    expect(invokeMock).toHaveBeenCalledWith("whitebox_reload", expect.objectContaining({ __trace_id: expect.any(String) }));
  });

  it("ipcPortHealthCheck forwards port + protocol", async () => {
    invokeMock.mockResolvedValueOnce({ port: 17990, reachable: true, socks5_ok: true, protocol_mismatch: false, latency_ms: 5, reason: "ok" });
    await ipcPortHealthCheck(17990, "socks5");
    expect(invokeMock).toHaveBeenCalledWith("port_health_check", expect.objectContaining({ port: 17990, protocol: "socks5" }));
  });

  it("ipcPortHealthCheck defaults to socks5 when protocol omitted", async () => {
    invokeMock.mockResolvedValueOnce({ port: 17991, reachable: true, socks5_ok: true, protocol_mismatch: false, latency_ms: 3, reason: "ok" });
    await ipcPortHealthCheck(17991);
    expect(invokeMock).toHaveBeenCalledWith("port_health_check", expect.objectContaining({ port: 17991, protocol: "socks5" }));
  });

  it("ipcPortHealthCheck rejects invalid protocol before invoke", async () => {
    await expect(ipcPortHealthCheck(17990, "ftp")).rejects.toThrow(/protocol must be socks5 or http/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

describe("strategy IPC (T4-4)", () => {
  it("ipcStrategyConfigGet forwards to strategy_config_get", async () => {
    invokeMock.mockResolvedValueOnce({ version: 1, platforms: [] });
    const cfg = await ipcStrategyConfigGet();
    expect(cfg.version).toBe(1);
    expect(cfg.platforms).toEqual([]);
    expect(invokeMock).toHaveBeenCalledWith("strategy_config_get", expect.objectContaining({ __trace_id: expect.any(String) }));
  });

  it("ipcStrategyConfigPut validates and forwards to strategy_config_put", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await ipcStrategyConfigPut({ version: 1, platforms: [{ platform_name: "Test", a_class: "manual", b_class: "balanced" }] });
    expect(invokeMock).toHaveBeenCalledWith("strategy_config_put", expect.objectContaining({ config: expect.any(Object) }));
  });

  it("ipcStrategyConfigPut rejects version != 1 before invoke", async () => {
    await expect(ipcStrategyConfigPut({ version: 2, platforms: [] } as any)).rejects.toThrow("version must be 1");
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("ipcStrategyConfigPut rejects too many regions before invoke", async () => {
    const regions = Array.from({ length: 65 }, (_, i) => "r" + i);
    await expect(ipcStrategyConfigPut({ version: 1, platforms: [{ platform_name: "T", a_class: "region", b_class: "balanced", regions }] } as any)).rejects.toThrow("regions");
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("ipcStrategyApply forwards to strategy_apply", async () => {
    invokeMock.mockResolvedValueOnce({ platforms: [{ platform: "T", region_filters: ["US"], patched: true }] });
    const result = await ipcStrategyApply();
    expect(result.platforms[0].patched).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("strategy_apply", expect.objectContaining({ __trace_id: expect.any(String) }));
  });
});
});
