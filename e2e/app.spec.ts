import { test, expect } from "@playwright/test";

test("app renders the topology view and nav switches to settings", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("EgressAPIKEY").first()).toBeVisible();
  // Nav contains the topology link
  await expect(page.getByRole("button", { name: /Topology/i })).toBeVisible();
  await page.getByRole("button", { name: /Settings/i }).click();
  // Settings view renders its network save control (the old "1..50" lanes
  // hint was removed with laneCount in 7605f685)
  await expect(page.getByTestId("net-save-btn")).toBeVisible();
});
