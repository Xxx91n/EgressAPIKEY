import { useTranslation } from "react-i18next";
import { ReactFlow, Background, BackgroundVariant, Controls, MiniMap } from "@xyflow/react";
import { useMemo } from "react";
import "@xyflow/react/dist/style.css";
import { useAppStore, LaneState } from "../store/appStore";
import type { ColorMode } from "@xyflow/react";
import i18n from "../i18n";

/// Build reactflow nodes for the live lane state. Text is resolved against the
/// i18n singleton so labels follow the current locale; the view header keeps
/// using the hook-based t() so React re-renders on locale change.
function buildNodes(lanes: LaneState[]) {
  return lanes.map((lane, i) => {
    const col = Math.floor(i / 5);
    const row = i % 5;
    const status = lane.busy ? "topology.busy" : "topology.free";
    const label = [
      i18n.t("topology.lane", { index: lane.index }),
      i18n.t(status),
      lane.exitIp ? i18n.t("topology.ip", { ip: lane.exitIp }) : "",
    ]
      .filter(Boolean)
      .join("\n");
    return {
      id: `lane-${lane.index}`,
      type: "default",
      position: { x: 80 + col * 220, y: 60 + row * 110 },
      data: { label },
    };
  });
}

export function TopologyView() {
  const { t } = useTranslation();
  const lanes = useAppStore((s) => s.lanes);
  const theme = useAppStore((s) => s.theme);
  // ReactFlow 12 built-in colorMode: light/dark/system map 1:1 to our Theme.
  const colorMode: ColorMode = theme;
  const nodes = useMemo(() => buildNodes(lanes), [lanes]);
  const edges = useMemo(
    () =>
      lanes.slice(0, -1).map((lane, i) => ({
        id: `e-${i}`,
        source: `lane-${lanes[i].index}`,
        target: `lane-${lanes[i + 1].index}`,
        animated: lane.busy,
      })),
    [lanes]
  );
  return (
    <section className="h-full flex flex-col">
      <header className="mb-2">
        <h2 className="font-medium">{t("topology.title")}</h2>
        <p className="text-xs text-zinc-500 dark:text-zinc-400">{t("topology.live")}</p>
      </header>
      <div className="flex-1 border border-zinc-200 dark:border-zinc-800 rounded">
        <ReactFlow nodes={nodes} edges={edges} fitView colorMode={colorMode}>
          <Background variant={BackgroundVariant.Dots} gap={18} size={1.4} />
          <Controls />
          <MiniMap pannable zoomable />
        </ReactFlow>
      </div>
    </section>
  );
}
