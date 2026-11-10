// SPDX-License-Identifier: Apache-2.0
// Browser coverage for the real probation templates and SQLite petition path.
// The console server talks to identity-fixture.mjs as its mocked NetBird API;
// no browser route mock stands in for the handler under test.
import { test, expect } from "@playwright/test";
import { CONSOLE_URL } from "../playwright.config";

const LIVE_URL = process.env.INTAKE_URL || "";

test.describe("probation recovery", () => {
  test.skip(!!LIVE_URL || process.env.CONSOLE_E2E === "0", "local console fixture is disabled");

  test("a held member sees the explainer and can submit one request", async ({ browser }) => {
    const context = await browser.newContext({
      viewport: { width: 1366, height: 768 },
      extraHTTPHeaders: {
        "X-Forwarded-For": "100.127.0.45",
        "X-Forwarded-Method": "GET",
        Accept: "text/html,application/xhtml+xml",
      },
    });
    const page = await context.newPage();
    const response = await page.goto(`${CONSOLE_URL}/internal/probation/auth`);
    expect(response?.status()).toBe(423);
    await expect(page.getByRole("heading", { name: "Your access is paused" })).toBeVisible();
    await expect(page.getByText(/Paused since .* \(\d+ days ago\)/)).toBeVisible();
    await expect(page.getByText(/\d+\.\d+ days/)).toHaveCount(0);
    await page.getByRole("button", { name: "Request reactivation" }).click();
    await expect(page.getByRole("status")).toHaveText("Your reactivation request has been recorded.");
    await page.goto(`${CONSOLE_URL}/internal/probation/auth`);
    await expect(page.getByRole("status")).toHaveText("Your reactivation request has been recorded.");
    await expect(page.getByRole("button", { name: "Request reactivation" })).toHaveCount(0);

    await page.goto(`${CONSOLE_URL}/reactivation-requests`);
    await expect(page.getByRole("heading", { name: "Reactivation requests" })).toBeVisible();
    const janeRow = page.locator("#reactivation-requests tbody tr", { hasText: "peer-jane-doe" });
    await expect(janeRow).toHaveCount(1);
    const memberCell = janeRow.locator("td").first();
    await expect(memberCell).toContainText("jane-doe");
    await expect(memberCell).toContainText("named in the hold record");
    await expect(janeRow.getByRole("link", { name: "View mesh hold" })).toBeVisible();
    await expect(janeRow.getByRole("link", { name: "Decline request" })).toBeVisible();
    await janeRow.getByRole("link", { name: "View mesh hold" }).click();
    await expect(page).toHaveURL(/\/holds\?filter=d0f1e2a3b4c5d6e7f8091a2b$/);
    await expect(page.locator("#holds-filter")).toHaveValue("d0f1e2a3b4c5d6e7f8091a2b");
    await expect(page.locator("tbody.hold-row:visible")).toHaveCount(1);
    await expect(page.locator("tbody.hold-row:visible")).toContainText("peer-jane-doe");

    await page.goto(`${CONSOLE_URL}/reactivation-requests`);
    await janeRow.getByRole("link", { name: "Decline request" }).click();
    await expect(page.getByRole("heading", { name: "Decline reactivation request" })).toBeVisible();
    await page.getByLabel("Reason").fill("Member asked before identity verification completed");
    await page.getByRole("button", { name: "Confirm decline" }).click();
    await expect(page).toHaveURL(/\/reactivation-requests\?declined=[^&]+$/);
    await expect(page.getByRole("status")).toHaveText("Reactivation request declined.");
    await expect(janeRow).toHaveCount(0);

    // Decline changes no hold. The same held member can ask again, which
    // reopens the exact row and returns it to the admin queue.
    await page.goto(`${CONSOLE_URL}/internal/probation/auth`);
    await expect(page.getByRole("button", { name: "Request reactivation" })).toBeVisible();
    await page.getByRole("button", { name: "Request reactivation" }).click();
    await page.goto(`${CONSOLE_URL}/reactivation-requests`);
    await expect(janeRow).toHaveCount(1);
    await expect(janeRow.getByRole("link", { name: "Decline request" })).toBeVisible();
    await context.close();
  });
});
