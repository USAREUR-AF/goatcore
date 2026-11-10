// SPDX-License-Identifier: Apache-2.0
// Real-browser coverage for the console's release preview and guarded enqueue.
import { test, expect, type APIRequestContext } from "@playwright/test";
import { CONSOLE_URL, CONSOLE_API_KEY, GOVERNED_CONSOLE_URL } from "../playwright.config";

const SKIP = !!process.env.INTAKE_URL || process.env.CONSOLE_E2E === "0";
const RUNNER = { Authorization: `Bearer ${CONSOLE_API_KEY}`, "Content-Type": "application/json" };

test.use({ baseURL: CONSOLE_URL, viewport: { width: 1366, height: 768 } });
test.skip(SKIP, "needs the local console (mocked mode only)");

async function seedHandle(request: APIRequestContext, name: string, email: string) {
  const invite = await request.post(CONSOLE_URL + "/api/onboard/invite", {
    headers: RUNNER,
    data: { env: "zagros", tier: "developer", affiliation: "partner" },
  });
  expect(invite.ok(), await invite.text()).toBeTruthy();
  const { token } = await invite.json();
  const submit = await request.post(CONSOLE_URL + "/public/onboard/submit", {
    data: {
      token,
      submission: {
        name,
        email,
        organization: "Console test",
        nationality: "CAN",
      },
    },
  });
  expect(submit.status(), await submit.text()).toBe(201);
}

async function seedReactivationRequest(request: APIRequestContext) {
  const headers = {
    "X-Forwarded-For": "100.127.0.46",
    "X-Forwarded-Method": "GET",
    Accept: "text/html,application/xhtml+xml",
  };
  const page = await request.get(CONSOLE_URL + "/internal/probation/auth", { headers });
  expect(page.status(), await page.text()).toBe(423);
  const html = await page.text();
  const token = html.match(/name="token" value="([^"]+)"/)?.[1];
  expect(token).toBeTruthy();
  const posted = await request.post(CONSOLE_URL + "/__goat/probation/request", {
    headers: { ...headers, "X-Forwarded-Method": "POST", "Content-Type": "application/x-www-form-urlencoded" },
    form: { token: token! },
  });
  expect(posted.ok(), await posted.text()).toBeTruthy();
}

test("admin previews, cancels, and confirms an exact dormancy release while security stays refused", async ({ page, request }) => {
  await seedHandle(request, "Release Operator", "dev@bypass.local");
  await seedHandle(request, "Release Target", "release-target@example.invalid");
  await seedReactivationRequest(request);
  await page.goto("/holds");
  await expect(page.getByText("c2d3e4f5a60718293b4c5d6e")).toHaveCount(0);

  const dormancy = page.locator("tbody.hold-row", { hasText: "b1c2d3e4f5a60718293b4c5d" });
  await expect(dormancy.getByRole("link", { name: "Release hold" })).toBeVisible();
  const security = page.locator("tbody.hold-row", { hasText: "0a1b2c3d4e5f60718293a4b5" });
  await expect(security.getByRole("link", { name: "Release hold" })).toHaveCount(0);

  await dormancy.getByRole("link", { name: "Release hold" }).click();
  await expect(page.getByRole("heading", { name: "Release dormancy hold" })).toBeVisible();
  const preview = page.locator("#release-hold-preview");
  await expect(preview).toContainText("dormancy");
  await expect(preview).toContainText("sweep");
  await expect(preview).toContainText("zagros");
  await expect(preview).toContainText("b1c2d3e4f5a60718293b4c5d");
  await expect(preview).toContainText("ZAGROS-DEVELOPER, ZAGROS-P2P");

  await page.getByRole("link", { name: "Cancel" }).click();
  await expect(page).toHaveURL(/\/holds$/);

  await page.goto("/users/00000000-0000-4000-8000-000000000003");
  await page.getByRole("link", { name: "Release hold" }).click();
  await expect(page.getByRole("heading", { name: "Release dormancy hold" })).toBeVisible();
  await page.getByRole("link", { name: "Cancel" }).click();
  await expect(page).toHaveURL(/\/holds$/);

  await dormancy.getByRole("link", { name: "Release hold" }).click();
  await page.getByRole("button", { name: "Confirm release" }).click();
  await expect(page).toHaveURL(/\/holds\?release=queued$/);
  await expect(page.getByRole("status")).toContainText("Release queued");

  const queued = await request.get(CONSOLE_URL + "/api/onboard/actions?status=queued", { headers: RUNNER });
  expect(queued.ok(), await queued.text()).toBeTruthy();
  const actions = await queued.json();
  const release = actions.find((action: { kind: string }) => action.kind === "release_hold");
  expect(release).toBeTruthy();
  const claimed = await request.post(CONSOLE_URL + `/api/onboard/actions/${release.id}/claim`, { headers: RUNNER });
  expect(claimed.ok(), await claimed.text()).toBeTruthy();
  const claimedAction = await claimed.json();
  const result = await request.post(CONSOLE_URL + `/api/onboard/actions/${release.id}/result`, {
    headers: RUNNER,
    data: { status: "done", claim_token: claimedAction.claim_token, release: { outcome: "released", audit_outcome: "recorded" } },
  });
  expect(result.ok(), await result.text()).toBeTruthy();

  await page.goto("/holds");
  await expect(page.locator("#release-terminal")).toContainText("Hold release for peer-release-target-acme-zagros-20260602-laptop finished at ");
  await expect(page.locator("#release-terminal")).toContainText(": released.");
  await page.goto("/reactivation-requests");
  const releasedRequest = page.locator("#reactivation-requests tbody tr", { hasText: "b1c2d3e4f5a60718293b4c5d" });
  await expect(releasedRequest).toHaveCount(0);

  await page.goto("/holds/0a1b2c3d4e5f60718293a4b5/release?entered_at=1788447761");
  await expect(page.getByRole("alert")).toContainText("Security holds are released only through the ADR 1091 incident close-out path");
  await expect(page.getByRole("button", { name: "Confirm release" })).toHaveCount(0);
});

test("warrant-enforcing scope explains the reviewed path and offers no direct release", async ({ page }) => {
  test.skip(!!process.env.CONSOLE_URL && !process.env.GOVERNED_CONSOLE_URL, "external console needs GOVERNED_CONSOLE_URL for the governed-scope check");
  await page.goto(GOVERNED_CONSOLE_URL + "/holds");
  const authority = page.locator("#holds-authority");
  await expect(authority).toContainText("Membership is warrant-governed");
  await expect(authority.getByRole("link", { name: /reviewed warrant declaration-removal procedure/i })).toBeVisible();
  const row = page.locator("tbody.hold-row", { hasText: "c2d3e4f5a60718293b4c5d6e" });
  await expect(row).toContainText("dormancy");
  await expect(row).toContainText("warrant-governed");
  await expect(row.getByRole("link", { name: "Release hold" })).toHaveCount(0);

  const enteredAt = Date.parse("2026-09-04T12:00:00Z") / 1000;
  await page.goto(GOVERNED_CONSOLE_URL + `/holds/c2d3e4f5a60718293b4c5d6e/release?entered_at=${enteredAt}`);
  await expect(page.getByRole("alert")).toContainText("membership is warrant-governed");
  await expect(page.getByRole("link", { name: "Open the reviewed warrant membership declaration" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Confirm release" })).toHaveCount(0);
});
