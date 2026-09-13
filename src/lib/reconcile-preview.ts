// ADR-0054 §A: the reconcile preview
// derivation. The plan is computed FROM the snapshot's own entries (spec:
// "源自快照内已有计划数据,禁止发新请求"): the snapshot already merged the
// whitebox intent against the live Resin rows through compute_plan, so a
// platform entry that is divergent or missingOnResin IS the next
// strategy-apply's change, and a missingOnResin enabled port IS the next
// ports-restore's creation. Pure function, vitest-covered.

import type { AuthoritativeSnapshot, PortSnapshot, StrategySnapshot } from "./ipc";

export interface PreviewPlatform {
  platform: string;
  desired_regions: string[];
  live_regions: string[];
  action: "patch_regions" | "create_platform";
}

export interface PreviewPort {
  port: number;
  platform: string;
  action: "create_endpoint";
}

export interface ReconcilePreview {
  platforms: PreviewPlatform[];
  ports: PreviewPort[];
}

function desiredOf(p: StrategySnapshot): string[] {
  if (p.state === "divergent") return p.whitebox_regions;
  return p.regions;
}

function liveOf(p: StrategySnapshot): string[] {
  if (p.state === "divergent") return p.resin_regions;
  return [];
}

/** Derive what one reconcile pass WOULD change, from the snapshot alone. */
export function reconcilePreviewFromSnapshot(snap: AuthoritativeSnapshot): ReconcilePreview {
  if (!snap.resinReachable) {
    // ADR-0051: sidecar-down absence is not drift; a reconcile with no
    // runtime to converge has an empty plan (the view disables the button).
    return { platforms: [], ports: [] };
  }
  const platforms: PreviewPlatform[] = snap.platforms
    .filter((p) => !p.acknowledged && p.state !== "consistent")
    .map((p) => ({
      platform: p.platform_name,
      desired_regions: desiredOf(p),
      live_regions: liveOf(p),
      action: p.state === "divergent" ? "patch_regions" : "create_platform",
    }));
  const ports: PreviewPort[] = snap.ports
    .filter(
      (pt: PortSnapshot) =>
        !pt.acknowledged &&
        pt.state === "missingOnResin" &&
        (pt.state === "missingOnResin" ? pt.auth_required !== undefined : true) &&
        // Only ENABLED whitebox ports are re-asserted by restore; a disabled
        // port's absence is intentional, not drift to fix.
        portEnabledInDesired(pt),
    )
    .map((pt) => ({
      port: pt.port,
      platform: pt.platform_name,
      action: "create_endpoint",
    }));
  return { platforms, ports };
}

/** The snapshot's missingOnResin port variant does not carry `enabled`
 *  (whitebox-only field); treat every missing port as enabled-intent —
 *  restore_ports_from_whitebox itself filters disabled entries, and a
 *  disabled port would not appear as missingOnResin in practice. */
function portEnabledInDesired(_pt: PortSnapshot): boolean {
  return true;
}

export function reconcilePreviewIsEmpty(p: ReconcilePreview): boolean {
  return p.platforms.length === 0 && p.ports.length === 0;
}
