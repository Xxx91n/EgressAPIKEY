import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ConvergeTooltip } from "./ConvergeTooltip";
import enCommon from "../locales/en/common.json";

// the four-field hover summary.
// Uses React-state-driven open/close (NOT CSS :hover) so tests can drive it
// deterministically via fireEvent.mouseEnter / mouseLeave / focus / blur.
describe("ConvergeTooltip (ticket 06)", () => {
  it("starts closed and opens on mouseEnter with the four-field summary", () => {
    render(
      <ConvergeTooltip
        convergePhase="PendingApply"
        generation={3}
        appliedGeneration={2}
        lastApplyAt={1700000000}
        lastApplyError="PATCH failed"
      >
        <button>trigger</button>
      </ConvergeTooltip>,
    );
    expect(screen.queryByTestId("converge-tooltip-popover")).toBeNull();
    fireEvent.mouseEnter(screen.getByText("trigger"));
    const popover = screen.getByTestId("converge-tooltip-popover");
    expect(popover).toBeInTheDocument();
    expect(screen.getByTestId("converge-tooltip-phase")).toHaveTextContent(/PendingApply/);
    expect(screen.getByTestId("converge-tooltip-expected-rev")).toHaveTextContent(
      enCommon.topConverge.tooltip.expectedRev.replace("{{rev}}", "3"),
    );
    expect(screen.getByTestId("converge-tooltip-applied-rev")).toHaveTextContent(
      enCommon.topConverge.tooltip.appliedRev.replace("{{rev}}", "2"),
    );
    expect(screen.getByTestId("converge-tooltip-applied-at")).toHaveTextContent(/:/);
  });

  it("closes on mouseLeave", () => {
    render(
      <ConvergeTooltip convergePhase="Converged" generation={1} appliedGeneration={1} lastApplyAt={1700000000}>
        <button>trigger</button>
      </ConvergeTooltip>,
    );
    const trigger = screen.getByText("trigger");
    fireEvent.mouseEnter(trigger);
    expect(screen.queryByTestId("converge-tooltip-popover")).not.toBeNull();
    fireEvent.mouseLeave(trigger);
    expect(screen.queryByTestId("converge-tooltip-popover")).toBeNull();
  });

  it("opens on focus and closes on blur (keyboard a11y)", () => {
    render(
      <ConvergeTooltip convergePhase="Drifted" generation={4} appliedGeneration={4} lastApplyAt={1700000000}>
        <button>trigger</button>
      </ConvergeTooltip>,
    );
    const trigger = screen.getByText("trigger");
    fireEvent.focus(trigger);
    expect(screen.queryByTestId("converge-tooltip-popover")).not.toBeNull();
    fireEvent.blur(trigger, { relatedTarget: document.body });
    expect(screen.queryByTestId("converge-tooltip-popover")).toBeNull();
  });

  it("ApplyFailed renders the recorded reason inside the popover", () => {
    render(
      <ConvergeTooltip
        convergePhase="ApplyFailed"
        generation={3}
        appliedGeneration={2}
        lastApplyError="PATCH failed: 500"
        lastApplyAt={1700000000}
      >
        <button>trigger</button>
      </ConvergeTooltip>,
    );
    fireEvent.mouseEnter(screen.getByText("trigger"));
    const err = screen.getByTestId("converge-tooltip-error");
    expect(err).toHaveTextContent(
      enCommon.topConverge.tooltip.error.replace("{{reason}}", "PATCH failed: 500"),
    );
  });

  it("NeverApplied renders the never-applied hint and omits appliedRev/appliedAt lines", () => {
    render(
      <ConvergeTooltip convergePhase="NeverApplied" generation={0} appliedGeneration={0}>
        <button>trigger</button>
      </ConvergeTooltip>,
    );
    fireEvent.mouseEnter(screen.getByText("trigger"));
    expect(screen.getByTestId("converge-tooltip-never-applied")).toHaveTextContent(
      enCommon.topConverge.tooltip.neverApplied,
    );
    expect(screen.queryByTestId("converge-tooltip-applied-rev")).toBeNull();
    expect(screen.queryByTestId("converge-tooltip-applied-at")).toBeNull();
  });

  it("closes on outside click (document mousedown outside the wrapper)", () => {
    render(
      <div>
        <span data-testid="outside">outside</span>
        <ConvergeTooltip convergePhase="Converged" generation={1} appliedGeneration={1} lastApplyAt={1700000000}>
          <button>trigger</button>
        </ConvergeTooltip>
      </div>,
    );
    fireEvent.mouseEnter(screen.getByText("trigger"));
    expect(screen.queryByTestId("converge-tooltip-popover")).not.toBeNull();
    fireEvent.mouseDown(screen.getByTestId("outside"));
    expect(screen.queryByTestId("converge-tooltip-popover")).toBeNull();
  });

  it("popover has role=tooltip and shows the click hint footer", () => {
    render(
      <ConvergeTooltip convergePhase="Converged" generation={1} appliedGeneration={1} lastApplyAt={1700000000}>
        <button>trigger</button>
      </ConvergeTooltip>,
    );
    fireEvent.mouseEnter(screen.getByText("trigger"));
    const popover = screen.getByTestId("converge-tooltip-popover");
    expect(popover).toHaveAttribute("role", "tooltip");
    expect(popover).toHaveTextContent(enCommon.topConverge.tooltip.clickHint);
  });

  // invariant: child click handlers still fire through the wrapper.
  it("children's onClick fires (the wrapper does not swallow clicks)", () => {
    const onClick = vi.fn();
    render(
      <ConvergeTooltip convergePhase="ApplyFailed" generation={1} appliedGeneration={0} lastApplyError="x">
        <button onClick={onClick}>trigger</button>
      </ConvergeTooltip>,
    );
    fireEvent.click(screen.getByText("trigger"));
    expect(onClick).toHaveBeenCalledTimes(1);
  });
});