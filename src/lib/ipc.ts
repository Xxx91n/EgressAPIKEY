/**
 * Re3 IPC bridge: typed wrappers over Tauri commands exposing the Resin
 * Platform/Account registry. TS-layer validation per AGENTS s7.6.
 */
import { invoke } from "@tauri-apps/api/core";

const NAME_MAX = 128;
const AUTHORITY_MAX = 253;

function assertShortName(v: string, field: string): void {
  if (!v || v.length > NAME_MAX || /[\x00-\x1f\x7f]/.test(v)) {
    throw new Error(`${field} invalid (1..${NAME_MAX} chars, no control)`);
  }
}

function assertAuthority(v: string): void {
  if (!v || v.length > AUTHORITY_MAX || /[\x00\x01-\x1f\x7f]/.test(v.replace(/\t/g, ""))) {
    throw new Error(`authority invalid (1..${AUTHORITY_MAX} chars, no control)`);
  }
}

function assertIp(v: string): void {
  if (!v || v.length > 253 || /[\x00-\x1f\x7f\s]/.test(v)) {
    throw new Error("exit_ip invalid");
  }
}

export interface Account {
  id: string;
  platform: string;
  exit_ip: string | null;
  lane: number;
  active: boolean;
}

export interface SelectResult {
  account: string | null;
  lane: number;
  exit_ip: string | null;
  reason: string;
}

export interface LaneSnapshot {
  lane_count: number;
  busy: number;
  latencies: [string, number, number, number][];
}

export async function ipcPlatformAdd(name: string): Promise<void> {
  assertShortName(name, "platform");
  await invoke("platform_add", { name });
}

export async function ipcPlatformRemove(name: string): Promise<boolean> {
  assertShortName(name, "platform");
  return invoke<boolean>("platform_remove", { name });
}

export async function ipcPlatformList(): Promise<string[]> {
  return invoke<string[]>("platform_list");
}

export async function ipcPlatformSnapshot(name: string): Promise<Account[]> {
  assertShortName(name, "platform");
  return invoke<Account[]>("platform_snapshot", { name });
}

export async function ipcAccountAdd(platform: string, id: string, lane: number): Promise<void> {
  assertShortName(platform, "platform");
  assertShortName(id, "account");
  if (lane < 0 || lane >= 50) throw new Error(`lane ${lane} out of range (0..49)`);
  await invoke("account_add", { platform, id, lane });
}

export async function ipcAccountBindIp(platform: string, account: string, ip: string): Promise<boolean> {
  assertShortName(platform, "platform");
  assertShortName(account, "account");
  assertIp(ip);
  return invoke<boolean>("account_bind_ip", { platform, account, ip });
}

export async function ipcGatewaySelectAccount(
  platform: string,
  apiKey: string,
  authority: string,
  weighted: boolean,
): Promise<SelectResult> {
  assertShortName(platform, "platform");
  if (!apiKey) throw new Error("api_key must be non-empty");
  assertAuthority(authority);
  return invoke<SelectResult>("gateway_select_account", { platform, apiKey, authority, weighted });
}

export async function ipcRefreshTray(): Promise<void> {
  await invoke("tray_refresh_labels").catch(() => {});
}

export async function ipcGatewaySnapshot(): Promise<LaneSnapshot> {
  return invoke<LaneSnapshot>("gateway_snapshot");
}

export async function ipcReserve(apiKey: string, account: string, authority: string, exitIp: string | null): Promise<{ lane: number; lease: number | null; reason: string }> {
  if (!apiKey || !account) throw new Error("api_key and account must be non-empty");
  assertAuthority(authority);
  return invoke("gateway_reserve", { apiKey, account, authority, exitIp });
}

export async function ipcRecordLatency(authority: string, latencyMs: number): Promise<void> {
  assertAuthority(authority);
  const capped = Math.max(0, Math.min(latencyMs, 24 * 60 * 60 * 1000));
  await invoke("gateway_record_latency", { authority, latencyMs: capped });
}

export async function ipcRelease(lease: number | null): Promise<void> {
  await invoke("gateway_release", { lease });
}

export async function ipcEvictLane(lane: number): Promise<void> {
  if (lane < 0 || lane >= 50) throw new Error(`lane ${lane} out of range (0..49)`);
  await invoke("gateway_evict_lane", { lane });
}
