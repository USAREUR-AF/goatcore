// SPDX-License-Identifier: Apache-2.0
//
// Pod Rules Waiting (carriage-pending.html): the org lead's queue for local
// rules a cut-off pod reported (ADR 1251, pod-carriage-and-provenance.md §7.5).
// Pins: the list comes from GET /api/carriage/local-rules?state=pending; each
// decision states its result and the `goat carriage` command first, does
// nothing on cancel, and calls /ratify or /lapse on yes; a refusal is shown.
// Backend mocked via page.route.

import { test, expect, type Page, type Route } from "@playwright/test";

const POD = "9d0c4b7e2a1f4c3d8e6b5a4f3c2d1e0f";
const LIDAR = "e5d4c3b2a1f04e9d8c7b6a5f4e3d2c1b";
const CAMERA = "b81e44d0c2a94f6e9a7d3c5b1e0f2a6c";

async function mock(page: Page, refuse = false) {
  const now = Math.floor(Date.now() / 1000);
  const decided: string[] = [];
  let rows = [
    { id: "lr-1", pod_slot: POD, seq: 1, action: "allow", pod_rule_id: "l-r1", source_slot: LIDAR, until: now + 86400,
      reason: "survey team on site", authority_identity: "zagros-op-alpha", entry_ts: now - 3600, state: "pending", received_at: now - 60 },
    { id: "lr-2", pod_slot: POD, seq: 2, action: "revoke", source_slot: CAMERA, until: now + 3600,
      reason: "device lost", authority_identity: "zagros-op-alpha", entry_ts: now - 1800, state: "pending", received_at: now - 60 },
  ];
  await page.addInitScript(() => localStorage.setItem("portalApiKey", "vp_test_key"));
  await page.route("**/api/**", (route: Route) => {
    const url = new URL(route.request().url());
    const json = (body: unknown, status = 200) =>
      route.fulfill({ status, contentType: "application/json", body: JSON.stringify(body) });
    if (url.pathname === "/api/branding") return json({ product_name: "Fabric Portal", org: "org", orgs: "orgs", org_title: "Org", orgs_title: "Orgs" });
    if (url.pathname === "/api/whoami") return json({ kind: "vendor", vendor: "a946693f0afcb5e707a9ecdc6d69ff6a", visible_orgs: 3 });
    if (url.pathname === "/api/siblings") return json({});
    if (url.pathname === "/api/carriage/local-rules") {
      expect(url.searchParams.get("state")).toBe("pending");
      return json({ local_rules: rows });
    }
    const m = url.pathname.match(/^\/api\/carriage\/local-rules\/([^/]+)\/(ratify|lapse)$/);
    if (m && route.request().method() === "POST") {
      if (refuse) return json({ error: { code: "forbidden", message: "only an org lead of the pod's org may decide this rule" } }, 403);
      decided.push(`${m[2]} ${m[1]}`);
      rows = rows.filter((r) => r.id !== m[1]);
      return json({ local_rule: { id: m[1], state: m[2] === "ratify" ? "ratified" : "lapsed" } });
    }
    return json({ error: { code: "not_mocked", message: url.pathname } }, 404);
  });
  return decided;
}

test("lists waiting local rules in plain words", async ({ page }) => {
  await mock(page);
  await page.goto("/carriage-pending.html");
  await expect(page.locator("#pending-status")).toHaveText("2 changes waiting.");
  const first = page.locator('tr[data-id="lr-1"]');
  await expect(first).toContainText("Let in");
  await expect(first).toContainText(LIDAR);
  await expect(first).toContainText("survey team on site");
  await expect(first).toContainText("zagros-op-alpha");
  await expect(page.locator('tr[data-id="lr-2"]')).toContainText("Shut out");
  await expect(page.locator('header nav a[href="/carriage-pending.html"]')).toHaveCount(1);
});

test("ratify and let lapse say the result and command first; cancel does nothing", async ({ page }) => {
  const decided = await mock(page);
  await page.goto("/carriage-pending.html");
  const messages: string[] = [];
  page.once("dialog", (d) => { messages.push(d.message()); d.dismiss(); });
  await page.locator('[data-ratify="lr-1"]').click();
  await expect.poll(() => messages.length).toBe(1);
  expect(messages[0]).toContain("as a goat rule");
  expect(messages[0]).toContain("Same as: goat carriage ratify lr-1");
  expect(decided).toEqual([]);

  page.once("dialog", (d) => { messages.push(d.message()); d.accept(); });
  await page.locator('[data-ratify="lr-1"]').click();
  await expect(page.locator('tr[data-id="lr-1"]')).toHaveCount(0);
  await expect(page.locator("#pending-status")).toContainText("Ratified: lr-1.");

  page.once("dialog", (d) => { messages.push(d.message()); d.accept(); });
  await page.locator('[data-lapse="lr-2"]').click();
  await expect(page.locator("#pending-status")).toContainText("Nothing is waiting.");
  expect(messages[2]).toContain("Same as: goat carriage lapse lr-2");
  expect(decided).toEqual(["ratify lr-1", "lapse lr-2"]);
});

test("a refusal is shown, and the row stays", async ({ page }) => {
  await mock(page, true);
  await page.goto("/carriage-pending.html");
  page.once("dialog", (d) => d.accept());
  await page.locator('[data-lapse="lr-2"]').click();
  await expect(page.locator("#pending-status")).toContainText("only an org lead");
  await expect(page.locator('tr[data-id="lr-2"]')).toHaveCount(1);
});
