import * as React from "react";
import { useState, useRef, useEffect } from "react";
import { useTranslation } from "react-i18next";
import type { ConvergePhase } from "../lib/ipc";

/**
* a controlled popover that
 * surfaces the 4-field convergence summary on hover or keyboard focus.
 *
 * Renders children inline, then an absolutely-positioned popover below.
 * Uses React state (not CSS :hover) so tests can drive the tooltip
 * deterministically via fireEvent.mouseEnter / focus.
 */
export interface ConvergeTooltipProps {
  convergePhase: ConvergePhase;
  generation: number;
  appliedGeneration: number;
  lastApplyAt?: number;
  lastApplyError?: string;
  children: React.ReactNode;
  /** Optional override for the popover placement (default: below, right-aligned). */
  placement?: "below" | "above";
}

function formatTime(unixSeconds?: number): string {
  if (!unixSeconds || !Number.isFinite(unixSeconds) || unixSeconds <= 0) return "—";
  return new Date(unixSeconds * 1000).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function ConvergeTooltip(props: ConvergeTooltipProps) {
  const { t } = useTranslation();
  const {
    convergePhase,
    generation,
    appliedGeneration,
    lastApplyAt,
    lastApplyError,
    children,
    placement = "below",
  } = props;
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLSpanElement | null>(null);

  // Close on outside click — keeps the popover honest if the user wanders.
  useEffect(() => {
    if (!open) return;
    const onDocClick = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, [open]);

  const popoverClass =
    "absolute z-50 min-w-[18rem] max-w-[24rem] rounded-md border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 text-xs text-zinc-700 dark:text-zinc-200 shadow-md p-3 space-y-1 " +
    (placement === "above"
      ? "bottom-full right-0 mb-1"
      : "top-full right-0 mt-1");

  return (
    <span
      ref={wrapRef}
      className="relative inline-flex"
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => setOpen(false)}
      onFocus={() => setOpen(true)}
      onBlur={(e) => {
        // Only close if focus left the wrapper entirely.
        if (!wrapRef.current?.contains(e.relatedTarget as Node | null)) {
          setOpen(false);
        }
      }}
    >
      {children}
      {open && (
        <div
          role="tooltip"
          data-testid="converge-tooltip-popover"
          className={popoverClass}
        >
          <div className="font-medium text-zinc-900 dark:text-zinc-100" data-testid="converge-tooltip-phase">
            {t("topConverge.tooltip.header")} · {convergePhase}
          </div>
          <div data-testid="converge-tooltip-expected-rev">
            {t("topConverge.tooltip.expectedRev", { rev: generation })}
          </div>
          {convergePhase === "NeverApplied" ? (
            <div data-testid="converge-tooltip-never-applied" className="italic text-zinc-500 dark:text-zinc-400">
              {t("topConverge.tooltip.neverApplied")}
            </div>
          ) : (
            <>
              <div data-testid="converge-tooltip-applied-rev">
                {t("topConverge.tooltip.appliedRev", { rev: appliedGeneration })}
              </div>
              <div data-testid="converge-tooltip-applied-at">
                {t("topConverge.tooltip.appliedAt", {
                  time: formatTime(lastApplyAt),
                })}
              </div>
              {convergePhase === "ApplyFailed" && lastApplyError ? (
                <div
                  data-testid="converge-tooltip-error"
                  className="text-red-600 dark:text-red-400 break-words"
                >
                  {t("topConverge.tooltip.error", { reason: lastApplyError })}
                </div>
              ) : null}
            </>
          )}
          <div className="pt-1 border-t border-zinc-100 dark:border-zinc-800 text-zinc-500 dark:text-zinc-400">
            {t("topConverge.tooltip.clickHint")}
          </div>
        </div>
      )}
    </span>
  );
}