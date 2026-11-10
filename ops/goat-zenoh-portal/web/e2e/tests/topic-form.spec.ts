// SPDX-License-Identifier: Apache-2.0
//
// E2E spec for the topic-registration form's protobuf footgun guard.
// The form's format dropdown used to submit the
// literal placeholder `protobuf:MyMessage`; a vendor shipped exactly that on
// all three of its radar topics and landed in the broken-decode state. The fix
// (topics.html + topics.js): the dropdown carries the bare `protobuf:`
// prefix and reveals a dedicated message-name field; createTopic splices
// the entered name into `protobuf:<name>` and refuses an empty value or
// the literal `MyMessage` client-side.
//
// Backend is mocked end-to-end via page.route — no Go portal, no DB.
// /api/orgs + the topics GET are stubbed so the form initializes;
// the topics POST is captured so we can assert what `format` was sent
// (and, for the guard cases, that NO POST happened).

import { test, expect, type Route, type Request } from "@playwright/test";

const KEY = "acme/radar1/RADAR-EXAMPLE/status";

// Install the API mocks and return a live array of captured POST bodies.
async function mockPortalApi(page: import("@playwright/test").Page): Promise<Array<Record<string, unknown>>> {
  const posts: Array<Record<string, unknown>> = [];

  // Vendor list — populates the vendor-select dropdown on init.
  await page.route("**/api/orgs", (route: Route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ vendors: [{ prefix: "acme", approved: true }] }),
    }),
  );

  // Topic list (GET refreshTopics) + topic create (POST) share the path.
  await page.route("**/api/orgs/*/topics", (route: Route, req: Request) => {
    if (req.method() === "POST") {
      const body = JSON.parse(req.postData() || "{}");
      posts.push(body);
      return route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ id: 1, ...body }),
      });
    }
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ topics: [] }),
    });
  });

  return posts;
}

async function openForm(page: import("@playwright/test").Page) {
  await page.goto("/topics.html");
  // Init fetches /api/orgs and populates the vendor select; wait for it.
  await expect(page.locator("#vendor-select")).toHaveValue("acme");
}

test.describe("topic-registration protobuf footgun guard", () => {
  test("dropdown carries the bare prefix, not the MyMessage placeholder", async ({ page }) => {
    await mockPortalApi(page);
    await openForm(page);
    const opt = page.locator('#t-format option', { hasText: "protobuf:" });
    await expect(opt).toHaveAttribute("value", "protobuf:");
    // The message-name row is hidden until protobuf is selected.
    await expect(page.locator("#t-format-msg-row")).toBeHidden();
  });

  test("selecting protobuf reveals the message-name field", async ({ page }) => {
    await mockPortalApi(page);
    await openForm(page);
    await page.selectOption("#t-format", "protobuf:");
    await expect(page.locator("#t-format-msg-row")).toBeVisible();
  });

  test("empty message name is rejected client-side with no POST", async ({ page }) => {
    const posts = await mockPortalApi(page);
    await openForm(page);
    await page.selectOption("#t-format", "protobuf:");
    await page.fill("#t-key", KEY);
    await page.fill("#t-format-msgname", "");
    await page.click('#create-topic-form button[type="submit"]');
    await expect(page.locator("#topic-create-result")).toContainText("Enter the protobuf message name");
    expect(posts).toHaveLength(0);
  });

  test("literal MyMessage placeholder is rejected with no POST", async ({ page }) => {
    const posts = await mockPortalApi(page);
    await openForm(page);
    await page.selectOption("#t-format", "protobuf:");
    await page.fill("#t-key", KEY);
    await page.fill("#t-format-msgname", "MyMessage");
    await page.click('#create-topic-form button[type="submit"]');
    await expect(page.locator("#topic-create-result")).toContainText("not the 'MyMessage' placeholder");
    expect(posts).toHaveLength(0);
  });

  test("a real message name POSTs format protobuf:<name>", async ({ page }) => {
    const posts = await mockPortalApi(page);
    await openForm(page);
    await page.selectOption("#t-format", "protobuf:");
    await page.fill("#t-key", KEY);
    await page.fill("#t-format-msgname", "PositionAttitude");
    await page.click('#create-topic-form button[type="submit"]');
    await expect.poll(() => posts.length).toBe(1);
    expect(posts[0]).toMatchObject({
      key_pattern: KEY,
      format: "protobuf:PositionAttitude",
    });
  });
});
