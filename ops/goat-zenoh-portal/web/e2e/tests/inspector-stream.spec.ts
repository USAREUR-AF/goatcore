// SPDX-License-Identifier: Apache-2.0
//
// E2E spec for the Inspector's live stream (#1497). The stream used to open
// an EventSource with the portal API key in the query string, which proxies
// write to their access logs, and the server (which reads only the
// Authorization header) answered 401 every time. The stream now reads the
// event stream with fetch and authenticates through its HttpOnly session.
//
// Backend is mocked via page.route, matching tests/topic-form.spec.ts: every
// /api/* call the page makes on load gets an empty 200, and the stream route
// is fulfilled with a canned text/event-stream body.

import { test, expect, type Page, type Request } from "@playwright/test";

const TEST_KEY = "test-portal-key";
const TOPIC = "testslot/sensor/temperature/v1";

function sample(celsius: number, ts: string): string {
  const payload = { key: TOPIC, value: { celsius }, encoding: "application/json", timestamp: ts };
  return `event: sample\ndata: ${JSON.stringify(payload)}\n\n`;
}

// Stub the page's load-time calls, seed the saved key, and route the stream
// to the given response. Returns the captured stream requests.
async function openInspector(
  page: Page,
  stream: { status: number; contentType: string; body: string },
): Promise<Request[]> {
  const requests: Request[] = [];
  await page.addInitScript((key) => localStorage.setItem("portalApiKey", key), TEST_KEY);
  await page.route("**/api/**", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: "{}" }),
  );
  await page.route("**/api/topics/stream**", (route, req) => {
    requests.push(req);
    return route.fulfill(stream);
  });
  await page.goto("/inspector.html");
  await page.fill("#key-expr", TOPIC);
  return requests;
}

test.describe("inspector live stream", () => {
  test("uses the browser session without putting credentials in the URL or headers", async ({ page }) => {
    const requests = await openInspector(page, {
      status: 200,
      contentType: "text/event-stream",
      body: ": heartbeat\n\n" + sample(21.4, "1") + sample(21.9, "2"),
    });
    await page.click("#toggle-stream");

    const log = page.locator("#stream-log");
    await expect(log).toContainText("21.9");
    await expect(log).toContainText("21.4");
    await expect(log).toContainText("# stream closed");
    await expect(page.locator("#toggle-stream")).toHaveText("Live stream: off");

    expect(requests).toHaveLength(1);
    const url = new URL(requests[0].url());
    expect(url.searchParams.get("key")).toBe(TOPIC);
    expect(url.searchParams.has("api_key")).toBe(false);
    expect(requests[0].url()).not.toContain(TEST_KEY);
    expect(requests[0].headers()["authorization"]).toBeUndefined();
    expect(await page.evaluate(() => localStorage.getItem("portalApiKey"))).toBeNull();
  });

  test("a refused stream says why and turns the button back off", async ({ page }) => {
    await openInspector(page, {
      status: 401,
      contentType: "application/problem+json",
      body: JSON.stringify({ title: "unauthorized" }),
    });
    await page.click("#toggle-stream");

    await expect(page.locator("#stream-log")).toContainText("# stream failed: HTTP 401 (check your API key)");
    await expect(page.locator("#toggle-stream")).toHaveText("Live stream: off");
  });
});
