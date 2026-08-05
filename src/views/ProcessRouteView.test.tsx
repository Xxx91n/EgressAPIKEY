import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor, cleanup } from "@testing-library/react";
import { invokeMock } from "../test/setup";
import { ProcessRouteView } from "./ProcessRouteView";
import { useAppStore } from "../store/appStore";

afterEach(() => cleanup());

describe("ProcessRouteView (closed-loop, IPC-mocked)", () => {
  beforeEach(() => {
    useAppStore.setState({ processRoutes: [], laneCount: 10 });
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "process_route_list") return [];
      return undefined;
    });
  });

  it("renders empty rules + add-rule form", async () => {
    render(<ProcessRouteView />);
    // The process-name input has a stable placeholder from t("processRoute.process")
    await waitFor(() =>
      expect(screen.getByPlaceholderText(/Process name|\u8fdb\u7a0b\u540d/i)).toBeInTheDocument()
    );
  });

  it("adds a rule: dispatches process_route_add with process + targetPort", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "process_route_add") return undefined;
      if (cmd === "process_route_list") return [{ process: "ollama", target_port: 17990 }];
      return undefined;
    });

    render(<ProcessRouteView />);
    const procInput = await screen.findByPlaceholderText(/Process name|\u8fdb\u7a0b\u540d/i);
    fireEvent.change(procInput, { target: { value: "ollama" } });
    const targetInput = screen.getByRole("spinbutton") as HTMLInputElement;
    fireEvent.change(targetInput, { target: { value: "17990" } });
    const addBtn = screen.getByRole("button", { name: /add|processRoute\.add|\u6dfb\u52a0/i });
    fireEvent.click(addBtn);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("process_route_add", { process: "ollama", targetPort: 17990 });
    });
    // After refresh-from-backend the rule shows up in the list.
    await waitFor(() => expect(screen.getByText("ollama")).toBeInTheDocument(), { timeout: 3000 });
  });

  it("surfaces a conflict toast when backend rejects the port", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "process_route_add")
        throw new Error("conflict: port 17990 already bound to process 'ollama'");
      if (cmd === "process_route_list") return [];
      return undefined;
    });
    render(<ProcessRouteView />);
    const procInput = await screen.findByPlaceholderText(/Process name|\u8fdb\u7a0b\u540d/i);
    fireEvent.change(procInput, { target: { value: "newproc" } });
    const targetInput = screen.getByRole("spinbutton") as HTMLInputElement;
    fireEvent.change(targetInput, { target: { value: "3" } });
    fireEvent.click(screen.getByRole("button", { name: /add|\u6dfb\u52a0/i }));
    await waitFor(() => expect(screen.getByText(/conflict|Lane .* bound/i)).toBeInTheDocument(), { timeout: 2000 });
  });
});