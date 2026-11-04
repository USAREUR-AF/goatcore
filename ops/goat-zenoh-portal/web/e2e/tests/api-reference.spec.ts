import { test, expect } from "@playwright/test";

for (const theme of ["light", "dark"]) {
  test(`API reference supports search, deep links and authorization (${theme})`, async ({ page }, testInfo) => {
    await page.addInitScript((mode) => localStorage.setItem("portalTheme", mode), theme);
    await page.goto("/api-docs.html");
    const search = page.getByRole("searchbox", { name: "Search endpoints" });
    await expect(search).toBeEnabled();
    await expect(page.locator("#api-groups a")).toHaveCount(9);
    await page.locator("#api-groups").getByRole("link", { name: "machines 1", exact: true }).click();
    await expect(page).toHaveURL(/#\/machines$/);
    const repair = page.locator("#operations-machines-repairMachineAffiliation");
    await repair.getByRole("button", { name: /^POST/ }).click();
    await expect(repair).toContainText("Requires an operator bearer for both preview and apply.");
    await expect(repair.getByRole("heading", { name: "Responses", exact: true })).toBeVisible();
    await expect(page.locator("#api-introduction")).toContainText("Self-service portal");
    await expect(page.locator("#api-conventions-body")).toContainText("Authentication.");
    await page.screenshot({ path: testInfo.outputPath(`api-reference-${theme}.png`) });
    await page.locator("#operations-tag-vendors").getByRole("button").click();
    await search.fill("rotate");
    await expect(page.locator("#swagger-ui .opblock:visible")).toHaveCount(1);
    await expect(page.getByRole("status")).toHaveText("1 matching endpoint");
    await search.fill("no-such-operation");
    await expect(page.getByRole("status")).toHaveText("0 matching endpoints");
    await page.locator("#api-groups").getByRole("link", { name: "inspector 3", exact: true }).click();
    await expect(search).toHaveValue("");
    await expect(page).toHaveURL(/#\/inspector$/);
    const operation = page.locator("#operations-inspector-readLatest");
    await operation.getByRole("button", { name: /^GET/ }).click();
    await expect(operation.getByRole("button", { name: "Execute", exact: true })).toBeVisible();
    await expect(operation.getByRole("heading", { name: "Responses", exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Authorize", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Available authorizations" })).toBeVisible();
    await page.getByRole("button", { name: "Close", exact: true }).first().click();
  });
}

test("API reference reflows at phone width with expandable navigation and operations", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/api-docs.html");
  await expect(page.locator("#api-search")).toBeEnabled();
  await expect(page.locator("#api-search")).toBeHidden();
  await page.getByText("Browse the reference", { exact: true }).click();
  await expect(page.locator("#api-search")).toBeVisible();
  const operation = page.locator("#operations-topics-updateTopic");
  await operation.getByRole("button", { name: /^PATCH/ }).click();
  await expect(operation.getByRole("heading", { name: "Responses", exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth)).toBe(false);
  await page.screenshot({ path: testInfo.outputPath("api-reference-mobile.png") });
});

test("API reference sidebar stays reachable when the viewport crosses the breakpoint", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/api-docs.html");
  const search = page.locator("#api-search");
  const toggle = page.getByText("Browse the reference", { exact: true });
  await expect(search).toBeEnabled();
  await expect(search).toBeHidden();

  await page.setViewportSize({ width: 844, height: 390 });
  await expect(toggle).toBeHidden();
  await expect(search).toBeVisible();
  await expect(page.locator("#api-groups")).toBeVisible();
  await expect(page.locator("#api-groups a")).toHaveCount(9);
  await page.locator("#api-groups").getByRole("link", { name: "inspector 3", exact: true }).click();
  await expect(page).toHaveURL(/#\/inspector$/);

  await page.setViewportSize({ width: 390, height: 844 });
  await expect(toggle).toBeVisible();
  await toggle.click();
  await expect(search).toBeVisible();
  await toggle.click();
  await expect(search).toBeHidden();
  await page.setViewportSize({ width: 844, height: 390 });
  await expect(search).toBeVisible();
});

for (const failure of ["http", "network"]) {
  test(`API reference reports a failed specification load (${failure})`, async ({ page }, testInfo) => {
    await page.route("**/openapi.yaml", (route) => failure === "http"
      ? route.fulfill({ status: 500, body: "unavailable" })
      : route.abort("failed"));
    await page.goto("/api-docs.html");
    const status = page.locator("#api-status");
    await expect(status).toHaveText("Unable to load the API reference. See the specification errors below.");
    await expect(status).toBeVisible();
    await expect(page.locator("#api-search")).toBeDisabled();
    await expect(page.locator("#swagger-ui")).toContainText("Failed to load API definition.");
    await expect(page.locator("#swagger-ui .errors-wrapper")).toContainText("Fetch error");
    await page.screenshot({ path: testInfo.outputPath(`api-reference-failure-${failure}.png`) });
  });
}
