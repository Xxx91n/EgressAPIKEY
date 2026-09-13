import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { SideRailConvergeDot } from "./SideRailConvergeDot";
import enCommon from "../locales/en/common.json";

// (checkpoint C): a 4px status dot
// in the SideRail. ArgoCD #22059 — Sync axis only; per-entry Health lives
// inside EffectiveConfigView and is NOT merged here.
//
// Checkpoint C: Converged state MUST hide via literal `display: none`
// (NOT opacity); the element stays in the DOM so tests can assert the hide
// and so a phase transition animates the same mounted instance.
describe("SideRailConvergeDot (ticket 06)", () => {
  const PHASES = ["Converged", "PendingApply", "ApplyFailed", "NeverApplied", "Drifted", "Unknown"] as const;

  it.each(PHASES)("renders the dot with data-phase=%s", (phase) => {
    render(<SideRailConvergeDot convergePhase={phase} />);
    const dot = screen.getByTestId("side-rail-converge-dot");
    expect(dot).toHaveAttribute("data-phase", phase);
    expect(dot).toHaveAttribute("role", "status");
  });

  it("Converged uses literal display: none (checkpoint C: not opacity)", () => {
    render(<SideRailConvergeDot convergePhase="Converged" />);
    const dot = screen.getByTestId("side-rail-converge-dot");
    expect(dot).toHaveStyle({ display: "none" });
    // Sanity: still mounted in the DOM so phase transitions animate in place.
    expect(dot).toBeInTheDocument();
  });

  it("non-Converged phases do not have display: none", () => {
    const visiblePhases = ["PendingApply", "ApplyFailed", "NeverApplied", "Drifted", "Unknown"] as const;
    for (const phase of visiblePhases) {
      const { unmount } = render(<SideRailConvergeDot convergePhase={phase} />);
      const dot = screen.getByTestId("side-rail-converge-dot");
      expect(dot.style.display).not.toBe("none");
      unmount();
    }
  });

  it("ApplyFailed renders the red colour token", () => {
    render(<SideRailConvergeDot convergePhase="ApplyFailed" lastApplyError="PATCH failed" />);
    const dot = screen.getByTestId("side-rail-converge-dot");
    expect(dot.className).toMatch(/bg-red-500/);
  });

  it("PendingApply / NeverApplied / Drifted render the amber colour token", () => {
    for (const phase of ["PendingApply", "NeverApplied", "Drifted"] as const) {
      const { unmount } = render(<SideRailConvergeDot convergePhase={phase} />);
      const dot = screen.getByTestId("side-rail-converge-dot");
      expect(dot.className).toMatch(/bg-amber-500/);
      unmount();
    }
  });

  it("Unknown renders the grey colour token", () => {
    render(<SideRailConvergeDot convergePhase="Unknown" />);
    const dot = screen.getByTestId("side-rail-converge-dot");
    expect(dot.className).toMatch(/bg-zinc-400/);
  });

  it("aria-label is the per-phase translated label (Converged has no aria — the dot is hidden)", () => {
    for (const phase of ["PendingApply", "ApplyFailed", "NeverApplied", "Drifted", "Unknown"] as const) {
      const { unmount } = render(<SideRailConvergeDot convergePhase={phase} />);
      const dot = screen.getByTestId("side-rail-converge-dot");
      const expected = (enCommon.topConverge.sideRail as Record<string, string>)[phase];
      expect(dot).toHaveAttribute("aria-label", expected);
      unmount();
    }
    render(<SideRailConvergeDot convergePhase="Converged" />);
    expect(screen.getByTestId("side-rail-converge-dot")).not.toHaveAttribute("aria-label");
  });
});