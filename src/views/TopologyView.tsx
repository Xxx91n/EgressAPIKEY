import { useTranslation } from "react-i18next";
import { ReactFlow, Background, BackgroundVariant, Controls, MiniMap } from "@xyflow/react";
import { useEffect, useMemo, useState } from "react";
import "@xyflow/react/dist/style.css";
import { useAppStore, LaneState } from "../store/appStore";
import { ipcGatewaySnapshot } from "../lib/ipc";
import { listen } from "@tauri-apps/api/event";
import type { ColorMode } from "@xyflow/react";
import { AlertTriangle } from "lucide-react";

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
  const setLanes = useAppStore((s) => s.setLanes);
  const theme = useAppStore((s) => s.theme);
  // #6 stop-toy: pull the REAL lane topology from the Resin gateway over IPC
  // instead of showing 2 hardcoded fake lanes forever. The snapshot is coarse
  // (lane_count + total busy + per-authority TD-EWMA latencies) so we resync
  // the lane node COUNT to the real configured lane_count, keep per-lane
  // busy as-is (snapshot has no per-lane busy today), and show the live
  // busy/total in a status strip. Outside Tauri (vitest) ipc throws and we
  // keep the existing laneCount-derived lanes. Polls every 5s.
  const [busyTotal, setBusyTotal] = useState<number | null>(null);
  const [laneTotal, setLaneTotal] = useState<number | null>(null);
  // G3 contract: listen for sidecar-status poll events from the Rust shell;
  // when the Ghost safety-net trips (3 consecutive /healthz failures), it
  // emits "unhealthy" and we raise a red warning banner above the canvas.
  // Outside Tauri, listen() rejects gracefully and we never set a banner.
  const [sidecarStatus, setSidecarStatus] = useState<"healthy" | "unhealthy" | null>(null);
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<string>("sidecar-status", (e) => {
      setSidecarStatus(e.payload as "healthy" | "unhealthy");
    }).then((fn) => { unlisten = fn; }).catch(() => { /* outside Tauri */ });
    return () => { if (unlisten) unlisten(); };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const sync = async () => {
      try {
        const snap = await ipcGatewaySnapshot();
        if (cancelled) return;
        setLaneTotal(snap.lane_count);
        setBusyTotal(snap.busy);
        // Rebuild lane nodes to match the real lane_count (gap-close / cap at 50).
        const count = Math.max(1, Math.min(50, snap.lane_count));
        setLanes(
          Array.from({ length: count }, (_, i) => ({
            index: i,
            exitIp: null,
            busy: false,
            account: null,
            authority: null,
          })),
        );
      } catch {
        // outside Tauri or registry not wired yet — keep local lane state
      }
    };
    void sync();
    const id = setInterval(() => void sync(), 5000);
    return () => { cancelled = true; clearInterval(id); };
  }, [setLanes]);

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
      {sidecarStatus === "unhealthy" && (
        <div className="mb-2 flex items-center gap-2 rounded-md border border-red-300 dark:border-red-800 bg-red-50 dark:bg-red-950/40 px-3 py-2 text-xs text-red-700 dark:text-red-300">
          <AlertTriangle size={14} className="shrink-0" />
          <span>{t("topology.sidecarUnhealthy", { defaultValue: "Sidecar unsafe: Resin proxy offline. System proxy cleared." })}</span>
        </div>
      )}
      <div className="flex items-center justify-between px-1 pb-2">
        <span className="text-xs text-zinc-500 dark:text-zinc-400">{t("topology.live")}</span>
        {laneTotal !== null && busyTotal !== null ? (
          <span className="text-xs font-mono text-zinc-500 dark:text-zinc-400">
            {t("topology.status", { lanes: laneTotal, busy: busyTotal })}
          </span>
        ) : null}
      </div>
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
