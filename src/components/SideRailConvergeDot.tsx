import * as React from "react";
import { useTranslation } from "react-i18next";
import type { ConvergePhase } from "../lib/ipc";

/**
 * Architecture-recovery ticket 06 (D-C2.1): a single 4px status dot inside
 * the SideRail so the user can spot convergence drift/failure from any view
 * without leaving the nav layer.
 *
 * Checkpoint C (D-C2.1) — Converged state uses literal `display: none`
 * (NOT opacity), because a converged-but-present dot is noise the user does
 * not want to see. The element remains in the DOM so tests can assert the
 * hide and so the same mounted instance survives phase transitions.
 *
 * ArgoCD #22059 — this dot renders the Sync axis (top-level ConvergePhase)
 * ONLY. Per-entry Health badges live inside EffectiveConfigView.
 */
export interface SideRailConvergeDotProps {
  convergePhase: ConvergePhase;
  lastApplyError?: string;
}

const DOT_CLASS: Record<ConvergePhase, string> = {
  Converged: "bg-green-500",
  PendingApply: "bg-amber-500",
  ApplyFailed: "bg-red-500",
  NeverApplied: "bg-amber-500",
  Drifted: "bg-amber-500",
  Unknown: "bg-zinc-400",
};

const DOT_LABEL_KEY: Record<ConvergePhase, string | null> = {
  Converged: null, // hidden per checkpoint C
  PendingApply: "topConverge.sideRail.PendingApply",
  ApplyFailed: "topConverge.sideRail.ApplyFailed",
  NeverApplied: "topConverge.sideRail.NeverApplied",
  Drifted: "topConverge.sideRail.Drifted",
  Unknown: "topConverge.sideRail.Unknown",
};

export function SideRailConvergeDot(props: SideRailConvergeDotProps) {
  const { t } = useTranslation();
  const { convergePhase, lastApplyError } = props;
  const labelKey = DOT_LABEL_KEY[convergePhase];
  const ariaLabel = labelKey
    ? t(labelKey, { error: lastApplyError ?? "" })
    : undefined;
  return (
    <span
      data-testid="side-rail-converge-dot"
      data-phase={convergePhase}
      role="status"
      aria-label={ariaLabel}
      // display:none (not opacity) per checkpoint C — see file header.
      style={{ display: convergePhase === "Converged" ? "none" : undefined }}
      className={
        "inline-block w-1 h-1 rounded-full " +
        (DOT_CLASS[convergePhase] ?? DOT_CLASS.Unknown)
      }
    />
  );
}