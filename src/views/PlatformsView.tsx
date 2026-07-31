import { useEffect, useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Trash2, Link2, Loader2, AlertCircle, CheckCircle2 } from "lucide-react";
import { useAppStore } from "../store/appStore";
import {
  ipcPlatformAdd,
  ipcPlatformRemove,
  ipcPlatformList,
  ipcPlatformSnapshot,
  ipcAccountAdd,
  ipcAccountBindIp,
  type Account,
} from "../lib/ipc";

/// Platforms view: the Resin Platform/Account model surfaced in the UI.
/// Each Platform owns a set of routable accounts; an account binds to a
/// lane and an anchored exit IP. This is the fork's defining feature —
/// not a generic settings table. On IPC failure (running outside Tauri or
/// the registry not yet wired) we fall back to the local appStore reducer
/// so the dev preview still renders.
export function PlatformsView() {
  const { t } = useTranslation();
  const laneCount = useAppStore((s) => s.laneCount);
  const localSetPlatforms = useAppStore((s) => s.setPlatforms);
  const localPlatforms = useAppStore((s) => s.platforms);
  const addPlatform = useAppStore((s) => s.addPlatform);
  const removePlatform = useAppStore((s) => s.removePlatform);
  const addAccount = useAppStore((s) => s.addAccount);
  const bindExitIp = useAppStore((s) => s.bindExitIp);

  const [newPlatform, setNewPlatform] = useState("");
  const [busy, setBusy] = useState(false);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [ipEdits, setIpEdits] = useState<Record<string, string>>({});
  const [newAcct, setNewAcct] = useState<{ platform: string; id: string; lane: string }>({ platform: "", id: "", lane: "0" });
  const [toast, setToast] = useState<{ kind: "ok" | "err"; msg: string } | null>(null);

  // Pull the live platform list + per-platform snapshots from the Resin
  // registry over IPC. On any failure (no Tauri, command missing) we keep
  // the local reducer state from appStore so the screen is never blank.
  const refresh = useCallback(async () => {
    try {
      const names = await ipcPlatformList();
      const snaps = await Promise.all(names.map((n) => ipcPlatformSnapshot(n).catch(() => [] as Account[])));
      localSetPlatforms(names.map((n, i) => ({ name: n, accounts: snaps[i] as any, regexFilters: null, regionFilters: null, allocationPolicy: "BALANCED", routableNodeCount: 0, stickyTtl: "168h0m0s" })));
    } catch {
      // outside Tauri or registry uninstantiated — keep local store state
    }
  }, [localSetPlatforms]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const handleAddPlatform = async () => {
    const name = newPlatform.trim();
    if (!name) return;
    setBusy(true); setToast(null);
    addPlatform(name);
    try {
      await ipcPlatformAdd(name);
      await refresh();
      setToast({ kind: "ok", msg: t("platform.addOk") });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setToast({ kind: "err", msg });
    }
    setNewPlatform("");
    setBusy(false);
  };

  const handleRemove = async (name: string) => {
    setBusy(true);
    removePlatform(name);
    try {
      await ipcPlatformRemove(name);
      await refresh();
    } catch {
      // local reducer already updated
    }
    setBusy(false);
  };

  const handleAddAccount = async (platform: string) => {
    const id = newAcct.id.trim();
    if (!id) return;
    const lane = Math.max(0, Math.min(laneCount - 1, Number(newAcct.lane) || 0));
    setBusy(true);
    addAccount(platform, id, lane);
    try {
      await ipcAccountAdd(platform, id, lane);
      await refresh();
    } catch {
      // local fallback
    }
    setNewAcct({ platform: "", id: "", lane: "0" });
    setBusy(false);
  };

  const handleBindIp = async (platform: string, account: string) => {
    const ip = (ipEdits[`${platform}/${account}`] || "").trim();
    if (!ip) return;
    setBusy(true);
    bindExitIp(platform, account, ip);
    try {
      await ipcAccountBindIp(platform, account, ip);
      await refresh();
    } catch {
      // local fallback
    }
    setIpEdits((s) => ({ ...s, [`${platform}/${account}`]: "" }));
    setBusy(false);
  };

  return (
    <section className="h-full overflow-auto p-5 space-y-4">
      <header>
        <h2 className="text-sm font-semibold tracking-tight">{t("platform.title")}</h2>
      </header>

      {toast && (
        <div className={`flex items-center gap-2 text-xs px-3 py-2 rounded-md ${
          toast.kind === "ok"
            ? "bg-green-50 dark:bg-green-950/40 text-green-700 dark:text-green-300 border border-green-200 dark:border-green-900"
            : "bg-red-50 dark:bg-red-950/40 text-red-700 dark:text-red-300 border border-red-200 dark:border-red-900"
        }`}>
          {toast.kind === "ok" ? <CheckCircle2 size={14} /> : <AlertCircle size={14} />}
          <span>{toast.msg}</span>
        </div>
      )}
      <div className="flex gap-2">
        <input
          value={newPlatform}
          onChange={(e) => setNewPlatform(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && !busy && handleAddPlatform()}
          placeholder={t("platform.name")}
          className="flex-1 max-w-xs text-sm border border-zinc-200 dark:border-zinc-800 rounded-md px-2.5 py-1.5 bg-white dark:bg-zinc-900 focus:outline-none focus:ring-1 focus:ring-zinc-400 dark:focus:ring-zinc-600"
        />
        <button
          onClick={handleAddPlatform}
          disabled={busy || !newPlatform.trim()}
          className="inline-flex items-center gap-1 text-sm px-3 py-1.5 rounded-md bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900 disabled:opacity-40"
        >
          {busy ? <Loader2 size={14} className="animate-spin" /> : <Plus size={14} />}
          {t("platform.add")}
        </button>
      </div>

      {localPlatforms.length === 0 ? (
        <p className="text-xs text-zinc-400 dark:text-zinc-500">{t("platform.empty")}</p>
      ) : (
        <div className="space-y-2">
          {localPlatforms.map((p) => {
            const isOpen = expanded === p.name;
            return (
              <div key={p.name} className="border border-zinc-200 dark:border-zinc-800 rounded-md overflow-hidden">
                <div className="flex items-center justify-between px-3 py-2 bg-zinc-50 dark:bg-zinc-900/60">
                  <button
                    onClick={() => setExpanded(isOpen ? null : p.name)}
                    className="text-sm font-medium flex-1 text-left flex items-center gap-2"
                  >
                    {p.name}
                    <span className="text-xs text-zinc-400">({p.accounts.length})</span>
                  </button>
                  <button
                    onClick={() => handleRemove(p.name)}
                    aria-label={t("common.delete")}
                    className="text-zinc-400 hover:text-red-600 dark:hover:text-red-400 p-1"
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
                {isOpen && (
                  <div className="px-3 py-2 space-y-2 border-t border-zinc-200 dark:border-zinc-800">
                    {p.accounts.length === 0 && (
                      <p className="text-xs text-zinc-400">{t("platform.empty")}</p>
                    )}
                    {p.accounts.map((a: any) => (
                      <div key={a.id} className="flex flex-wrap items-center gap-2 text-xs">
                        <span className="font-mono px-1.5 py-0.5 rounded bg-zinc-100 dark:bg-zinc-800">{a.id}</span>
                        <span className="text-zinc-400">{t("account.lane")}: {a.lane}</span>
                        <span className="text-zinc-400">{t("account.exitIp")}: {a.exitIp ?? "—"}</span>
                        <input
                          value={ipEdits[`${p.name}/${a.id}`] || ""}
                          onChange={(e) => setIpEdits((s) => ({ ...s, [`${p.name}/${a.id}`]: e.target.value }))}
                          placeholder="IP"
                          className="w-28 text-xs border border-zinc-200 dark:border-zinc-800 rounded px-1.5 py-0.5 bg-white dark:bg-zinc-900"
                        />
                        <button
                          onClick={() => handleBindIp(p.name, a.id)}
                          disabled={busy}
                          className="inline-flex items-center gap-1 text-xs px-1.5 py-0.5 rounded border border-zinc-200 dark:border-zinc-700 hover:bg-zinc-100 dark:hover:bg-zinc-800 disabled:opacity-40"
                        >
                          <Link2 size={11} /> {t("account.bindIp")}
                        </button>
                      </div>
                    ))}
                    <div className="flex flex-wrap items-center gap-2 pt-1">
                      <input
                        value={newAcct.platform === p.name ? newAcct.id : ""}
                        onChange={(e) => setNewAcct((s) => ({ ...s, platform: p.name, id: e.target.value, lane: s.lane }))}
                        placeholder={t("account.id")}
                        className="w-36 text-xs border border-zinc-200 dark:border-zinc-800 rounded px-1.5 py-0.5 bg-white dark:bg-zinc-900"
                      />
                      <input
                        type="number"
                        min={0}
                        max={laneCount - 1}
                        value={newAcct.platform === p.name ? newAcct.lane : "0"}
                        onChange={(e) => setNewAcct((s) => ({ ...s, platform: p.name, lane: e.target.value }))}
                        aria-label={t("account.lane")}
                        className="w-16 text-xs border border-zinc-200 dark:border-zinc-800 rounded px-1.5 py-0.5 bg-white dark:bg-zinc-900"
                      />
                      <button
                        onClick={() => handleAddAccount(p.name)}
                        disabled={busy || (newAcct.platform === p.name && !newAcct.id.trim())}
                        className="inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900 disabled:opacity-40"
                      >
                        <Plus size={11} /> {t("account.add")}
                      </button>
                    </div>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
