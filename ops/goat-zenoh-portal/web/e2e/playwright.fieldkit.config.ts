// SPDX-License-Identifier: Apache-2.0
//
// Playwright config for Fieldkit tools (ops/fieldkit/), sharing this directory's
// Playwright install so Fieldkit carries no package manifest of its own. Run:
//   npx playwright test -c playwright.fieldkit.config.ts
// Gatepost is served by the local mock of the pod API
// (ops/fieldkit/gatepost/mock_pod_api.py), which answers the same routes and
// shapes as the pod and serves gatepost.html at /. Tests reset the mock's
// synthetic data through its /_mock/reset route, so they run one at a time.
// The portal's own config (playwright.config.ts) does not pick these specs up.

import { defineConfig, devices } from "@playwright/test";

const PORT = Number(process.env.FIELDKIT_PORT) || 8790;
const BASE_URL = `http://127.0.0.1:${PORT}`;

export default defineConfig({
  testDir: "./fieldkit",
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: "list",
  use: { baseURL: BASE_URL, trace: "retain-on-failure" },
  outputDir: "test-results/fieldkit",
  projects: [
    { name: "desktop", use: { ...devices["Desktop Chrome"] } },
    { name: "tablet", use: { ...devices["Desktop Chrome"], viewport: { width: 800, height: 1280 }, hasTouch: true } },
  ],
  webServer: {
    command: `python3 ../../../fieldkit/gatepost/mock_pod_api.py --port ${PORT}`,
    url: `${BASE_URL}/v1/status`,
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
  },
});
