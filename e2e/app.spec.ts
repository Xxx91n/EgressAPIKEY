import { test, expect } from "@playwright/test";

test("app renders the topology view and nav switches to settings", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("AI API Route").first()).toBeVisible();
  // Nav contains the topology link
  await expect(page.getByRole("button", { name: /Topology/i })).toBeVisible();
  await page.getByRole("button", { name: /Settings/i }).click();
  // Settings view renders the lanes input (1..50 hint)
  await expect(page.getByText("1..50")).toBeVisible();
});
