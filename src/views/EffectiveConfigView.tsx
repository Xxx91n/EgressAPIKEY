import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ClipboardCheck, History, Loader2, RefreshCw, RefreshCcwDot } from "lucide-react";
import {
  ipcAuthoritativeSnapshot,
  ipcReconcileNow,
  ipcStrategyBackupList,
  ipcStrategyRollback,
  ipcWhiteboxBackupList,
  ipcWhiteboxRollback,
  type AuthoritativeSnapshot,
  type StrategySnapshot,
  type WhiteboxBackupEntry,
} from "../lib/ipc";
import {
  reconcilePreviewFromSnapshot,
  reconcilePreviewIsEmpty,
  type ReconcilePreview,
} from "../lib/reconcile-preview";
import { translateError } from "../lib/i18n-error";
import { HeadlessCapabilityNotice, commandBlocked } from "../components/HeadlessCapabilityNotice";

// one-level "effective config" view
// (CONTEXT.md: Authoritative Snapshot; spec Implementation Decision 1/7).
// Consumer of the authoritative_snapshot IPC (ADR-0051): renders the
// pre-merged desired|live comparison per platform/port with the three-state
// badge (consistent / divergent / missingOnResin), a grey "known"
// degradation for acknowledged entities (ADR-0054 D: exemptions never enter
// the three-state merge), divergentSince per entry and lastCheckedAt on top.
// (ADR-0054 §A): the "sync to desired state" action — click opens
// the PREVIEW dialog computed from the snapshot already in memory (zero
// extra requests); confirm runs the serial reconcile_now (strategy apply ->
// ports restore, fail-fast, IpcError per ADR-0045) and auto re-verifies by
// re-pulling the snapshot; cancel closes with zero side effects; the button
// is disabled while a reconcile is in flight (no re-entry). ONE-WAY: the
// whitebox always wins; there is no "accept current state" button.
// (ADR-0054 §B) adds the versioned-whitebox history + rollback.

const badgeBase =
  "ml-2 inline-flex items-center rounded px-1.5 py-0.5 text-[11px] font-medium";

function fmtTs(ts: number | undefined): string {
  return ts ? new Date(ts * 1000).toLocaleString() : "";
}

function StateBadge(props: { state: string; acknowledged: boolean; resinReachable: boolean }) {
  const { t } = useTranslation();
  if (props.acknowledged) {
    return (
      <span
        data-testid="ec-badge-known"
        data-state={props.state}
        className={badgeBase + " bg-zinc-200 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-400"}
      >
        {t("effectiveConfig.known")}
      </span>
    );
  }
  if (props.state === "consistent") {
    return (
      <span
        data-testid="ec-badge-consistent"
        data-state="consistent"
        className={badgeBase + " bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-400"}
      >
        {t("effectiveConfig.consistent")}
      </span>
    );
  }
  if (props.state === "divergent") {
    return (
      <span
        data-testid="ec-badge-divergent"
        data-state="divergent"
        className={badgeBase + " bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-400"}
      >
        {t("effectiveConfig.divergent")}
      </span>
    );
  }
  // missingOnResin. Amber while the sidecar is reachable; neutral zinc when
  // resinReachable is false - ADR-0051: sidecar-down absence is NOT drift.
  const neutral = !props.resinReachable;
  return (
    <span
      data-testid="ec-badge-missing"
      data-state="missingOnResin"
      data-neutral={neutral ? "true" : "false"}
      className={
        badgeBase +
        (neutral
          ? " bg-zinc-200 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-400"
          : " bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-400")
      }
    >
      {t("effectiveConfig.missing")}
    </span>
  );
}

function desiredRegions(p: StrategySnapshot): string {
  return (p.state === "divergent" ? p.whitebox_regions : p.regions).join(", ");
}

// ADR-0058: top-level convergence chip. Data comes from the
// SAME 5s/open snapshot pull as the rest of the view (zero new requests).
// Converged => green "effective at HH:MM (rev N)"; ApplyFailed => red with
// the recorded reason; PendingApply => amber, clickable, opens the reconcile
// preview; the remaining phases (NeverApplied/Drifted/Unknown) render their
// own honest labels instead of masquerading as one of the three.
function ConvergeChip({ snap, onReconcile }: { snap: AuthoritativeSnapshot; onReconcile: () => void }) {
  const { t } = useTranslation();
  const phase = snap.convergePhase ?? "Unknown";
  const time = snap.lastApplyAt
    ? new Date(snap.lastApplyAt * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : "";
  const common = { "data-testid": "ec-converge-chip", "data-phase": phase } as const;
  const chip =
    "ml-2 inline-flex items-center rounded px-1.5 py-0.5 text-[11px] font-medium";
  if (phase === "Converged") {
    return (
      <span {...common} className={chip + " bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-400"}>
        {t("effectiveConfig.convergeConverged", { time: time || t("effectiveConfig.notRecorded"), rev: snap.strategyAppliedGeneration })}
      </span>
    );
  }
  if (phase === "ApplyFailed") {
    return (
      <span {...common} className={chip + " bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-400"}>
        {t("effectiveConfig.convergeFailed", { reason: snap.lastApplyError || t("effectiveConfig.notRecorded") })}
      </span>
    );
  }
  if (phase === "PendingApply") {
    return (
      <button
        {...common}
        onClick={onReconcile}
        title={t("effectiveConfig.reconcile")}
        className={chip + " bg-amber-100 text-amber-700 hover:bg-amber-200 dark:bg-amber-900/40 dark:text-amber-400"}
      >
        {t("effectiveConfig.convergePending")}
      </button>
    );
  }
  if (phase === "NeverApplied") {
    return (
      <span {...common} className={chip + " bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-400"}>
        {t("effectiveConfig.convergeNever")}
      </span>
    );
  }
  if (phase === "Drifted") {
    return (
      <span {...common} className={chip + " bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-400"}>
        {t("effectiveConfig.convergeDrifted")}
      </span>
    );
  }
  return (
    <span {...common} className={chip + " bg-zinc-200 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-400"}>
      {t("effectiveConfig.convergeUnknown")}
    </span>
  );
}

function liveRegions(p: StrategySnapshot): string {
  if (p.state === "consistent") return p.regions.join(", ");
  if (p.state === "divergent") return p.resin_regions.join(", ");
  return ""; // missingOnResin: no live value exists.
}

export function EffectiveConfigView() {
  const { t } = useTranslation();
  const [snap, setSnap] = useState<AuthoritativeSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  // (ADR-0054 section B): versioned-whitebox history + rollback.
  const [history, setHistory] = useState<{ strategy: WhiteboxBackupEntry[]; ports: WhiteboxBackupEntry[] }>({ strategy: [], ports: [] });
  const [confirmTarget, setConfirmTarget] = useState<{ store: "strategy" | "ports"; backup: WhiteboxBackupEntry } | null>(null);
  const [rollbackBusy, setRollbackBusy] = useState(false);
  // (ADR-0054 §A): reconcile preview dialog + in-flight guard.
  const [previewOpen, setPreviewOpen] = useState(false);
  const [reconciling, setReconciling] = useState(false);

  const refresh = useCallback(async () => {
    setBusy(true);
    setError("");
    try {
      const [nextSnap, stratH, portsH] = await Promise.all([
        ipcAuthoritativeSnapshot(),
        ipcStrategyBackupList().catch(() => [] as WhiteboxBackupEntry[]),
        ipcWhiteboxBackupList().catch(() => [] as WhiteboxBackupEntry[]),
      ]);
      setSnap(nextSnap);
      setHistory({ strategy: stratH, ports: portsH });
    } catch (e) {
      setError(translateError(e, t));
    } finally {
      setBusy(false);
    }
  }, [t]);

  // Pull once on open; re-checks are manual (spec: open fetch + manual
  // re-verify button). No polling in this view.
  useEffect(() => {
    void refresh();
  }, [refresh]);

// the confirm dialog shows the TARGET timestamp, then the
  // rollback re-enters the backend validate-before-swap -> apply chain and
  // the view re-pulls snapshot + history ( post-rollback
  // snapshot re-check).
  const performRollback = async (store: "strategy" | "ports", backup: WhiteboxBackupEntry) => {
    setRollbackBusy(true);
    setError("");
    try {
      if (store === "strategy") await ipcStrategyRollback(backup.file_name);
      else await ipcWhiteboxRollback(backup.file_name);
      setConfirmTarget(null);
      await refresh();
    } catch (e) {
      setError(translateError(e, t));
    } finally {
      setRollbackBusy(false);
    }
  };

  // §A: preview is derived from the snapshot ALREADY in memory
  // (spec: data comes from the snapshot, no new requests). Cancel closes
  // the dialog with zero side effects; confirm runs the serial reconcile
  // then re-pulls the snapshot so the user sees the fresh three-state.
  const preview: ReconcilePreview | null = snap ? reconcilePreviewFromSnapshot(snap) : null;

  const performReconcile = async () => {
    setReconciling(true);
    setError("");
    let reconcileError = "";
    try {
      await ipcReconcileNow();
      setPreviewOpen(false);
    } catch (e) {
      reconcileError = translateError(e, t);
    } finally {
// Auto re-verify on BOTH outcomes (执行后自动复验) — the
      // user must see the freshest three-state whether the pass converged
      // or failed. refresh() clears the error slot, so the reconcile error
      // is re-applied after the re-pull completes.
      await refresh();
      if (reconcileError) setError(reconcileError);
      setReconciling(false);
    }
  };

  const lastChecked =
    snap && snap.lastCheckedAt ? new Date(snap.lastCheckedAt * 1000).toLocaleString() : "";

  return (
    <div className="w-full max-w-none px-6 py-4 space-y-4" data-testid="ec-view">
      <HeadlessCapabilityNotice commands={["authoritative_snapshot", "reconcile_now", "strategy_backup_list", "strategy_rollback", "whitebox_backup_list", "whitebox_rollback"]} />
      <div className="flex items-center gap-2 flex-wrap">
        <ClipboardCheck size={16} strokeWidth={1.75} />
        <h2 className="text-sm font-semibold tracking-tight">{t("nav.effectiveConfig")}</h2>
        {/* Round 5 T09 / ADR-0058: top-level convergence chip (green
            Converged / red ApplyFailed / amber PendingApply-reconcile). */}
        {snap ? <ConvergeChip snap={snap} onReconcile={() => setPreviewOpen(true)} /> : null}
        <span className="text-xs text-zinc-500 dark:text-zinc-400" data-testid="ec-last-checked">
          {t("effectiveConfig.lastCheckedAt")}: {lastChecked || t("effectiveConfig.notRecorded")}
        </span>
        <button
          data-testid="ec-refresh"
          onClick={() => void refresh()}
          disabled={busy}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 hover:bg-zinc-50 dark:hover:bg-zinc-800 text-sm font-medium transition-colors disabled:opacity-50"
        >
          {busy ? <Loader2 size={14} className="animate-spin" /> : <RefreshCw size={14} strokeWidth={1.75} />}
          {t("effectiveConfig.refresh")}
        </button>
        {/* Ticket 14 §A: one-way sync-to-desired entry. Disabled while any
            snapshot fetch OR reconcile is in flight (no re-entry); hidden
            until the first snapshot exists (nothing to preview from). */}
        {snap ? (
          <button
            data-testid="ec-reconcile"
            onClick={() => setPreviewOpen(true)}
            disabled={busy || reconciling || rollbackBusy || reconcilePreviewIsEmpty(preview!)}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-blue-600 text-white hover:bg-blue-700 text-sm font-medium transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {reconciling ? <Loader2 size={14} className="animate-spin" /> : <RefreshCcwDot size={14} strokeWidth={1.75} />}
            {t("effectiveConfig.reconcile")}
          </button>
        ) : null}
        {error ? (
          <span data-testid="ec-error" className="text-xs text-red-500">{error}</span>
        ) : null}
      </div>
      {snap && !snap.resinReachable ? (
        <p data-testid="ec-resin-down" className="text-xs text-amber-600 dark:text-amber-400">
          {t("effectiveConfig.resinDown")}
        </p>
      ) : null}
      {snap === null && !error ? (
        <p data-testid="ec-loading" className="text-xs text-zinc-500 dark:text-zinc-400">
          {t("effectiveConfig.loading")}
        </p>
      ) : null}
      {snap && snap.platforms.length === 0 && snap.ports.length === 0 && snap.routes.length === 0 ? (
        <p data-testid="ec-no-entries" className="text-xs text-zinc-500 dark:text-zinc-400">
          {t("effectiveConfig.noEntries")}
        </p>
      ) : null}
      {snap && snap.platforms.length > 0 ? (
        <section data-testid="ec-strategy-section" className="space-y-2">
          <h3 className="text-xs font-semibold uppercase tracking-wide text-zinc-500 dark:text-zinc-400">
            {t("effectiveConfig.strategySection")}
          </h3>
          <ul className="space-y-2">
            {snap.platforms.map((p) => (
              <li
                key={p.platform_name}
                data-testid={"ec-platform-" + p.platform_name}
                className="rounded-md border border-zinc-200 dark:border-zinc-800 px-3 py-2 space-y-1"
              >
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="font-mono text-sm font-medium">{p.platform_name}</span>
                  <StateBadge state={p.state} acknowledged={p.acknowledged} resinReachable={snap.resinReachable} />
                  {p.state !== "consistent" && !p.acknowledged ? (
                    <span data-testid={"ec-divergent-since-" + p.platform_name} className="text-[11px] text-zinc-400">
                      {t("effectiveConfig.divergentSince")}: {fmtTs(p.divergent_since) || t("effectiveConfig.notRecorded")}
                    </span>
                  ) : null}
                </div>
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-1 font-mono text-xs">
                  <div>
                    <span className="text-zinc-400">{t("effectiveConfig.desired")}: </span>
                    {desiredRegions(p) || t("effectiveConfig.notRecorded")}
                  </div>
                  <div>
                    <span className="text-zinc-400">{t("effectiveConfig.live")}: </span>
                    {liveRegions(p) || t("effectiveConfig.notRecorded")}
                  </div>
                </div>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      {snap && snap.ports.length > 0 ? (
        <section data-testid="ec-ports-section" className="space-y-2">
          <h3 className="text-xs font-semibold uppercase tracking-wide text-zinc-500 dark:text-zinc-400">
            {t("effectiveConfig.portsSection")}
          </h3>
          <ul className="space-y-2">
            {snap.ports.map((pt) => (
              <li
                key={"port-" + pt.port}
                data-testid={"ec-port-" + pt.port}
                className="rounded-md border border-zinc-200 dark:border-zinc-800 px-3 py-2 space-y-1"
              >
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="font-mono text-sm font-medium">
                    {t("effectiveConfig.port")} {pt.port}
                  </span>
                  <span className="text-xs text-zinc-500 dark:text-zinc-400">
                    {pt.platform_name}
                    {pt.account ? " \u00b7 " + pt.account : ""}
                  </span>
                  <StateBadge state={pt.state} acknowledged={pt.acknowledged} resinReachable={snap.resinReachable} />
                  {pt.state === "missingOnResin" && !pt.acknowledged ? (
                    <span data-testid={"ec-divergent-since-port-" + pt.port} className="text-[11px] text-zinc-400">
                      {t("effectiveConfig.divergentSince")}: {fmtTs(pt.divergent_since) || t("effectiveConfig.notRecorded")}
                    </span>
                  ) : null}
                </div>
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-1 font-mono text-xs">
                  <div>
                    <span className="text-zinc-400">{t("effectiveConfig.desired")}: </span>
                    {pt.state === "consistent"
                      ? (pt.enabled ? t("effectiveConfig.enabled") : t("effectiveConfig.disabled")) + " \u00b7 " + pt.protocol
                      : t("effectiveConfig.enabled") + " \u00b7 " + pt.protocol}
                  </div>
                  <div>
                    <span className="text-zinc-400">{t("effectiveConfig.live")}: </span>
                    {pt.state === "consistent"
                      ? pt.protocol + (pt.auth_required ? " \u00b7 " + t("effectiveConfig.authRequired") : "")
                      : t("effectiveConfig.notRecorded")}
                  </div>
                </div>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      {snap && snap.routes.length > 0 ? (
        <section data-testid="ec-routes-section" className="space-y-2">
          <h3 className="text-xs font-semibold uppercase tracking-wide text-zinc-500 dark:text-zinc-400">
            {t("effectiveConfig.routesSection")}
          </h3>
          <ul className="space-y-2">
            {snap.routes.map((rr) => (
              <li
                key={"route-" + rr.process}
                data-testid={"ec-route-" + rr.process}
                className="rounded-md border border-zinc-200 dark:border-zinc-800 px-3 py-2 space-y-1"
              >
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="font-mono text-sm font-medium">{rr.process}</span>
                  <StateBadge state={rr.state} acknowledged={rr.acknowledged} resinReachable={snap.resinReachable} />
                  {rr.state === "missingOnResin" && !rr.acknowledged ? (
                    <span data-testid={"ec-divergent-since-route-" + rr.process} className="text-[11px] text-zinc-400">
                      {t("effectiveConfig.divergentSince")}: {fmtTs(rr.divergent_since) || t("effectiveConfig.notRecorded")}
                    </span>
                  ) : null}
                </div>
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-1 font-mono text-xs">
                  <div>
                    <span className="text-zinc-400">{t("effectiveConfig.desired")}: </span>
                    {t("effectiveConfig.route")} :{rr.target_port}
                  </div>
                  <div>
                    <span className="text-zinc-400">{t("effectiveConfig.live")}: </span>
                    {rr.state === "consistent"
                      ? t("effectiveConfig.route") + " :".concat(String(rr.target_port))
                      : t("effectiveConfig.notRecorded")}
                  </div>
                </div>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      {snap ? (
        <section data-testid="ec-history-section" className="space-y-2 border-t border-zinc-200 pt-3 dark:border-zinc-800">
          <h3 className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-zinc-500 dark:text-zinc-400">
            <History size={13} strokeWidth={1.75} />
            {t("effectiveConfig.historySection")}
          </h3>
          {(["strategy", "ports"] as const).map((store) => (
            <div key={store} className="space-y-1">
              <p className="text-[11px] text-zinc-500 dark:text-zinc-400">
                {t(store === "strategy" ? "effectiveConfig.strategySection" : "effectiveConfig.portsSection")}
              </p>
              {history[store].length === 0 ? (
                <p data-testid={"ec-history-empty-" + store} className="text-[11px] text-zinc-400">
                  {t("effectiveConfig.historyEmpty")}
                </p>
              ) : (
                <ul className="space-y-1">
                  {history[store].map((b) => (
                    <li key={b.file_name} data-testid={"ec-history-" + store + "-" + b.unix_ts} className="flex items-center gap-2 text-xs">
                      <span className="font-mono text-[11px] text-zinc-500 dark:text-zinc-400">
                        {new Date(b.unix_ts * 1000).toLocaleString()}
                      </span>
                      <span className="text-[11px] text-zinc-400">({b.size_bytes} B)</span>
                      <button
                        data-testid={"ec-rollback-" + store + "-" + b.unix_ts}
                        onClick={() => setConfirmTarget({ store, backup: b })}
                        disabled={rollbackBusy}
                        className="ml-auto inline-flex items-center gap-1 rounded border border-zinc-200 px-2 py-0.5 text-[11px] text-zinc-600 hover:bg-zinc-50 disabled:opacity-50 dark:border-zinc-700 dark:text-zinc-300 dark:hover:bg-zinc-800"
                      >
                        {t("effectiveConfig.rollbackTo")}
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          ))}
        </section>
      ) : null}
      {previewOpen && preview ? (
        <div data-testid="ec-preview-overlay" className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
          <div data-testid="ec-preview-dialog" className="w-full max-w-lg rounded-lg border border-zinc-200 bg-white p-4 shadow-lg space-y-3 dark:border-zinc-700 dark:bg-zinc-900">
            <h4 className="text-sm font-semibold">{t("effectiveConfig.reconcilePreviewTitle")}</h4>
            {reconcilePreviewIsEmpty(preview) ? (
              <p data-testid="ec-preview-empty" className="text-xs text-zinc-500 dark:text-zinc-400">
                {t("effectiveConfig.reconcileNothing")}
              </p>
            ) : (
              <div className="max-h-72 space-y-3 overflow-y-auto text-xs">
                {preview.platforms.length > 0 ? (
                  <div data-testid="ec-preview-platforms" className="space-y-1">
                    <p className="font-semibold text-zinc-600 dark:text-zinc-300">{t("effectiveConfig.strategySection")}</p>
                    <ul className="space-y-1">
                      {preview.platforms.map((p) => (
                        <li key={p.platform} data-testid={"ec-preview-platform-" + p.platform} className="font-mono">
                          <span className="font-medium">{p.platform}</span>
                          {" · "}
                          {p.action === "create_platform"
                            ? t("effectiveConfig.reconcileCreatePlatform")
                            : t("effectiveConfig.reconcilePatchRegions", {
                                desired: p.desired_regions.join(", ") || t("effectiveConfig.notRecorded"),
                                live: p.live_regions.join(", ") || t("effectiveConfig.notRecorded"),
                              })}
                        </li>
                      ))}
                    </ul>
                  </div>
                ) : null}
                {preview.ports.length > 0 ? (
                  <div data-testid="ec-preview-ports" className="space-y-1">
                    <p className="font-semibold text-zinc-600 dark:text-zinc-300">{t("effectiveConfig.portsSection")}</p>
                    <ul className="space-y-1">
                      {preview.ports.map((pt) => (
                        <li key={pt.port} data-testid={"ec-preview-port-" + pt.port} className="font-mono">
                          <span className="font-medium">{t("effectiveConfig.port")} {pt.port}</span>
                          {" · "}
                          {t("effectiveConfig.reconcileCreateEndpoint")}
                        </li>
                      ))}
                    </ul>
                  </div>
                ) : null}
              </div>
            )}
            <p className="text-[11px] text-zinc-400">{t("effectiveConfig.reconcileOneWay")}</p>
            <div className="flex justify-end gap-2">
              <button
                data-testid="ec-preview-cancel"
                onClick={() => setPreviewOpen(false)}
                disabled={reconciling}
                className="rounded-md border border-zinc-200 px-3 py-1.5 text-sm hover:bg-zinc-50 disabled:opacity-50 dark:border-zinc-700 dark:hover:bg-zinc-800"
              >
                {t("effectiveConfig.confirmCancel")}
              </button>
              <button
                data-testid="ec-preview-confirm"
                onClick={() => void performReconcile()}
                disabled={reconciling || reconcilePreviewIsEmpty(preview) || commandBlocked("reconcile_now")}
                className="rounded-md bg-blue-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50"
              >
                {reconciling ? t("effectiveConfig.reconcileWorking") : t("effectiveConfig.reconcileConfirm")}
                {reconciling ? <Loader2 size={13} className="ml-1.5 inline animate-spin" /> : null}
              </button>
            </div>
          </div>
        </div>
      ) : null}
      {confirmTarget ? (
        <div data-testid="ec-confirm-overlay" className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
          <div data-testid="ec-confirm-dialog" className="w-full max-w-sm rounded-lg border border-zinc-200 bg-white p-4 shadow-lg space-y-3 dark:border-zinc-700 dark:bg-zinc-900">
            <h4 className="text-sm font-semibold">{t("effectiveConfig.confirmTitle")}</h4>
            <p className="text-xs text-zinc-600 dark:text-zinc-300">
              <span data-testid="ec-confirm-time" className="font-mono">{new Date(confirmTarget.backup.unix_ts * 1000).toLocaleString()}</span>
              {t("effectiveConfig.confirmBody", {
                store: t(confirmTarget.store === "strategy" ? "effectiveConfig.strategySection" : "effectiveConfig.portsSection"),
                time: new Date(confirmTarget.backup.unix_ts * 1000).toLocaleString(),
              })}
            </p>
            <div className="flex justify-end gap-2">
              <button
                data-testid="ec-confirm-cancel"
                onClick={() => setConfirmTarget(null)}
                disabled={rollbackBusy}
                className="rounded-md border border-zinc-200 px-3 py-1.5 text-sm hover:bg-zinc-50 disabled:opacity-50 dark:border-zinc-700 dark:hover:bg-zinc-800"
              >
                {t("effectiveConfig.confirmCancel")}
              </button>
              <button
                data-testid="ec-confirm-rollback"
                onClick={() => void performRollback(confirmTarget.store, confirmTarget.backup)}
                disabled={rollbackBusy || commandBlocked("strategy_rollback")}
                className="rounded-md bg-red-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-red-700 disabled:opacity-50"
              >
                {rollbackBusy ? t("effectiveConfig.rollbackWorking") : t("effectiveConfig.confirmOk")}
                {rollbackBusy ? <Loader2 size={13} className="ml-1.5 inline animate-spin" /> : null}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
