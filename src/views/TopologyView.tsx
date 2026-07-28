import { useTranslation } from "react-i18next";
import { ReactFlow, Background, BackgroundVariant, Controls, MiniMap } from "@xyflow/react";
import { useMemo } from "react";
import "@xyflow/react/dist/style.css";
import { useAppStore, LaneState } from "../store/appStore";
import type { ColorMode } from "@xyflow/react";

/// Build reactflow nodes for the live lane state. `t` comes from
/// useTranslation()'s bound t, which is guaranteed to be the locale-resolved
/// translator for the CURRENT language (not the i18next singleton, which can
/// momentarily return the fallback en string before the lazy locale chunk
/// finishes loading — that was the #6 bug: canvas nodes showed English on the
/// first mount, then flipped to zh only after navigating away and back).
function buildNodes(lanes: LaneState[], t: ReturnType<typeof useTranslation>["t"]) {
  return lanes.map((lane, i) => {
    const col = Math.floor(i / 5);
    const row = i % 5;
    const status = lane.busy ? "topology.busy" : "topology.free";
    const label = [
      t("topology.lane", { index: lane.index }),
      t(status),
      lane.exitIp ? t("topology.ip", { ip: lane.exitIp }) : "",
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
  const { t, i18n } = useTranslation();
  const lanes = useAppStore((s) => s.lanes);
  const theme = useAppStore((s) => s.theme);
  // ReactFlow 12 built-in colorMode: light/dark/system map 1:1 to our Theme.
  const colorMode: ColorMode = theme;
  // i18n.language is a dep so the nodes rebuild when the lazy locale chunk
  // finishes loading (the translator `t` then resolves to the new locale).
  // Without it the canvas kept stale English labels until the view remounted.
  const nodes = useMemo(() => buildNodes(lanes, t), [lanes, t, i18n.language]);
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
