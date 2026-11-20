// SPDX-License-Identifier: Apache-2.0
//
// Fieldkit Gatepost against the local mock of the pod API. Pins what the design
// requires (docs/design/fieldkit-and-bleating-edge.md §3, §6):
//   * the six actions, each showing its result and the `goat pod …` command it
//     equals BEFORE it runs, and doing nothing when the person says no;
//   * the duration picker offers 4, 24 and 72 hours and never more than the pod allows;
//   * a person who is not local authority can look but not change;
//   * a refusal and a silent pod both show what happened and what to do;
//   * the page asks nothing of any host but the pod that served it;
//   * every control is at least 44 px tall.

import { test, expect, type Page } from "@playwright/test";

const RADAR = "3f2a9c1e0b7d4e5f8a6b9c0d1e2f3a4b";
const CAMERA = "b81e44d0c2a94f6e9a7d3c5b1e0f2a6c";
const LIDAR = "e5d4c3b2a1f04e9d8c7b6a5f4e3d2c1b";

async function reset(page: Page, opts: Record<string, unknown> = {}) {
  const r = await page.request.post("/_mock/reset", { data: opts });
  expect(r.status()).toBe(204);
}

async function open(page: Page, opts: Record<string, unknown> = {}) {
  await reset(page, opts);
  await page.goto("/");
  await expect(page.locator("#banner")).not.toHaveAttribute("data-state", "unknown");
}

const action = (page: Page, label: string) => page.locator(".be-actions").getByRole("button", { name: label, exact: true });
const strip = (page: Page, name: string) => page.locator(`[data-strip="${name}"] code`);

test("status shows the cut-off banner, the sources, the rules and the command", async ({ page }) => {
  const outside: string[] = [];
  page.on("request", (r) => { if (!r.url().startsWith("http://127.0.0.1:")) outside.push(r.url()); });
  await open(page);
  await expect(page.locator("#banner")).toHaveAttribute("data-state", "cutoff");
  await expect(page.locator("#banner")).toContainText("Cut off from goat");
  await expect(page.locator("#who")).toContainText("You may change rules on this pod.");
  await expect(page.locator("#sources")).toContainText("radar-east");
  await expect(page.locator("#sources .be-row", { hasText: "radar-east" })).toContainText("Connection not known");
  await expect(page.locator("#sources .be-row", { hasText: "lidar-north" })).toContainText("Refused");
  await expect(page.locator("#rules")).toContainText("camera-gate");
  await expect(strip(page, "status")).toHaveText("goat pod status");
  await page.screenshot({ path: test.info().outputPath("status.png"), fullPage: true });
  expect(outside).toEqual([]);
});

test("connected pod says so", async ({ page }) => {
  await open(page, { connected: true });
  await expect(page.locator("#banner")).toHaveAttribute("data-state", "connected");
  await expect(page.locator("#banner")).toContainText("Connected to goat.");
});

test("let in: preview and command first, nothing on No, the rule on Yes, then waiting", async ({ page }) => {
  await open(page);
  await action(page, "Let in").click();
  await expect(page.locator("#p-allow")).toBeVisible();
  await expect(page.locator("#allow-go")).toBeDisabled();
  await page.locator("#allow-refused .be-row", { hasText: LIDAR }).getByRole("button", { name: "Pick" }).click();
  await expect(page.locator("#allow-source")).toHaveValue(LIDAR);
  const chips = page.locator("#allow-duration label");
  await expect(chips).toHaveText(["4 hours", "24 hours", "72 hours"]);
  await chips.filter({ hasText: "24 hours" }).click();
  await page.locator("#allow-reason").fill("survey team on site");
  await expect(page.locator("#allow-preview")).toContainText(`Result: ${LIDAR} may send through this pod until`);
  await expect(strip(page, "allow")).toHaveText(
    new RegExp(`^goat pod rules allow ${LIDAR} --until \\d{4}-\\d\\d-\\d\\dT\\d\\d:\\d\\d:00Z --reason 'survey team on site'$`));
  const command = await strip(page, "allow").textContent();

  await page.locator("#allow-go").click();
  const sheet = page.locator("dialog.be-sheet");
  await expect(sheet).toBeVisible();
  await expect(sheet.locator(".be-preview")).toContainText("may send through this pod until");
  await expect(sheet.locator("code")).toHaveText(command!);
  await sheet.getByRole("button", { name: "No, go back" }).click();
  await expect(sheet).toHaveCount(0);
  expect((await (await page.request.get("/v1/pending")).json()).pending).toEqual([]);

  await page.locator("#allow-go").click();
  await page.locator("dialog.be-sheet").getByRole("button", { name: "Yes, let in" }).click();
  await expect(page.locator("#done")).toContainText(`${LIDAR} may now send through this pod until`);
  const pending = (await (await page.request.get("/v1/pending")).json()).pending;
  expect(pending).toHaveLength(1);
  expect(pending[0]).toMatchObject({ action: "allow", source_slot: LIDAR, reason: "survey team on site" });

  await action(page, "Waiting").click();
  await expect(page.locator("#pending .be-row")).toHaveCount(1);
  await expect(page.locator("#pending")).toContainText(`Let in: ${LIDAR}`);
  await expect(page.locator("#pending")).toContainText("By zagros-op-alpha,");
  await expect(page.locator("#pending")).not.toContainText("[object Object]");
  await expect(strip(page, "pending")).toHaveText("goat pod pending");
});

test("shut out: the source is refused afterwards", async ({ page }) => {
  await open(page);
  await action(page, "Shut out").click();
  await page.locator("#revoke-list .be-row", { hasText: CAMERA }).getByRole("button", { name: "Pick" }).click();
  await page.locator("#revoke-reason").fill("device lost");
  await expect(page.locator("#revoke-preview")).toContainText("is shut out of this pod at once");
  await expect(strip(page, "revoke")).toHaveText(`goat pod rules revoke ${CAMERA} --reason 'device lost'`);
  await page.locator("#revoke-go").click();
  await page.locator("dialog.be-sheet").getByRole("button", { name: "Yes, shut out" }).click();
  await expect(page.locator("#done")).toContainText(`${CAMERA} is shut out of this pod.`);
  await action(page, "Status").click();
  await expect(page.locator("#sources .be-row", { hasText: "camera-gate" })).toContainText("Refused");
});

test("extend and bring back carry the rule id, the end time and the reason", async ({ page }) => {
  await open(page);
  await action(page, "Extend").click();
  await page.locator("#extend-list .be-row", { hasText: "c-102" }).getByRole("button", { name: "Pick" }).click();
  await page.locator("#extend-duration label", { hasText: "72 hours" }).click();
  await page.locator("#extend-reason").fill("exercise runs longer");
  await expect(page.locator("#extend-preview")).toContainText("Result: rule c-102 for camera-gate");
  await expect(strip(page, "extend")).toHaveText(/^goat pod rules extend c-102 --until \S+Z --reason 'exercise runs longer'$/);
  await page.locator("#extend-go").click();
  await page.locator("dialog.be-sheet").getByRole("button", { name: "Yes, extend" }).click();
  await expect(page.locator("#done")).toContainText("Rule c-102 now ends");

  await action(page, "Bring back").click();
  await page.locator("#reactivate-list .be-row", { hasText: "c-090" }).getByRole("button", { name: "Pick" }).click();
  await page.locator("#reactivate-duration label", { hasText: /^4 hours$/ }).click();
  await page.locator("#reactivate-reason").fill("team is back");
  await expect(page.locator("#reactivate-preview")).toContainText("is on again until");
  await expect(strip(page, "reactivate")).toHaveText(/^goat pod rules reactivate c-090 --until \S+Z --reason 'team is back'$/);
  await page.locator("#reactivate-go").click();
  await page.locator("dialog.be-sheet").getByRole("button", { name: "Yes, bring back" }).click();
  await expect(page.locator("#done")).toContainText("Rule c-090 now ends");
  const actions = (await (await page.request.get("/v1/pending")).json()).pending.map((p: { action: string }) => p.action);
  expect(actions).toEqual(["extend", "reactivate"]);
});

test("a person who is not local authority may look only", async ({ page }) => {
  await open(page, { may_change: false });
  await expect(page.locator("#who")).toContainText("You may look only. These people may change rules: zagros-op-alpha.");
  for (const label of ["Let in", "Shut out", "Extend", "Bring back"]) await expect(action(page, label)).toBeDisabled();
  for (const label of ["Status", "Waiting"]) await expect(action(page, label)).toBeEnabled();
});

test("the picker never offers more than the pod allows", async ({ page }) => {
  await open(page, { max_hours: 24 });
  await action(page, "Let in").click();
  await expect(page.locator('#allow-duration input[value="72"]')).toBeDisabled();
  await expect(page.locator('#allow-duration input[value="24"]')).toBeEnabled();
});

test("a refusal shows what happened, why and what to do", async ({ page }) => {
  await open(page);
  await page.route("**/v1/rules", (route) => route.request().method() === "POST"
    ? route.fulfill({ status: 400, contentType: "application/json",
        body: JSON.stringify({ error: { code: "E_POD_UNTIL_TOO_LONG", what: "That is longer than 72 hours.", why: "A rule made on a pod runs at most 72 hours.", fix: "Pick a shorter time." } }) })
    : route.continue());
  await action(page, "Let in").click();
  await page.locator("#allow-source").fill(RADAR);
  await page.locator("#allow-reason").fill("test");
  await page.locator("#allow-go").click();
  await page.locator("dialog.be-sheet").getByRole("button", { name: "Yes, let in" }).click();
  const err = page.locator("#error");
  await expect(err).toContainText("What happened");
  await expect(err).toContainText("That is longer than 72 hours.");
  await expect(err).toContainText("What to do");
  await expect(err).toContainText("Pick a shorter time.");
  await expect(err).toContainText("E_POD_UNTIL_TOO_LONG");
});

test("a pod that does not answer is said plainly", async ({ page }) => {
  await open(page);
  await page.route("**/v1/**", (route) => route.abort());
  await page.locator("#refresh").click();
  await expect(page.locator("#error")).toContainText("The pod did not answer.");
  await expect(page.locator("#error")).toContainText("press Refresh");
});

test("every control is at least 44 px tall", async ({ page }) => {
  await open(page);
  for (const panel of ["Status", "Let in", "Shut out", "Extend", "Bring back", "Waiting"]) {
    await action(page, panel).click();
    const small = await page.evaluate(() => Array.from(document.querySelectorAll("button, input[type=text], .be-duration span"))
      .filter((el) => (el as HTMLElement).offsetParent !== null)
      .map((el) => ({ el: (el.id || el.textContent || "").trim(), h: el.getBoundingClientRect().height }))
      .filter((x) => x.h < 44));
    expect(small, `controls under 44 px on ${panel}`).toEqual([]);
  }
});

test("a change the router rolled back is saved with its warning shown", async ({ page }) => {
  await open(page, { rollback: true });
  await action(page, "Let in").click();
  await page.locator("#allow-source").fill(`${LIDAR}.g2`);
  await page.locator("#allow-reason").fill("survey");
  await expect(strip(page, "allow")).toContainText(`goat pod rules allow ${LIDAR}.g2 --until`);
  await page.locator("#allow-go").click();
  await page.locator("dialog.be-sheet").getByRole("button", { name: "Yes, let in" }).click();
  await expect(page.locator("#done")).toContainText("Warning: the change is recorded, but the router did not start with it");
  await expect(page.locator("#banner")).toContainText("not sent to goat yet");
});

test("a bare slot the pod has never carried is refused with what to do", async ({ page }) => {
  await open(page);
  await action(page, "Let in").click();
  await page.locator("#allow-source").fill("00112233445566778899aabbccddeeff");
  await page.locator("#allow-reason").fill("new device");
  await page.locator("#allow-go").click();
  await page.locator("dialog.be-sheet").getByRole("button", { name: "Yes, let in" }).click();
  await expect(page.locator("#error")).toContainText("That source is not a certificate name.");
  await expect(page.locator("#error")).toContainText("Copy the name from the device's certificate");
});
