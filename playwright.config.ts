import { defineConfig, devices } from "@playwright/test";

/**
 * Playwright e2e config. The dev server (vite) runs on port 1420. The Tauri
 * desktop build is exercised separately by tauri-action in CI; these tests
 * target the web frontend surface route, mirroring the React flow.
 */
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  use: {
    baseURL: "http://localhost:1420",
    trace: "on-first-retry",
  },
  webServer: {
    command: "pnpm dev",
    url: "http://localhost:1420",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
  ],
});
