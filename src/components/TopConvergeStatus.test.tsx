import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { TopConvergeStatus } from "./TopConvergeStatus";
import enCommon from "../locales/en/common.json";

// (ArgoCD #22059 警戒).
// This component is pure presentational; no IPC mocks needed. The
// pins the 6-state colour palette + compact label mapping; the test must
// lock the wiring for every phase.
describe("TopConvergeStatus (ticket 06)", () => {
  const PHASES = ["Converged", "PendingApply", "ApplyFailed", "NeverApplied", "Drifted", "Unknown"] as const;

  it.each(PHASES)("renders the pill with data-phase=%s and the matching compact label", (phase) => {
    render(
      <TopConvergeStatus
        convergePhase={phase}
        generation={3}
        appliedGeneration={2}
        lastApplyAt={1700000000}
        lastApplyError="boom"
        onOpenDetail={() => {}}
      />,
    );
    const pill = screen.getByTestId("top-converge-status");
    expect(pill).toHaveAttribute("data-phase", phase);
    expect(pill.tagName).toBe("BUTTON");
    // Compact label is the enCommon value with {{rev}} interpolated. For
    // Converged we render appliedGeneration; for every other phase the
    // desired generation (mirrors the EffectiveConfigView ConvergeChip
    // contract — observed vs desired are two distinct axes).
    const expected = (enCommon.topConverge.compact as Record<string, string>)[phase].replace(
      "{{rev}}",
      phase === "Converged" ? "2" : "3",
    );
    expect(pill).toHaveTextContent(expected);
  });

  it("clicking the pill invokes onOpenDetail (route contract from the issue)", () => {
    const onOpen = vi.fn();
    render(
      <TopConvergeStatus
        convergePhase="ApplyFailed"
        generation={5}
        appliedGeneration={4}
        lastApplyError="PATCH failed: 500"
        onOpenDetail={onOpen}
      />,
    );
    fireEvent.click(screen.getByTestId("top-converge-status"));
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("ApplyFailed pill carries the red colour token (parity with ConvergeChip)", () => {
    render(
      <TopConvergeStatus
        convergePhase="ApplyFailed"
        generation={1}
        appliedGeneration={0}
        lastApplyError="x"
        onOpenDetail={() => {}}
      />,
    );
    const pill = screen.getByTestId("top-converge-status");
    expect(pill.className).toMatch(/bg-red-100/);
    expect(pill.className).toMatch(/text-red-700/);
  });

  it("Converged pill carries the green colour token (kept visible — only the SideRailConvergeDot hides it per checkpoint C)", () => {
    render(
      <TopConvergeStatus
        convergePhase="Converged"
        generation={1}
        appliedGeneration={1}
        lastApplyAt={1700000000}
        onOpenDetail={() => {}}
      />,
    );
    const pill = screen.getByTestId("top-converge-status");
    expect(pill.className).toMatch(/bg-green-100/);
    expect(pill.className).toMatch(/text-green-700/);
  });
});