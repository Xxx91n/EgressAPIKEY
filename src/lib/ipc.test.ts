import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

/// closed-loop coverage for the IPC boundary. We mock
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
  ipcWhiteboxBackupList, ipcWhiteboxRollback, ipcStrategyBackupList, ipcStrategyRollback,
  ipcPlatformAdd, ipcPlatformRemove, ipcPlatformList,
  ipcSubscriptionAdd, ipcSubscriptionRemove, ipcSubscriptionList,
  ipcSubscriptionRefresh,
  ipcProcessRouteAdd, ipcProcessRouteRemove, ipcProcessRouteList,
  ipcPlatformUpdate, ipcNodeList,
  ipcNodeProbe,
  ipcPlatformCreateWithFields, ipcPlatformLeases,
  ipChannelList, ipChannelPolicySet, ipChannelCreate, ipChannelDelete,
  ipcPortList, ipcPortUpsert,
  ipcPortToggle, ipcPortRemove, ipcPortRunning,
  ipcPortHealthCheck,
  ipcKeyAccountLookup,
  ipcOrchestrationGet, ipcOrchestrationConfigPut, ipcOrchestrationTick,
  ipcOrchestrationApprove, ipcOrchestrationDismiss,
  ipcProbeExitIp,
  ipcCheckFirewallStatus,
  ipcRequestLogTail,
  ipcWhiteboxPath, ipcWhiteboxGet, ipcWhiteboxReload,
  ipcIpReputationSnapshot,
  ipcStrategyConfigGet, ipcStrategyConfigPut, ipcStrategyApply, ipcStrategyPlatformRegionsSet,
  ipcAuthoritativeSnapshot,
  ipcReconcileNow, snapReconcilePlan,
  ipcSetLogLevel,
  ipcMetricsRealtimeThroughput, ipcMetricsProbeHistory,
  ipcRequestLogDetail, ipcRequestLogPayloads,
  assertLogId, decodePayloadPart, PAYLOAD_DISPLAY_CAP_BYTES,
  IpcUnavailableError,
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

  it("key_account_lookup trims, forwards key, rejects empty/overlong", async () => {
    invokeMock.mockResolvedValue([{ port: 17990, protocol: "socks5", platform_name: "Default", account: "sk-aaa", label: "", enabled: true, auth_required: true, leases: [{ egress_ip: "9.9.9.9", node_tag: "n1", target_domain: "api.x", ts: "t" }] }]);
    const hits = await ipcKeyAccountLookup("  sk-aaa  ");
    expect(invokeMock).toHaveBeenCalledWith("key_account_lookup", expect.objectContaining({ key: "sk-aaa" }));
    expect(hits[0].port).toBe(17990);
    expect(hits[0].leases[0].egress_ip).toBe("9.9.9.9");
    invokeMock.mockReset();
    await expect(ipcKeyAccountLookup("   ")).rejects.toThrow();
    await expect(ipcKeyAccountLookup("x".repeat(4097))).rejects.toThrow();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("orchestration wrappers dispatch + validate platform name", async () => {
    invokeMock.mockResolvedValue({ orchestration: null, autonomy: "suggest" });
    const st = await ipcOrchestrationGet();
    expect(st.autonomy).toBe("suggest");
    await ipcOrchestrationConfigPut({ enabled: true, consecutive_failure_threshold: 4 });
    expect(invokeMock).toHaveBeenCalledWith("orchestration_config_put", expect.objectContaining({ params: { enabled: true, consecutive_failure_threshold: 4 } }));
    invokeMock.mockResolvedValue({ enabled: false, actions: [] });
    await ipcOrchestrationTick();
    expect(invokeMock).toHaveBeenCalledWith("orchestration_tick", expect.objectContaining({}));
    invokeMock.mockResolvedValue(undefined);
    await ipcOrchestrationApprove("Default");
    expect(invokeMock).toHaveBeenCalledWith("orchestration_approve", expect.objectContaining({ platformName: "Default" }));
    await ipcOrchestrationDismiss("Default");
    expect(invokeMock).toHaveBeenCalledWith("orchestration_dismiss", expect.objectContaining({ platformName: "Default" }));
    invokeMock.mockReset();
    await expect(ipcOrchestrationApprove("")).rejects.toThrow();
    await expect(ipcOrchestrationDismiss("x".repeat(200))).rejects.toThrow();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("subscription_add validates URL shape (http prefix)", async () => {
    await expect(ipcSubscriptionAdd("n", "ftp://x")).rejects.toThrow(/http:/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("subscription_add forwards name + url to the backend (pipeline null by default)", async () => {
    invokeMock.mockResolvedValue(undefined);
    await ipcSubscriptionAdd("n", "https://x.invalid/sub");
    expect(invokeMock).toHaveBeenCalledWith("subscription_add", expect.objectContaining({ name: "n", url: "https://x.invalid/sub", pipeline: null }));
  });

  it("subscription_add forwards pipeline=establish when requested (round7 T01)", async () => {
    invokeMock.mockResolvedValue(undefined);
    await ipcSubscriptionAdd("n", "https://x.invalid/sub", undefined, "establish");
    expect(invokeMock).toHaveBeenCalledWith("subscription_add", expect.objectContaining({ pipeline: "establish" }));
  });

  it("subscription_add rejects a pipeline value other than establish", async () => {
    await expect(ipcSubscriptionAdd("n", "https://x.invalid/sub", undefined, "auto" as never)).rejects.toThrow(/establish/);
    expect(invokeMock).not.toHaveBeenCalled();
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

  it("T02: ipcSubscriptionRefresh forwards name and resolves changed result", async () => {
    const result = { node_count: 42, changed: true };
    invokeMock.mockResolvedValue(result);
    const r = await ipcSubscriptionRefresh("main");
    expect(r).toEqual(result);
    expect(invokeMock).toHaveBeenCalledWith("subscription_refresh", expect.objectContaining({ name: "main" }));
  });

  it("T19-P2: ipcSubscriptionRefresh validates name length", async () => {
    await expect(ipcSubscriptionRefresh("")).rejects.toThrow(/subscription invalid/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("T19-P3: ipcNodeProbe forwards hash + kind for latency", async () => {
    invokeMock.mockResolvedValue({ latency_ewma_ms: 89 });
    const r = (await ipcNodeProbe("abc123", "latency")) as { latency_ewma_ms: number };
    expect(r.latency_ewma_ms).toBe(89);
    expect(invokeMock).toHaveBeenCalledWith("node_probe", expect.objectContaining({ nodeHash: "abc123", kind: "latency" }));
  });

  it("T19-P3: ipcNodeProbe forwards hash + kind for egress", async () => {
    invokeMock.mockResolvedValue({ egress_ip: "1.2.3.4", region: "us", latency_ewma_ms: 12 });
    const r = (await ipcNodeProbe("abc123", "egress")) as { egress_ip: string };
    expect(r.egress_ip).toBe("1.2.3.4");
    expect(invokeMock).toHaveBeenCalledWith("node_probe", expect.objectContaining({ nodeHash: "abc123", kind: "egress" }));
  });

  it("T19-P3: ipcNodeProbe rejects control chars in hash (TS guard)", async () => {
    await expect(ipcNodeProbe("bad\x01hash", "latency")).rejects.toThrow(/node_hash invalid/);
    expect(invokeMock).not.toHaveBeenCalled();
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

  // (ADR-0042): port_toggle IPC forwards to Rust command with port + enabled; not privileged port.
  it("ipcPortToggle forwards port + enabled to port_toggle and rejects privileged port", async () => {
    invokeMock.mockResolvedValueOnce({ port: 17991, protocol: "socks5", platform_name: "Default", account: "port-17991", label: "k", enabled: false, auth_required: false });
    await ipcPortToggle(17991, false);
    expect(invokeMock).toHaveBeenCalledWith("port_toggle", expect.objectContaining({ port: 17991, enabled: false }));
    invokeMock.mockClear();
    await expect(ipcPortToggle(80, true)).rejects.toThrow(/out of range/);
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
    await expect(ipcPortUpsert({ port: 17990, protocol: "ftp", platform_name: "OpenAI" })).rejects.toThrow(/protocol must be socks5, http or mixed/);
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

  it("ipcPortRemove / ipcPortRunning forward", async () => {
    invokeMock.mockResolvedValueOnce(true);
    await expect(ipcPortRemove(17990)).resolves.toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("port_remove", expect.objectContaining({ port: 17990 }));

    invokeMock.mockResolvedValueOnce([17990, 17991]);
    await expect(ipcPortRunning()).resolves.toEqual([17990, 17991]);
    expect(invokeMock).toHaveBeenCalledWith("port_running", expect.objectContaining({ __trace_id: expect.any(String) }));
  });
  it("T8-1 ipcPortBindPlatform forwards port + platformName (empty = unbind)", async () => {
    const { ipcPortBindPlatform } = await import("./ipc");
    invokeMock.mockResolvedValue(true);
    const ok = await ipcPortBindPlatform(17990, "Default");
    expect(invokeMock).toHaveBeenCalledWith("port_bind_platform", expect.objectContaining({ port: 17990, platformName: "Default" }));
    expect(ok).toBe(true);
    // unbind = empty string
    await ipcPortBindPlatform(17990, "");
    expect(invokeMock).toHaveBeenLastCalledWith("port_bind_platform", expect.objectContaining({ port: 17990, platformName: "" }));
  });
  it("T8-1 ipcPortBindPlatform rejects invalid platformName chars", async () => {
    const { ipcPortBindPlatform } = await import("./ipc");
    await expect(ipcPortBindPlatform(17990, "bad\x00name")).rejects.toThrow();
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

  it("ipcPortHealthCheck defaults to mixed when protocol omitted", async () => {
    invokeMock.mockResolvedValueOnce({ port: 17991, reachable: true, socks5_ok: true, protocol_mismatch: false, latency_ms: 3, reason: "ok" });
    await ipcPortHealthCheck(17991);
    expect(invokeMock).toHaveBeenCalledWith("port_health_check", expect.objectContaining({ port: 17991, protocol: "mixed" }));
  });

  it("ipcPortHealthCheck rejects invalid protocol before invoke", async () => {
    await expect(ipcPortHealthCheck(17990, "ftp")).rejects.toThrow(/protocol must be socks5, http or mixed/);
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

    it("ipcStrategyPlatformRegionsSet forwards name + regions (ticket 10 deep IPC)", async () => {
      invokeMock.mockResolvedValue({ version: 1, platforms: [] });
      await ipcStrategyPlatformRegionsSet("openai", ["US", "HK"]);
      expect(invokeMock).toHaveBeenCalledWith("strategy_platform_regions_set", expect.objectContaining({ platformName: "openai", regions: ["US", "HK"] }));
    });

    it("ipcStrategyPlatformRegionsSet rejects bad name / region bounds before invoke", async () => {
      await expect(ipcStrategyPlatformRegionsSet("bad\x00name", ["US"])).rejects.toThrow(/platform invalid/);
      await expect(ipcStrategyPlatformRegionsSet("openai", ["x".repeat(33)])).rejects.toThrow(/region code invalid/);
      await expect(ipcStrategyPlatformRegionsSet("openai", Array.from({ length: 65 }, (_, i) => "R" + i))).rejects.toThrow(/regions list too long/);
      expect(invokeMock).not.toHaveBeenCalled();
    });

    it("ipcStrategyPlatformRegionsSet rejects unexpected response shape", async () => {
      invokeMock.mockResolvedValue({ version: 2 });
      await expect(ipcStrategyPlatformRegionsSet("openai", ["US"])).rejects.toThrow(/unexpected response shape/);
    });
  });
});

// probe_exit_ip IPC wrapper closed-loop.
describe("T6-4 ipcProbeExitIp", () => {
  it("forwards port + protocol to probe_exit_ip invoke", async () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({ port: 1790, protocol: "http", exit_ip: "1.2.3.4", latency_ms: 50, status: 200 });
    await ipcProbeExitIp(1790, "http");
    expect(invokeMock).toHaveBeenCalledWith("probe_exit_ip", expect.objectContaining({ port: 1790, protocol: "http" }));
  });

  it("rejects invalid protocol before invoke", () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    expect(() => ipcProbeExitIp(1790, "ftp")).toThrow(/protocol must be socks5, http or mixed/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("rejects out-of-range port before invoke", () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    expect(() => ipcProbeExitIp(80, "http")).toThrow(/out of range/);
    expect(invokeMock).not.toHaveBeenCalled();
  });
});

// Firewall + request log tail IPC wrappers.
describe("T6-5 firewall + request log tail", () => {
  it("ipcCheckFirewallStatus forwards to check_firewall_status", async () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({ platform: "windows", firewall_on: true, inbound_blocked: true, detail: "..." });
    await ipcCheckFirewallStatus();
    expect(invokeMock).toHaveBeenCalledWith("check_firewall_status", expect.anything());
  });

  it("ipcRequestLogTail forwards limit param", async () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue([]);
    await ipcRequestLogTail(10);
    expect(invokeMock).toHaveBeenCalledWith("request_log_tail", expect.objectContaining({ limit: 10 }));
  });

  it("ipcRequestLogTail omits limit param when not provided", async () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue([]);
    await ipcRequestLogTail();
    // When no limit is provided, the wrapper still passes an object (possibly with __trace_id).
    expect(invokeMock).toHaveBeenCalledWith("request_log_tail", expect.anything());
  });
});

describe("T19 (ADR-0064) metrics minimal-set wrappers", () => {
  it("ipcMetricsRealtimeThroughput forwards with no params and coerces items", async () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({ step_seconds: 10, items: [{ ts: "2026-09-04T00:00:00Z", ingress_bps: 1, egress_bps: 2 }] });
    const r = await ipcMetricsRealtimeThroughput();
    expect(invokeMock).toHaveBeenCalledWith("metrics_realtime_throughput", expect.anything());
    expect(r.items).toHaveLength(1);
    expect(r.step_seconds).toBe(10);
  });

  it("ipcMetricsRealtimeThroughput coerces missing items to empty array", async () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(null);
    const r = await ipcMetricsRealtimeThroughput();
    expect(r.items).toEqual([]);
  });

  it("ipcMetricsProbeHistory forwards RFC3339 from/to", async () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({ bucket_seconds: 60, items: [] });
    await ipcMetricsProbeHistory("2026-09-04T00:00:00Z", "2026-09-04T01:00:00Z");
    expect(invokeMock).toHaveBeenCalledWith(
      "metrics_probe_history",
      expect.objectContaining({ from: "2026-09-04T00:00:00Z", to: "2026-09-04T01:00:00Z" }),
    );
  });

  it("ipcMetricsProbeHistory rejects non-RFC3339 from (TS §7.5 guard)", async () => {
    invokeMock.mockClear();
    await expect(
      ipcMetricsProbeHistory("1727654400", undefined),
    ).rejects.toThrow(/RFC3339/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("ipcMetricsProbeHistory rejects from >= to", async () => {
    await expect(
      ipcMetricsProbeHistory("2026-09-04T01:00:00Z", "2026-09-04T00:00:00Z"),
    ).rejects.toThrow(/before/);
    await expect(
      ipcMetricsProbeHistory("2026-09-04T00:00:00Z", "2026-09-04T00:00:00Z"),
    ).rejects.toThrow(/before/);
  });

  it("ipcMetricsProbeHistory rejects windows over 7 days", async () => {
    await expect(
      ipcMetricsProbeHistory("2026-08-01T00:00:00Z", "2026-09-04T00:00:00Z"),
    ).rejects.toThrow(/7 days/);
    // from-only window (to defaults upstream to now) capped the same way.
    await expect(
      ipcMetricsProbeHistory("2026-08-01T00:00:00Z", undefined),
    ).rejects.toThrow(/7 days/);
  });

  it("ipcMetricsProbeHistory rejects future to", async () => {
    const far = new Date(Date.now() + 2 * 3600 * 1000).toISOString();
    await expect(ipcMetricsProbeHistory(undefined, far)).rejects.toThrow(/future/);
  });

  it("ipcMetricsProbeHistory omits params when both undefined", async () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({ bucket_seconds: 60, items: [] });
    await ipcMetricsProbeHistory();
    // The invoke wrapper injects __trace_id, so assert no from/to keys ride along.
    expect(invokeMock).toHaveBeenCalledWith(
      "metrics_probe_history",
      expect.not.objectContaining({ from: expect.anything(), to: expect.anything() }),
    );
  });
});

describe("T21 (Round 5) request-log detail + payload wrappers", () => {
  it("ipcRequestLogDetail forwards a valid UUID log_id", async () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({ id: "abc123", http_method: "POST", http_status: 200 });
    const r = await ipcRequestLogDetail("0b7fd2a8-1f3e-4c5d-9a6b-7c8d9e0f1a2b");
    expect(invokeMock).toHaveBeenCalledWith(
      "request_log_detail",
      expect.objectContaining({ logId: "0b7fd2a8-1f3e-4c5d-9a6b-7c8d9e0f1a2b" }),
    );
    expect(r.id).toBe("abc123");
    expect(r.http_status).toBe(200);
  });

  it("ipcRequestLogDetail rejects an oversized log_id before invoke (TS §7.5)", async () => {
    invokeMock.mockClear();
    await expect(ipcRequestLogDetail("a".repeat(65))).rejects.toThrow(/64/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("ipcRequestLogDetail rejects a non-UUID charset before invoke", async () => {
    invokeMock.mockClear();
    await expect(ipcRequestLogDetail("../etc/passwd")).rejects.toThrow(/UUID/);
    await expect(ipcRequestLogDetail("has space")).rejects.toThrow(/UUID/);
    await expect(ipcRequestLogDetail("")).rejects.toThrow(/64/);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("ipcRequestLogDetail coerces a null wire face to an all-default detail", async () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(null);
    const r = await ipcRequestLogDetail("abc123");
    expect(r.id).toBe("");
    expect(r.payload_present).toBe(false);
    expect(r.http_status).toBe(0);
  });

  it("ipcRequestLogPayloads forwards log_id and coerces the truncated flags", async () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({
      req_headers_b64: btoa("x: 1"),
      req_body_b64: btoa('{"a":1}'),
      resp_body_b64: btoa("ok"),
      truncated: { req_body: true },
    });
    const r = await ipcRequestLogPayloads("abc123");
    expect(invokeMock).toHaveBeenCalledWith("request_log_payloads", expect.objectContaining({ logId: "abc123" }));
    expect(r.req_body_b64).toBe(btoa('{"a":1}'));
    expect(r.truncated.req_body).toBe(true);
    expect(r.truncated.req_headers).toBe(false);
    expect(r.resp_headers_b64).toBe("");
  });

  it("ipcRequestLogPayloads coerces a null wire face to empty strings", async () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(null);
    const r = await ipcRequestLogPayloads("abc123");
    expect(r.req_body_b64).toBe("");
    expect(r.truncated.resp_body).toBe(false);
  });

  it("assertLogId accepts UUID shape and rejects the rest", () => {
    expect(() => assertLogId("0b7fd2a8-1f3e-4c5d-9a6b-7c8d9e0f1a2b")).not.toThrow();
    expect(() => assertLogId("a".repeat(64))).not.toThrow();
    expect(() => assertLogId("a".repeat(65))).toThrow(/64/);
    expect(() => assertLogId("")).toThrow();
    expect(() => assertLogId("a/b")).toThrow(/UUID/);
    expect(() => assertLogId("a\tb")).toThrow(/UUID/);
  });

  it("decodePayloadPart decodes base64 to UTF-8 text", () => {
    const r = decodePayloadPart(btoa('{"model":"gpt"}'));
    expect(r.text).toBe('{"model":"gpt"}');
    expect(r.displayTruncated).toBe(false);
    expect(r.bytes).toBe(15);
  });

  it("decodePayloadPart returns empty text for invalid/empty input", () => {
    expect(decodePayloadPart("").text).toBe("");
    expect(decodePayloadPart(undefined).text).toBe("");
    expect(decodePayloadPart("not!!valid").text).toBe("");
  });

  it("decodePayloadPart slices over-cap payloads before decode (1 MB display cap)", () => {
    const big = "A".repeat(PAYLOAD_DISPLAY_CAP_BYTES + 1000);
    const r = decodePayloadPart(btoa(big));
    expect(r.displayTruncated).toBe(true);
    expect(r.bytes).toBe(PAYLOAD_DISPLAY_CAP_BYTES + 1000);
    // Sliced at the cap before UTF-8 decode.
    expect(r.text.length).toBeLessThanOrEqual(PAYLOAD_DISPLAY_CAP_BYTES);
  });
});


// --- dual-mode tests (isTauri=false → fetch fallback) ---
describe("T17 dual-mode: isTauri=false falls back to fetch", () => {
  let originalTauriInternals: unknown;
  let fetchMock: ReturnType<typeof vi.fn>;

  let originalTauriIsTauriFlag: unknown;
  beforeEach(() => {
    // Save the stubs set by setup.ts beforeAll, then unset them for fetch-mode tests.
    // setup.ts now sets BOTH globalThis.__TAURI_INTERNALS__ AND window.isTauri
    // (jsdom vitest: globalThis and window are distinct objects), so clear both
    // or isTauri() would still return true and we'd never reach the fetch branch.
    const w = globalThis as unknown as {
      __TAURI_INTERNALS?: unknown;
      isTauri?: unknown;
      fetch?: typeof fetch;
    };
    originalTauriInternals = w.__TAURI_INTERNALS;
    originalTauriIsTauriFlag = w.isTauri;
    w.__TAURI_INTERNALS = undefined;
    w.isTauri = undefined;
    // Provide a mock fetch; restore in afterEach
    fetchMock = vi.fn();
    w.fetch = fetchMock as unknown as typeof fetch;
  });

  afterEach(() => {
    const w = globalThis as unknown as {
      __TAURI_INTERNALS?: unknown;
      isTauri?: unknown;
    };
    w.__TAURI_INTERNALS = originalTauriInternals;
    w.isTauri = originalTauriIsTauriFlag;
  });

  it("platform_add forwards POST /api/v1/platforms via fetch when isTauri=false", async () => {
    fetchMock.mockResolvedValueOnce(new Response(JSON.stringify({ ok: true }), { status: 200, headers: { "Content-Type": "application/json" } }));
    await ipcPlatformAdd("openai");
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/v1/platforms");
    expect((init as RequestInit).method).toBe("POST");
    const body = JSON.parse((init as RequestInit).body as string);
    expect(body.name).toBe("openai");
    // __trace_id must NOT be injected for the fetch path (only Tauri uses it)
    expect(body.__trace_id).toBeUndefined();
  });

  it("platform_list forwards GET /api/v1/platforms via fetch when isTauri=false", async () => {
    fetchMock.mockResolvedValueOnce(new Response(JSON.stringify({ items: [{ name: "a" }, { name: "b" }] }), { status: 200, headers: { "Content-Type": "application/json" } }));
    const r = await ipcPlatformList() as unknown;
    // invokeHttp unwraps Resin's {items:[...]} wrapper AND projects each row to
    // its `name` field. Which side is right, argued from both ends:
    //   - the Tauri command is `platform_list -> Result<Vec<String>>`
    //     (commands/platform.rs returns platform_names(&list));
    //   - this wrapper is typed Promise<string[]> to match it;
    //   - the headless route therefore declares `unwrapItems` + `project: "name"`
    //     so the SPA sees the SAME shape in both modes (equivalent face).
    // rework: the old object-array expectation contradicted
    // both the signature and its own comment above it.
    expect(r).toEqual(["a", "b"]);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/v1/platforms");
    expect((init as RequestInit).method).toBe("GET");
  });

  it("a headless-disabled command throws a typed IpcUnavailableError (no silent fallback)", async () => {
    // rework. A command with no HTTP route is no
    // longer an opaque missing-route string: every manifest command is either
    // MAPPED (CMD_TO_HTTP) or explicitly DISABLED with a typed reason, and the
    // UI reads that reason through ipcCommandAvailability() to render a
    // disabled state instead of a runtime surprise.
    //
    // get_config_dir stays disabled under desktop_only_local_path (ADR-0071
    // re-triage): it resolves a desktop filesystem path, a genuinely
    // local-only surface — everything else was promoted to a BFF route.
    const { ipcGetConfigDir } = await import("./ipc");
    const err: unknown = await ipcGetConfigDir().catch((e: unknown) => e);
    expect(err).toBeInstanceOf(IpcUnavailableError);
    const typed = err as IpcUnavailableError;
    expect(typed.name).toBe("IpcUnavailableError");
    expect(typed.command).toBe("get_config_dir");
    expect(typed.reason).toBe("desktop_only_local_path");
    expect(typed.i18nKey).toBe("ipc.disabled.desktop_only_local_path");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("fetch 4xx/5xx throws an error with status + a body excerpt", async () => {
    fetchMock.mockResolvedValueOnce(new Response("upstream conflict", { status: 409, headers: { "Content-Type": "text/plain" } }));
    await expect(ipcPlatformAdd("openai")).rejects.toThrow(/409.*upstream conflict/);
  });
  // --- name-based contract verified (BFF translation is server-side) ---
  // The SPA continues to send the business name in the JSON body to the
  // collection URL; the headless BFF (proxy_to_resin) resolves name -> id in
  // Rust before forwarding. These tests pin the SPA side of the contract so
  // a refactor of the BFF layer never silently breaks the frontend shape.
  it("platform_remove sends DELETE /api/v1/platforms with body {name} (BFF resolves id server-side)", async () => {
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 204 }));
    await ipcPlatformRemove("openai");
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/v1/platforms");
    expect((init as RequestInit).method).toBe("DELETE");
    // Body carries the business name; the BFF proxy reads it and rewrites.
    const body = JSON.parse((init as RequestInit).body as string);
    expect(body.name).toBe("openai");
    expect(body.__trace_id).toBeUndefined();
  });

  it("subscription_remove sends DELETE /api/v1/subscriptions with body {name} (BFF resolves id server-side)", async () => {
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 204 }));
    await ipcSubscriptionRemove("n");
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/v1/subscriptions");
    expect((init as RequestInit).method).toBe("DELETE");
    const body = JSON.parse((init as RequestInit).body as string);
    expect(body.name).toBe("n");
  });

  it("platform_update sends PATCH /api/v1/platforms with body {name, allocation_policy} (BFF resolves id server-side, strips name)", async () => {
    fetchMock.mockResolvedValueOnce(new Response(JSON.stringify({ ok: true }), { status: 200, headers: { "Content-Type": "application/json" } }));
    await ipcPlatformUpdate("openai", "BALANCED", undefined, undefined, undefined);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/v1/platforms");
    expect((init as RequestInit).method).toBe("PATCH");
    const body = JSON.parse((init as RequestInit).body as string);
    expect(body.name).toBe("openai");
    // SPA sends camelCase IPC keys; the headless BFF (proxy_to_resin)
    // rewrites to Resin snake_case AFTER this fetch (server-side contract).
    expect(body.allocationPolicy).toBe("BALANCED");
    expect(body.regexFilters).toBeNull();
    expect(body.regionFilters).toBeNull();
    expect(body.stickyTtl).toBeNull();
    expect(body.passive_circuit_breaker_disabled).toBeNull();
  });

});

// ---- (tauri-specta pilot): type-contract swap regression ----
describe("specta-pilot wrappers (ticket 09)", () => {
  beforeEach(() => { invokeMock.mockReset(); });

  it("ipcSetLogLevel rejects out-of-union values before invoke", async () => {
    await expect(ipcSetLogLevel("fatal")).rejects.toThrow("invalid log level");
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("ipcSetLogLevel forwards in-union values unchanged", async () => {
    invokeMock.mockResolvedValueOnce("debug");
    await expect(ipcSetLogLevel("debug")).resolves.toBe("debug");
    expect(invokeMock).toHaveBeenCalledWith("set_log_level", expect.objectContaining({ level: "debug" }));
  });
});

// --- snapshot metadata + acknowledged exemption sanitizers ---
describe("ticket 12: authoritative snapshot metadata + acknowledged", () => {
  beforeEach(() => { invokeMock.mockReset(); });

  it("ipcAuthoritativeSnapshot passes lastCheckedAt + divergent_since + acknowledged through", async () => {
    invokeMock.mockResolvedValueOnce({
      strategyVersion: 1,
      resinReachable: true,
      lastCheckedAt: 1_756_521_600,
      platforms: [
        { state: "divergent", platform_name: "alpha", platform_id: "id", whitebox_regions: ["jp"], resin_regions: ["us"], resin_allocation_policy: "BALANCED", b_class: "BALANCED", a_class: "region", manual_nodes: [], subscriptions: [], divergent_since: 1_756_521_590, acknowledged: true },
        { state: "consistent", platform_name: "beta", platform_id: "id2", regions: ["hk"], resin_allocation_policy: "BALANCED", b_class: "BALANCED", a_class: "region", manual_nodes: [], subscriptions: [], acknowledged: false },
      ],
      ports: [
        { state: "missingOnResin", port: 17990, platform_name: "alpha", protocol: "socks5", account: "a", label: "", auth_required: false, divergent_since: 1_756_521_591, acknowledged: false },
      ],
    });
    const snap = await ipcAuthoritativeSnapshot();
    expect(snap.lastCheckedAt).toBe(1_756_521_600);
    expect(snap.platforms[0]).toMatchObject({ state: "divergent", divergent_since: 1_756_521_590, acknowledged: true });
    expect(snap.platforms[1]).toMatchObject({ state: "consistent", acknowledged: false });
    expect(snap.ports[0]).toMatchObject({ state: "missingOnResin", divergent_since: 1_756_521_591 });
    expect(invokeMock).toHaveBeenCalledWith("authoritative_snapshot", expect.objectContaining({ __trace_id: expect.any(String) }));
  });

  it("ipcAuthoritativeSnapshot sanitizes malformed timestamps and non-boolean acknowledged", async () => {
    invokeMock.mockResolvedValueOnce({
      strategyVersion: 1,
      resinReachable: false,
      lastCheckedAt: "not-a-number",
      platforms: [
        { state: "divergent", platform_name: "alpha", platform_id: "id", whitebox_regions: [], resin_regions: [], resin_allocation_policy: "BALANCED", b_class: "BALANCED", a_class: "region", manual_nodes: [], subscriptions: [], divergent_since: -5, acknowledged: "yes" },
        { state: "divergent", platform_name: "huge", platform_id: "id", whitebox_regions: [], resin_regions: [], resin_allocation_policy: "BALANCED", b_class: "BALANCED", a_class: "region", manual_nodes: [], subscriptions: [], divergent_since: 9_999_999_999, acknowledged: 1 },
      ],
      ports: [],
    });
    const snap = await ipcAuthoritativeSnapshot();
    // malformed top-level timestamp degrades to 0
    expect(snap.lastCheckedAt).toBe(0);
    // out-of-range / negative timestamps are dropped (undefined)
    const d0 = snap.platforms[0] as Extract<typeof snap.platforms[0], { state: "divergent" }>;
    const d1 = snap.platforms[1] as Extract<typeof snap.platforms[1], { state: "divergent" }>;
    expect(d0.divergent_since).toBeUndefined();
    expect(d1.divergent_since).toBeUndefined();
    // acknowledged is a strict boolean: any non-true value becomes false
    expect(d0.acknowledged).toBe(false);
    expect(d1.acknowledged).toBe(false);
  });

  it("ipcWhiteboxGet sanitizes the acknowledged exemption list", async () => {
    invokeMock.mockResolvedValueOnce({
      version: 1,
      entry_ports: [],
      acknowledged: ["17990", "ok-name", 42, "", "x".repeat(200), "bad\u0000name"],
    });
    const cfg = await ipcWhiteboxGet();
    // non-string / empty / oversized / control-char members are dropped, cap 64
    expect(cfg.acknowledged).toEqual(["17990", "ok-name"]);
  });

  it("ipcWhiteboxGet omits acknowledged when absent", async () => {
    invokeMock.mockResolvedValueOnce({ version: 1, entry_ports: [] });
    const cfg = await ipcWhiteboxGet();
    expect(cfg.acknowledged).toBeUndefined();
  });

  it("ipcStrategyConfigGet sanitizes the acknowledged exemption list", async () => {
    invokeMock.mockResolvedValueOnce({ version: 1, platforms: [], acknowledged: ["Anthropic", "OpenAI"] });
    const cfg = await ipcStrategyConfigGet();
    expect(cfg.acknowledged).toEqual(["Anthropic", "OpenAI"]);
  });

  it("ipcStrategyConfigPut validates the acknowledged list before invoke", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await ipcStrategyConfigPut({ version: 1, platforms: [], acknowledged: ["A"] });
    expect(invokeMock).toHaveBeenCalledWith("strategy_config_put", expect.objectContaining({ config: expect.objectContaining({ acknowledged: ["A"] }) }));

    // too long
    await expect(ipcStrategyConfigPut({ version: 1, platforms: [], acknowledged: Array.from({ length: 65 }, (_, i) => "P" + i) } as any))
      .rejects.toThrow(/acknowledged list too long/);
    // non-string member
    await expect(ipcStrategyConfigPut({ version: 1, platforms: [], acknowledged: [42] } as any))
      .rejects.toThrow(/acknowledged entry invalid/);
    // control char
    await expect(ipcStrategyConfigPut({ version: 1, platforms: [], acknowledged: ["bad\u0000name"] } as any))
      .rejects.toThrow(/acknowledged entry invalid/);
    // duplicate
    await expect(ipcStrategyConfigPut({ version: 1, platforms: [], acknowledged: ["A", "A"] } as any))
      .rejects.toThrow(/duplicated/);
    // not an array
    await expect(ipcStrategyConfigPut({ version: 1, platforms: [], acknowledged: "A" } as any))
      .rejects.toThrow(/acknowledged must be a string array/);
  });

  it("ipcWhiteboxBackupList sanitizes untrusted entries (bad names/timestamps dropped)", async () => {
    invokeMock.mockResolvedValueOnce([
      { file_name: "egressapikey-ports.json.100.bak", unix_ts: 100, size_bytes: 12 },
      { file_name: "../evil.bak", unix_ts: 200, size_bytes: 1 },
      { file_name: "junk.txt", unix_ts: 300, size_bytes: 1 },
      { file_name: "egressapikey-ports.json.300.bak", unix_ts: -5, size_bytes: 1 },
      { file_name: "egressapikey-ports.json.400.bak", unix_ts: "NaN", size_bytes: 1 },
      { file_name: "egressapikey-strategy.json.500.bak", unix_ts: 500, size_bytes: 3 },
    ]);
    const list = await ipcWhiteboxBackupList();
    expect(list.map((e) => e.file_name)).toEqual([
      "egressapikey-ports.json.100.bak",
      "egressapikey-strategy.json.500.bak",
    ]);
  });

  it("ipcWhiteboxRollback validates the backup name before invoking", async () => {
    invokeMock.mockResolvedValueOnce(1);
    await ipcWhiteboxRollback("egressapikey-ports.json.100.bak");
    expect(invokeMock).toHaveBeenCalledWith("whitebox_rollback", expect.objectContaining({ backupName: "egressapikey-ports.json.100.bak" }));
    await expect(ipcWhiteboxRollback("../evil.bak")).rejects.toThrow(/backup_name invalid/);
    await expect(ipcWhiteboxRollback("no-timestamp.bak")).rejects.toThrow(/backup_name invalid/);
    await expect(ipcWhiteboxRollback("egressapikey-ports.json.100.secrets")).rejects.toThrow(/backup_name invalid/);
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it("ipcStrategyBackupList forwards and sanitizes; ipcStrategyRollback validates before invoke", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "strategy_backup_list") {
        return Promise.resolve([{ file_name: "egressapikey-strategy.json.100-1.bak", unix_ts: 100, size_bytes: 55 }]);
      }
      return Promise.resolve({ platforms: [] });
    });
    const list = await ipcStrategyBackupList();
    expect(list).toHaveLength(1);
    expect(list[0].file_name).toBe("egressapikey-strategy.json.100-1.bak");
    await ipcStrategyRollback("egressapikey-strategy.json.100-1.bak");
    expect(invokeMock).toHaveBeenCalledWith("strategy_rollback", expect.objectContaining({ backupName: "egressapikey-strategy.json.100-1.bak" }));
    await expect(ipcStrategyRollback("x.json.1.secrets")).rejects.toThrow(/backup_name invalid/);
  });
});

// --- reconcile plan/report sanitizers (untrusted backend data) ---
describe("ticket 14: reconcile plan + report wrappers", () => {
  beforeEach(() => { invokeMock.mockReset(); });

  it("snapReconcilePlan narrows an untrusted plan and drops malformed rows", () => {
    const plan = snapReconcilePlan({
      platforms: [
        { platform: "alpha", desired_regions: ["hk"], live_regions: ["us"], action: "patch_regions" },
        { platform: "", desired_regions: [], live_regions: [], action: "patch_regions" }, // no name -> dropped
        "junk", // non-object -> dropped
        { platform: "x".repeat(600), desired_regions: [], live_regions: [], action: "patch_regions" }, // oversized name truncated then kept? -> snapStr caps at 512, still non-empty
      ],
      ports: [
        { port: 17990, platform: "alpha", action: "create_endpoint" },
        { port: 70000, platform: "alpha", action: "create_endpoint" }, // out of range -> dropped
        { port: "NaN", platform: "alpha", action: "create_endpoint" }, // non-numeric -> dropped
      ],
    });
    expect(plan.platforms).toHaveLength(2); // empty-name and junk dropped
    expect(plan.platforms[0]).toEqual({ platform: "alpha", desired_regions: ["hk"], live_regions: ["us"], action: "patch_regions" });
    expect(plan.platforms[1].platform.length).toBe(512); // snapStr cap applied
    expect(plan.ports).toHaveLength(1);
    expect(plan.ports[0]).toEqual({ port: 17990, platform: "alpha", action: "create_endpoint" });
    // null/undefined input degrades to an empty plan
    expect(snapReconcilePlan(null)).toEqual({ platforms: [], ports: [] });
    expect(snapReconcilePlan(undefined)).toEqual({ platforms: [], ports: [] });
  });

  it("ipcReconcileNow sanitizes the report: bounded ports, numeric skip cap", async () => {
    invokeMock.mockResolvedValueOnce({
      strategy: { platforms: [{ platform: "alpha", region_filters: ["hk"], patched: true }] },
      portsRestored: [17990, 70000, "junk", 18000],
      portsSkipped: 3,
    });
    const report = await ipcReconcileNow();
    expect(report.strategy.platforms).toHaveLength(1);
    expect(report.portsRestored).toEqual([17990, 18000]); // range 1024..65535 filter
    expect(report.portsSkipped).toBe(3);
  });

  it("ipcReconcileNow degrades malformed responses to safe defaults", async () => {
    invokeMock.mockResolvedValueOnce(null);
    const report = await ipcReconcileNow();
    expect(report.strategy).toEqual({ platforms: [] });
    expect(report.portsRestored).toEqual([]);
    expect(report.portsSkipped).toBe(0);
  });
});


// --- ADR-0055: routes in the authoritative snapshot ---
describe("ticket 17: route snapshot sanitizers", () => {
  beforeEach(() => { invokeMock.mockReset(); });

  it("ipcAuthoritativeSnapshot passes route variants through with the routes array", async () => {
    invokeMock.mockResolvedValueOnce({
      strategyVersion: 1,
      resinReachable: true,
      lastCheckedAt: 100,
      platforms: [],
      ports: [],
      routes: [
        { state: "consistent", process: "webview.exe", target_port: 17990, acknowledged: false },
        { state: "missingOnResin", process: "ollama.exe", target_port: 17991, divergent_since: 90, acknowledged: true },
      ],
    });
    const snap = await ipcAuthoritativeSnapshot();
    expect(snap.routes).toHaveLength(2);
    expect(snap.routes[0]).toMatchObject({ state: "consistent", process: "webview.exe", target_port: 17990 });
    expect(snap.routes[1]).toMatchObject({ state: "missingOnResin", process: "ollama.exe", divergent_since: 90, acknowledged: true });
  });

  it("ipcAuthoritativeSnapshot drops malformed route entries and junk states", async () => {
    invokeMock.mockResolvedValueOnce({
      strategyVersion: 1,
      resinReachable: true,
      lastCheckedAt: 100,
      platforms: [],
      ports: [],
      routes: [
        { state: "consistent", process: "", target_port: 17990 }, // empty name dropped
        { state: "consistent", process: "ok.exe", target_port: 70000 }, // out-of-range port dropped
        { state: "unknown-state", process: "x.exe", target_port: 17990 }, // unknown variant dropped
        "junk",
        { state: "missingOnResin", process: "kept.exe", target_port: 17992, divergent_since: -1, acknowledged: "yes" },
      ],
    });
    const snap = await ipcAuthoritativeSnapshot();
    expect(snap.routes).toHaveLength(1);
    const only = snap.routes[0] as Extract<typeof snap.routes[0], { state: "missingOnResin" }>;
    expect(only.process).toBe("kept.exe");
    expect(only.divergent_since).toBeUndefined();
    expect(only.acknowledged).toBe(false);
  });

  it("snapshots WITHOUT a routes array degrade to an empty route list", async () => {
    invokeMock.mockResolvedValueOnce({
      strategyVersion: 1,
      resinReachable: false,
      lastCheckedAt: 0,
      platforms: [],
      ports: [],
    });
    const snap = await ipcAuthoritativeSnapshot();
    expect(snap.routes).toEqual([]);
  });
});

// --- subscription_phases sanitizers ---
describe("ticket T02: subscriptionPhases sanitizers", () => {
  it("passes well-formed phase rows through (camelCase parent field)", async () => {
    invokeMock.mockResolvedValueOnce({
      strategyVersion: 1,
      resinReachable: true,
      lastCheckedAt: 0,
      platforms: [],
      ports: [],
      subscriptionPhases: [
        { name: "sub-a", phase: "Establishing", stage: "platform" },
        { name: "sub-b", phase: "Failed", stage: "apply", phase_error: "PATCH 500" },
        { name: "sub-c", phase: "Converged" },
      ],
    });
    const snap = await ipcAuthoritativeSnapshot();
    expect(snap.subscriptionPhases).toEqual([
      { name: "sub-a", phase: "Establishing", stage: "platform" },
      { name: "sub-b", phase: "Failed", stage: "apply", phase_error: "PATCH 500" },
      { name: "sub-c", phase: "Converged" },
    ]);
  });

  it("degrades malformed phases to Never (identity), never to a terminal state", async () => {
    invokeMock.mockResolvedValueOnce({
      strategyVersion: 1,
      resinReachable: true,
      lastCheckedAt: 0,
      platforms: [],
      ports: [],
      subscriptionPhases: [
        { name: "junk", phase: "TotallyUnknown" },
        { name: "junk-stage", phase: "Establishing", stage: "not-a-stage" },
        null,
        { phase: "Converged" },
      ],
    });
    const snap = await ipcAuthoritativeSnapshot();
    // Rows without a name are dropped; junk phase/stage degrade to Never /
    // drop; the null row is dropped.
    expect(snap.subscriptionPhases).toEqual([
      { name: "junk", phase: "Never" },
      { name: "junk-stage", phase: "Establishing" },
    ]);
  });

  it("T04: passes a well-formed last_cascade_error through and drops it on rows without one", async () => {
    invokeMock.mockResolvedValueOnce({
      strategyVersion: 1,
      resinReachable: true,
      lastCheckedAt: 0,
      platforms: [],
      ports: [],
      subscriptionPhases: [
        {
          name: "sub-fail",
          phase: "Failed",
          stage: "apply",
          phase_error: "PATCH failed: 500",
          last_cascade_error: {
            stage: "apply",
            reason: "PATCH failed: 500",
            rollback_actions: [
              "sub: kept (user data)",
              "plat: deleted on Resin (cascade-created, apply failed)",
              "port: none (not run)",
              "apply: failed on sub-fail",
            ],
          },
        },
        { name: "sub-ok", phase: "Converged" },
      ],
    });
    const snap = await ipcAuthoritativeSnapshot();
    expect(snap.subscriptionPhases?.[0].last_cascade_error).toEqual({
      stage: "apply",
      reason: "PATCH failed: 500",
      rollback_actions: [
        "sub: kept (user data)",
        "plat: deleted on Resin (cascade-created, apply failed)",
        "port: none (not run)",
        "apply: failed on sub-fail",
      ],
    });
    expect(snap.subscriptionPhases?.[1].last_cascade_error).toBeUndefined();
  });

  it("T04: a malformed cascade record drops WHOLE (no half-valid marking)", async () => {
    invokeMock.mockResolvedValueOnce({
      strategyVersion: 1,
      resinReachable: true,
      lastCheckedAt: 0,
      platforms: [],
      ports: [],
      subscriptionPhases: [
        { name: "bad-stage", phase: "Failed", stage: "apply", phase_error: "x", last_cascade_error: { stage: "not-a-stage", reason: "r" } },
        { name: "bad-reason", phase: "Failed", stage: "apply", phase_error: "x", last_cascade_error: { stage: "apply", reason: "  " } },
        { name: "nul-action", phase: "Failed", stage: "apply", phase_error: "x", last_cascade_error: { stage: "apply", reason: "r", rollback_actions: ["a\u0000b", " ok "] } },
        { name: "not-object", phase: "Failed", stage: "apply", phase_error: "x", last_cascade_error: "junk" },
      ],
    });
    const snap = await ipcAuthoritativeSnapshot();
    const rows = snap.subscriptionPhases ?? [];
    expect(rows[0].last_cascade_error).toBeUndefined();
    expect(rows[1].last_cascade_error).toBeUndefined();
    // Control chars in actions are sanitized to spaces, not fatal.
    expect(rows[2].last_cascade_error?.rollback_actions).toEqual(["a b", "ok"]);
    expect(rows[3].last_cascade_error).toBeUndefined();
  });

  it("snapshots WITHOUT a subscriptionPhases array degrade to an empty list", async () => {
    invokeMock.mockResolvedValueOnce({
      strategyVersion: 1,
      resinReachable: false,
      lastCheckedAt: 0,
      platforms: [],
      ports: [],
    });
    const snap = await ipcAuthoritativeSnapshot();
    expect(snap.subscriptionPhases).toEqual([]);
  });
});
