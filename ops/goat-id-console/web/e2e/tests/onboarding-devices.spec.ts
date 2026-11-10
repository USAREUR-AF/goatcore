// #2384: the candidate page stays usable while its selected device panel loads.
// Go handler tests pin this placeholder against the real rendered template;
// these browser tests exercise the actual script, with controlled responses.
import { test, expect, type Page } from "@playwright/test";

test.skip(!!process.env.INTAKE_URL, "mocked mode only");

async function mount(page: Page, selected = true) {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  await page.route("**/onboarding?*", (route) => route.fulfill({
    contentType: "text/html",
    body: `<!doctype html><html><body>
      <a href="/onboarding?status=archived">Archived</a>
      <input aria-label="Find a candidate">
      ${selected ? '<div data-devices-url="/onboarding/person/devices" aria-live="polite"><a href="/onboarding/person/devices">View mesh devices</a></div>' : ''}
      <script src="/onboard-devices.js"></script>
      </body></html>`,
  }));
  return errors;
}

test("a pending device lookup does not block the page; only one lookup is made", async ({ page }) => {
  const errors = await mount(page);
  let requests = 0;
  let finish!: () => void;
  const pending = new Promise<void>((resolve) => { finish = resolve; });
  await page.route("**/onboarding/person/devices", async (route) => {
    requests++;
    await pending;
    await route.fulfill({ contentType: "text/html", body: '<p class="ob-note">Devices on the mesh: fixture laptop · Windows 11</p>' });
  });
  await page.goto("/onboarding?status=enrolled&id=person");
  const panel = page.locator("[data-devices-url]");
  try {
    await expect(panel).toHaveAttribute("aria-busy", "true");
    await page.getByRole("textbox").fill("another candidate");
    await expect(page.getByRole("link", { name: "Archived" })).toBeVisible();
    expect(requests).toBe(1);
  } finally {
    finish();
  }
  await expect(panel).toContainText("fixture laptop · Windows 11");
  await expect(panel).toHaveAttribute("aria-busy", "false");
  expect(requests).toBe(1);
  expect(errors).toEqual([]);
});

test("a failed lookup offers a retry without reloading the page", async ({ page }) => {
  const errors = await mount(page);
  let requests = 0;
  await page.route("**/onboarding/person/devices", (route) => {
    requests++;
    return requests === 1
      ? route.fulfill({ status: 503, body: "unavailable" })
      : route.fulfill({ contentType: "text/html", body: "<p>No devices attributed to this handle.</p>" });
  });
  await page.goto("/onboarding?status=onboarded&id=person");
  await expect(page.locator("[data-devices-url]")).toContainText("Mesh devices could not be loaded");
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(page.locator("[data-devices-url]")).toContainText("No devices attributed");
  expect(requests).toBe(2);
  expect(errors).toEqual([]);
});

test("a login redirect is not inserted into the device panel", async ({ page }) => {
  const errors = await mount(page);
  await page.route("**/onboarding/person/devices", (route) => route.fulfill({ status: 302, headers: { Location: "/login" } }));
  await page.route("**/login", (route) => route.fulfill({ contentType: "text/html", body: "<h1>Login fixture</h1>" }));
  await page.goto("/onboarding?status=enrolled&id=person");
  await expect(page.getByRole("button", { name: "Retry", exact: true })).toBeVisible();
  await expect(page.locator("[data-devices-url]")).not.toContainText("Login fixture");
  expect(errors).toEqual([]);
});

test("a stage list without a selected device panel makes no device request", async ({ page }) => {
  const errors = await mount(page, false);
  let requests = 0;
  page.on("request", (request) => { if (request.url().endsWith("/devices")) requests++; });
  await page.goto("/onboarding?status=enrolled");
  await page.getByRole("textbox").fill("candidate");
  expect(requests).toBe(0);
  expect(errors).toEqual([]);
});
