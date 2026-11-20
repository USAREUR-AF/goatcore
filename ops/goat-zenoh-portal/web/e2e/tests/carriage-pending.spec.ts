// SPDX-License-Identifier: Apache-2.0
//
// Pod Rules Waiting (carriage-pending.html): the org lead's queue for local
// rules a cut-off pod reported (ADR 1251, pod-carriage-and-provenance.md §7.5).
// Pins: the list comes from GET /api/carriage/local-rules?state=pending; each
// decision states its result and the `goat carriage` command first in an
// in-page sheet, does nothing on No or Escape, and calls /ratify or /lapse on yes; a refusal is shown.
// Backend mocked via page.route.

import { test, expect, type Page, type Route } from "@playwright/test";

const POD = "9d0c4b7e2a1f4c3d8e6b5a4f3c2d1e0f";
const LIDAR = "e5d4c3b2a1f04e9d8c7b6a5f4e3d2c1b";
const CAMERA = "b81e44d0c2a94f6e9a7d3c5b1e0f2a6c";

async function mock(page: Page, refuse: false | "forbidden" | "conflict" = false) {
  const now = Math.floor(Date.now() / 1000);
  const decided: string[] = [];
  let rows = [
    { id: "lr-1", pod_slot: POD, pod_org: "zagros-org-a", seq: 1, action: "allow", pod_rule_id: "l-r1", source_slot: LIDAR, until: now + 86400,
      reason: "survey team on site", authority_identity: "zagros-op-alpha", entry_ts: now - 3600, state: "pending", received_at: now - 60 },
    { id: "lr-2", pod_slot: POD, pod_org: "zagros-org-a", seq: 2, action: "revoke", source_slot: CAMERA, until: 0,
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
      if (refuse) {
        const [status, code] = refuse === "conflict" ? [409, "carriage_source_revoked"] : [403, "carriage_not_authorized"];
        return route.fulfill({ status, contentType: "application/problem+json",
          body: JSON.stringify({ type: "about:blank", title: "refused", status, code, detail: code, error: { code, message: code } }) });
      }
      decided.push(`${m[2]} ${m[1]}`);
      const decidedRule = { ...rows.find((r) => r.id === m[1]), state: m[2] === "ratify" ? "ratified" : "lapsed" };
      rows = rows.filter((r) => r.id !== m[1]);
      return json(decidedRule);
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
  await expect(page.locator('tr[data-id="lr-2"]')).toContainText("at once");
  await expect(page.locator('header nav a[href="/carriage-pending.html"]')).toHaveCount(1);
});

test("ratify and let lapse say the result and command first; No does nothing", async ({ page }) => {
  const decided = await mock(page);
  await page.goto("/carriage-pending.html");
  const sheet = page.locator("#decide-sheet");
  await page.locator('[data-ratify="lr-1"]').click();
  await expect(sheet).toBeVisible();
  await expect(sheet).toContainText("is kept as a goat rule");
  await expect(page.locator("#decide-cli")).toHaveText("goat carriage ratify lr-1");
  await sheet.getByRole("button", { name: "No, go back" }).click();
  await expect(sheet).toBeHidden();
  expect(decided).toEqual([]);

  await page.locator('[data-ratify="lr-1"]').click();
  await sheet.getByRole("button", { name: "Yes, ratify" }).click();
  await expect(page.locator('tr[data-id="lr-1"]')).toHaveCount(0);
  await expect(page.locator("#pending-status")).toContainText("Ratified: lr-1.");

  await page.locator('[data-lapse="lr-2"]').click();
  await expect(page.locator("#decide-cli")).toHaveText("goat carriage lapse lr-2");
  await page.keyboard.press("Escape");
  await expect(sheet).toBeHidden();
  expect(decided).toEqual(["ratify lr-1"]);
  await page.locator('[data-lapse="lr-2"]').click();
  await sheet.getByRole("button", { name: "Yes, let lapse" }).click();
  await expect(page.locator("#pending-status")).toContainText("Nothing is waiting for you.");
  expect(decided).toEqual(["ratify lr-1", "lapse lr-2"]);
});

for (const [kind, words] of [
  ["forbidden", "Only an org lead of the pod's org can decide this change."],
  ["conflict", "A revoke made in goat always wins"],
] as const) {
  test(`a ${kind} refusal is shown in plain words, and the row stays`, async ({ page }) => {
    await mock(page, kind);
    await page.goto("/carriage-pending.html");
    await page.locator('[data-ratify="lr-2"]').click();
    await expect(page.locator("#decide-result")).toContainText(`source ${CAMERA} stays shut out`);
    await page.locator("#decide-sheet").getByRole("button", { name: "Yes, ratify" }).click();
    await expect(page.locator("#pending-status")).toContainText(words);
    await expect(page.locator('tr[data-id="lr-2"]')).toHaveCount(1);
  });
}
