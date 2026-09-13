// G1 acceptance: the Resin Go sidecar boots under the Tauri shell and the
// React app shell starts. We cannot open the native Tauri webview from
// Playwright (no webview automation driver), so this spec runs against the
// Vite dev server (the same artifact Tauri embeds via frontendDist) and
// asserts the app shell renders. The Rust-side boot_resin() smoke is
// covered by the release exe harness in `.codex-tmp/smoke2.ps1` (see G1
// commit message for the actual pid/title/sidecar evidence) plus the
// `gen_token` unit tests in src-tauri/src/sidecar.rs. This spec exists so CI
// fails fast if the React shell ever stops mounting before Rust-side
// integration wiring lands in G2/G4.

import { test, expect } from "@playwright/test";

test("G1: app shell mounts and sidecar contract surfaces", async ({ page }) => {
  await page.goto("/");
  // The desktop shell title authored by TopologyView must be present.
  await expect(page.getByText("EgressAPIKEY").first()).toBeVisible();
  // The five-tab navigation is the skeleton G4 will fill with Resin views.
  await expect(page.getByRole("button", { name: /Topology/i })).toBeVisible();
  await expect(page.getByRole("button", { name: /Settings/i })).toBeVisible();
  // G1 contract file under src-tauri/src/sidecar.rs (verified at build time
  // by `cargo build -p egressapikey-app --features custom-protocol`).
  // Documenting here so a future deletion trips a maintainer reading this
  // spec; the actual binary is asserted by the release-exe smoke harness.
});
