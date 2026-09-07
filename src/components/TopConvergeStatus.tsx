import * as React from "react";
import { useTranslation } from "react-i18next";
import type { ConvergePhase } from "../lib/ipc";
import { ConvergeTooltip } from "./ConvergeTooltip";

/**
 * Architecture-recovery ticket 06 (D-C2.1 / D-C2.4 / ArgoCD #22059 警戒):
 * a compact pill in the App header that surfaces the top-level convergence
 * phase. Data flows from a single authoritative snapshot owned by App.tsx;
 * this component is pure presentational and never reaches for IPC itself.
 *
 * ArgoCD #22059 — Sync (ConvergePhase, here) is intentionally SEPARATE from
 * Health (per-entry drift badges inside EffectiveConfigView). We do NOT
 * merge the two axes into one chip; this component renders the Sync axis only.
 *
 * Hover/focus surfaces ConvergeTooltip with the 4-field summary; click
 * navigates to the EffectiveConfigView (route contract from the issue).
 */
export interface TopConvergeStatusProps {
  convergePhase: ConvergePhase;
  generation: number;
  appliedGeneration: number;
  lastApplyAt?: number;
  lastApplyError?: string;
  /** Set by App.tsx to navigate to the EffectiveConfigView; required. */
  onOpenDetail: () => void;
}

const COMPACT_CLASS: Record<ConvergePhase, string> = {
  Converged: "bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-400",
  PendingApply: "bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-400",
  ApplyFailed: "bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-400",
  NeverApplied: "bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-400",
  Drifted: "bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-400",
  Unknown: "bg-zinc-200 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-400",
};

const COMPACT_REV_KEY: Record<ConvergePhase, string> = {
  Converged: "topConverge.compact.Converged",
  PendingApply: "topConverge.compact.PendingApply",
  ApplyFailed: "topConverge.compact.ApplyFailed",
  NeverApplied: "topConverge.compact.NeverApplied",
  Drifted: "topConverge.compact.Drifted",
  Unknown: "topConverge.compact.Unknown",
};

export function TopConvergeStatus(props: TopConvergeStatusProps) {
  const { t } = useTranslation();
  const { convergePhase, generation, appliedGeneration, lastApplyAt, lastApplyError, onOpenDetail } = props;
  const pillBase = "inline-flex items-center rounded px-1.5 py-0.5 text-[11px] font-medium transition-colors hover:opacity-90 cursor-pointer focus:outline-none focus:ring-2 focus:ring-zinc-400 dark:focus:ring-zinc-500";
  const compactClass = COMPACT_CLASS[convergePhase] ?? COMPACT_CLASS.Unknown;
  const compactLabel = t(COMPACT_REV_KEY[convergePhase], {
    rev: convergePhase === "Converged" ? appliedGeneration : generation,
  });
  return (
    <ConvergeTooltip
      convergePhase={convergePhase}
      generation={generation}
      appliedGeneration={appliedGeneration}
      lastApplyAt={lastApplyAt}
      lastApplyError={lastApplyError}
    >
      <button
        type="button"
        data-testid="top-converge-status"
        data-phase={convergePhase}
        onClick={onOpenDetail}
        className={pillBase + " " + compactClass}
      >
        {compactLabel}
      </button>
    </ConvergeTooltip>
  );
}