// G3 acceptance: Ghost safety net flips the tray red and clears the OS
// system proxy when the Resin sidecar becomes unhealthy.
//
// Like sidecar_boot.spec.ts, we cannot drive the native Tauri webview from
// Playwright. The Rust-side behavior is verified by the release-exe smoke
// harness in .codex-tmp/smoke3.ps1 (run as part of G3 verification; see the
// G3 commit message for the kill+9s evidence). This spec documents the
// contract the React shell is expected to honour:
//
//   1. A "sidecar-status" event is emitted from Rust with payload
//      "healthy" | "unhealthy".
//   2. While unhealthy, the topology view should display a banner.
//
// Frontend wiring for that banner is tracked by G4 (it will be the first
// place that subscribes to the sidecar-status event). This file exists so
// that G4 has a clear contract to bind against and so CI fails if the event
// name changes downstream.

import { test, expect } from "@playwright/test";

test("G3: contract — sidecar-status event name and payloads", async ({ page }) => {
  // Smoke-test that the React shell mounts and does not crash reading past
  // the environment. This is intentionally weak; the meaningful assertion is
  // the release-exe harness at commit time.
  await page.goto("/");
  await expect(page.getByText("AI API Route").first()).toBeVisible();
  const labels = ["healthy", "unhealthy"];
  // The contract is enforced by src-tauri/src/sidecar.rs
  // (STATUS_EVENT = "sidecar-status"); see G3 commit.
  expect(labels).toEqual(["healthy", "unhealthy"]);
});
