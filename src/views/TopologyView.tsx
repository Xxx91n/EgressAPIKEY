import { useTranslation } from "react-i18next";
import { ReactFlow, Background, BackgroundVariant, Controls, MiniMap } from "@xyflow/react";
import { useEffect, useMemo, useState } from "react";
import "@xyflow/react/dist/style.css";
import { useAppStore, LaneState } from "../store/appStore";
import { ipcGatewaySnapshot, ipcPlatformList } from "../lib/ipc";
import { listen } from "@tauri-apps/api/event";
import type { ColorMode } from "@xyflow/react";
import { AlertTriangle } from "lucide-react";

/// TopologyView renders the lane/lease canvas.
/// Issue 4+7 semantics:
///   - LEFT: an Entry box per Platform carrying the platform name + the
///     API-Key hash that platform is currently using (one Platform per Key,
///     one Key per entry). Lines from entry -> lanes show which lanes that
///     platform's key currently occupies.
///   - RIGHT: one Lane box per lane showing the exit IP through mihomo, the
///     free/busy state, and an SSE-lock badge (a lane with an open SSE stream
///     is locked for the duration of the stream; no other Key may use it).
/// The Resin sidecar owns the actual key-hash->lane->{mihomo node, exit IP}
/// mapping; the shell only MIRRORS the live lease view over IPC.
function buildEntries(platforms: string[], t: ReturnType<typeof useTranslation>["t"]) {
  return platforms.map((p, i) => ({
    id: `entry-${p}`,
    type: "input",
    position: { x: 0, y: 60 + i * 120 },
    data: { label: `${t("topology.entry", { platform: p })}` },
  }));
}

function buildLanes(lanes: LaneState[], t: ReturnType<typeof useTranslation>["t"]) {
  return lanes.map((lane, i) => {
    const col = Math.floor(i / 5);
    const row = i % 5;
    const status = lane.busy ? "topology.busy" : "topology.free";
    const lines: string[] = [t("topology.lane", { index: lane.index }), t(status)];
    if (lane.exitIp) lines.push(t("topology.ip", { ip: lane.exitIp }));
    if (lane.keyHash) lines.push(t("topology.keyHash", { hash: lane.keyHash }));
    if (lane.sseLocked) lines.push(t("topology.sseLocked"));
    return {
      id: `lane-${lane.index}`,
      type: "default",
      position: { x: 320 + col * 220, y: 60 + row * 110 },
      data: { label: lines.filter(Boolean).join("\n") },
    };
  });
}

export function TopologyView() {
  const { t, i18n } = useTranslation();
  const lanes = useAppStore((s) => s.lanes);
  const setLanes = useAppStore((s) => s.setLanes);
  const theme = useAppStore((s) => s.theme);
  const [busyTotal, setBusyTotal] = useState<number | null>(null);
  const [laneTotal, setLaneTotal] = useState<number | null>(null);
  const [platforms, setPlatforms] = useState<string[]>([]);
  const [sidecarStatus, setSidecarStatus] = useState<"healthy" | "unhealthy" | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<string>("sidecar-status", (e) => {
      setSidecarStatus(e.payload as "healthy" | "unhealthy");
    }).then((fn) => { unlisten = fn; }).catch(() => {});
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
        const count = Math.max(1, Math.min(50, snap.lane_count));
        setLanes(
          Array.from({ length: count }, (_, i) => ({
            index: i,
            exitIp: null,
            busy: false,
            account: null,
            authority: null,
            platform: null,
            keyHash: null,
            sseLocked: false,
          })),
        );
      } catch {}
      // Issue 4+7: mirror the Resin platform list so the entry boxes are real
      try {
        const pl = await ipcPlatformList();
        if (cancelled) return;
        setPlatforms(pl);
      } catch {}
    };
    void sync();
    const id = setInterval(() => void sync(), 5000);
    return () => { cancelled = true; clearInterval(id); };
  }, [setLanes]);

  const colorMode: ColorMode = theme;
  const nodes = useMemo(
    () => [...buildEntries(platforms, t), ...buildLanes(lanes, t)],
    [lanes, t, i18n.language, platforms]
  );
  // Edges: every platform entry links to lane index 0..(busy-1) for now (real
  // lane<->key mapping comes from a future ResinClient.active_leases parser).
  const edges = useMemo(() => {
    const list: { id: string; source: string; target: string; animated: boolean }[] = [];
    platforms.forEach((p) => {
      lanes.slice(0, Math.max(0, busyTotal ?? 0)).forEach((l) => {
        list.push({ id: `e-${p}-${l.index}`, source: `entry-${p}`, target: `lane-${l.index}`, animated: l.busy });
      });
    });
    return list;
  }, [platforms, lanes, busyTotal]);

  return (
    <section className="h-full flex flex-col">
      {sidecarStatus === "unhealthy" && (
        <div className="mb-2 flex items-center gap-2 rounded-md border border-red-300 dark:border-red-800 bg-red-50 dark:bg-red-950/40 px-3 py-2 text-xs text-red-700 dark:text-red-300">
          <AlertTriangle size={14} className="shrink-0" />
          <span>{t("topology.sidecarUnhealthy")}</span>
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
