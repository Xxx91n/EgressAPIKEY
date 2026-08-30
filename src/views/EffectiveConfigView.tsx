import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ClipboardCheck, Loader2, RefreshCw } from "lucide-react";
import {
  ipcAuthoritativeSnapshot,
  type AuthoritativeSnapshot,
  type StrategySnapshot,
} from "../lib/ipc";
import { translateError } from "../lib/i18n-error";

// Architecture-recovery ticket 13: one-level "effective config" view
// (CONTEXT.md: Authoritative Snapshot; spec Round 2 Implementation Decision 1/7).
// Read-only consumer of the authoritative_snapshot IPC (ADR-0051): renders
// the pre-merged desired|live comparison per platform/port with the
// three-state badge (consistent / divergent / missingOnResin), a grey
// "known" degradation for acknowledged entities (ADR-0054 D: exemptions
// never enter the three-state merge), divergentSince per entry and
// lastCheckedAt on top. ZERO write paths by contract (actions live in
// tickets 14/15); edits stay in the whitebox editors.

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

  const refresh = useCallback(async () => {
    setBusy(true);
    setError("");
    try {
      setSnap(await ipcAuthoritativeSnapshot());
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

  const lastChecked =
    snap && snap.lastCheckedAt ? new Date(snap.lastCheckedAt * 1000).toLocaleString() : "";

  return (
    <div className="w-full max-w-none px-6 py-4 space-y-4" data-testid="ec-view">
      <div className="flex items-center gap-2 flex-wrap">
        <ClipboardCheck size={16} strokeWidth={1.75} />
        <h2 className="text-sm font-semibold tracking-tight">{t("nav.effectiveConfig")}</h2>
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
      {snap && snap.platforms.length === 0 && snap.ports.length === 0 ? (
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
    </div>
  );
}
