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

// ---- Subscriptions + node pool (G4) ----
/// Resin-level (name, node_count) for each imported subscription.
export interface SubscriptionSnapshotEntry {
  name: string;
  node_count: number;
}

export async function ipcSubscriptionAdd(name: string, url: string): Promise<void> {
  assertShortName(name, "subscription");
  if (!url || url.length > 4096) throw new Error("subscription url invalid");
  if (!/^https?:\/\//.test(url)) throw new Error("subscription url must start with http:// or https://");
  await invoke("subscription_add", { name, url });
}

export async function ipcSubscriptionRemove(name: string): Promise<boolean> {
  assertShortName(name, "subscription");
  return invoke<boolean>("subscription_remove", { name });
}

export async function ipcSubscriptionList(): Promise<SubscriptionSnapshotEntry[]> {
  return invoke<SubscriptionSnapshotEntry[]>("subscription_list");
}

export async function ipcNodePoolSnapshot(): Promise<{ total_nodes: number; healthy_nodes: number; egress_ip_count: number; healthy_egress_ip_count: number }> {
  return invoke("node_pool_snapshot");
}


/// Backup: create zip of settings + resin state, return temp path.
export async function ipcBackupCreate(): Promise<string> {
  return invoke<string>("backup_create");
}

/// Backup: upload zip to WebDAV server.
export async function ipcBackupUpload(url: string, username: string, password: string, zipPath: string): Promise<void> {
  if (!url || url.length > 2048) throw new Error("webdav url invalid");
  if (!/^https?:\/\//.test(url)) throw new Error("webdav url must start with http:// or https://");
  if (!zipPath) throw new Error("zip path must be non-empty");
  await invoke("backup_upload", { url, username, password, zipPath });
}

/// Backup: list backups on WebDAV server.
export async function ipcBackupList(url: string, username: string, password: string): Promise<string[]> {
  if (!url || url.length > 2048) throw new Error("webdav url invalid");
  if (!/^https?:\/\//.test(url)) throw new Error("webdav url must start with http:// or https://");
  return invoke<string[]>("backup_list", { url, username, password });
}
