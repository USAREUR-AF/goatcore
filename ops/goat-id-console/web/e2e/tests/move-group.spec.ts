// SPDX-License-Identifier: Apache-2.0
// Browser coverage for the Move Group entry point on the user detail page.
import { execFileSync } from "node:child_process";
import { test, expect, type Page } from "@playwright/test";
import { CONSOLE_URL, CONSOLE_API_KEY, GOVERNED_CONSOLE_URL, IDENTITY_URL } from "../playwright.config";

const REAL_PLANNER = !!(process.env.MOVE_PLANNER_GOAT_BIN && process.env.MOVE_PLANNER_PROFILE_DIR);
const USER_DETAIL = "/users/00000000-0000-4000-8000-000000000004";
const PEERS = [
  { name: "peer-move-target-acme-zagros-20260604-laptop", id: "e1f2a3b4c5d6e7f8091a2b3c" },
  { name: "peer-move-target-acme-zagros-20260604-phone", id: "f2a3b4c5d6e7f8091a2b3c4d" },
];

test.describe.configure({ mode: "default" });
test.use({ baseURL: CONSOLE_URL });
test.skip(!!process.env.INTAKE_URL || process.env.CONSOLE_E2E === "0", "needs the local console (mocked mode only)");

test.beforeAll(async ({ request }) => {
  const existing = await request.get(CONSOLE_URL + USER_DETAIL);
  expect(existing.ok(), await existing.text()).toBeTruthy();
  if ((await existing.text()).includes(PEERS[0].id)) return;

  const invite = await request.post(CONSOLE_URL + "/api/onboard/invite", {
    headers: { Authorization: `Bearer ${CONSOLE_API_KEY}` },
    data: { env: "zagros", tier: "developer", affiliation: "partner" },
  });
  expect(invite.ok(), await invite.text()).toBeTruthy();
  const { token } = await invite.json();
  const submit = await request.post(CONSOLE_URL + "/public/onboard/submit", {
    data: {
      token,
      submission: {
        name: "Move Target",
        email: "move-target@example.invalid",
        organization: "Console test",
        nationality: "CAN",
      },
    },
  });
  expect(submit.status(), await submit.text()).toBe(201);
});

test("a platform admin opens the move dialog for each selected peer", async ({ page }) => {
  await page.setExtraHTTPHeaders({
    "X-Forwarded-Email": "platform-admin@example.invalid",
    "X-Forwarded-Groups": "PLATFORM-ADMIN",
  });
  await page.goto(USER_DETAIL);

  const dialog = page.getByRole("dialog", { name: "Move Group" });
  await expect(dialog).toHaveCount(0);
  for (const peer of PEERS) {
    const row = page.getByRole("row").filter({ hasText: peer.id });
    const move = row.getByRole("button", { name: "Move Group", exact: true });
    await expect(move).toBeVisible();
    await expect(move).toBeEnabled();
    await move.click();
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText(peer.name);
    const from = dialog.getByLabel("From group");
    await expect(from).toHaveValue("");
    const group = peer === PEERS[0] ? "ZAGROS-DEVELOPER" : "ZAGROS-OBSERVER";
    await expect(from.getByRole("option")).toHaveText(["Select a group", group]);
    await from.selectOption(group);
    await expect(from).toHaveValue(group);
    const to = dialog.getByRole("combobox", { name: "To group" });
    await expect(to).toHaveValue("");
    const preview = dialog.getByRole("button", { name: "Preview Move" });
    await expect(preview).not.toHaveClass(/primary/);
    const initialHeight = await dialog.evaluate((element) => element.offsetHeight);
    await to.click();
    await expect(dialog.getByRole("listbox")).toBeVisible();
    await expect(dialog.getByRole("option", { name: /-QUARANTINED$|-PROBATION-|-P2P$/ })).toHaveCount(0);
    expect(await dialog.evaluate((element) => element.offsetHeight)).toBe(initialHeight);
    await to.fill("  obsERv  ");
    const options = dialog.getByRole("listbox", { name: "Destination groups" });
    await expect(options.getByRole("option")).toHaveText(["ZAGROS-OBSERVER"]);
    expect(await dialog.evaluate((element) => element.offsetHeight)).toBe(initialHeight);
    await to.press("ArrowDown");
    await to.press("Enter");
    await expect(to).toHaveValue("ZAGROS-OBSERVER");
    await expect(preview).toHaveClass(/primary/);
    await expect(options).toBeHidden();
    await to.fill("missing-group");
    await expect(preview).not.toHaveClass(/primary/);
    await expect(dialog.getByRole("status")).toHaveText("No matching groups.");
    expect(await dialog.evaluate((element) => element.offsetHeight)).toBe(initialHeight);
    await to.fill("infra");
    await options.getByRole("option", { name: "ZAGROS-INFRA", exact: true }).click();
    await expect(to).toHaveValue("ZAGROS-INFRA");
    await expect(preview).toHaveClass(/primary/);
    if (!process.env.CONSOLE_URL || REAL_PLANNER) await expect(preview).toBeEnabled();
    else await expect(preview).toBeDisabled();
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toBeHidden();
    await move.click();
    await expect(from).toHaveValue("");
    await expect(dialog.getByLabel("To group")).toHaveValue("");
    await expect(options).toBeHidden();
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await expect(move).toBeFocused();
  }
});

test("an IAM admin sees a disabled move action and can discover the reason", async ({ page }) => {
  await page.setExtraHTTPHeaders({
    "X-Forwarded-Email": "iam-admin@example.invalid",
    "X-Forwarded-Groups": "IAM-ADMIN",
  });
  await page.goto(USER_DETAIL);

  const row = page.getByRole("row").filter({ hasText: PEERS[0].id });
  const move = row.getByRole("button", { name: "Move Group", exact: true });
  const tooltip = page.getByRole("tooltip");
  const reason = /you don.t have permission to move this device.*contact an administrator/i;

  await expect(move).toBeVisible();
  await expect(move).toBeDisabled();
  await move.hover();
  await expect(tooltip).toBeVisible();
  await expect(tooltip).toHaveText(reason);
  expect(await tooltip.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
  await page.getByRole("heading", { name: "Peers", exact: true }).hover();
  await expect(tooltip).toBeHidden();

  await move.focus();
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Tab");
  await expect(move).toBeFocused();
  await expect(tooltip).toBeVisible();
  await expect(move).toHaveAccessibleDescription(reason);
  const dialog = page.getByRole("dialog", { name: "Move Group" });
  for (const key of ["Enter", "Space"]) {
    await page.keyboard.press(key);
    await expect(dialog).toHaveCount(0);
  }
  // Use the mouse directly because Playwright will refuse to click an aria-disabled button.
  const bounds = await move.boundingBox();
  expect(bounds).not.toBeNull();
  await page.mouse.click(bounds!.x + bounds!.width / 2, bounds!.y + bounds!.height / 2);
  await expect(dialog).toHaveCount(0);
});

test.describe("disabled move explanation", () => {
  test.use({
    extraHTTPHeaders: {
      "X-Forwarded-Email": "iam-admin@example.invalid",
      "X-Forwarded-Groups": "IAM-ADMIN",
    },
  });

  test("keyboard users can dismiss the tooltip and keep button focus", async ({ page }) => {
    await page.goto(USER_DETAIL);
    const row = page.getByRole("row").filter({ hasText: PEERS[0].id });
    const move = row.getByRole("button", { name: "Move Group", exact: true });
    const tooltip = row.getByRole("tooltip");

    await move.focus();
    await expect(tooltip).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(tooltip).toBeHidden();
    await expect(move).toBeFocused();

    await page.keyboard.press("Shift+Tab");
    await page.keyboard.press("Tab");
    await expect(move).toBeFocused();
    await expect(tooltip).toBeVisible();
  });

  test("the tooltip stays inside a narrow viewport", async ({ page }) => {
    await page.setViewportSize({ width: 320, height: 568 });
    await page.goto(USER_DETAIL);
    const row = page.getByRole("row").filter({ hasText: PEERS[0].id });
    const move = row.getByRole("button", { name: "Move Group", exact: true });
    const tooltip = row.getByRole("tooltip");

    await move.focus();
    await expect(tooltip).toBeVisible();
    await expect.poll(() => tooltip.evaluate((element) => {
      const { top, right, bottom, left } = element.getBoundingClientRect();
      return left >= 0 && right <= window.innerWidth && top >= 0 && bottom <= window.innerHeight;
    })).toBe(true);
  });
});


async function openLaptopMove(page: Page) {
  await page.setExtraHTTPHeaders({
    "X-Forwarded-Email": "platform-admin@example.invalid",
    "X-Forwarded-Groups": "PLATFORM-ADMIN",
  });
  await page.goto(USER_DETAIL);
  await page.getByRole("row").filter({ hasText: PEERS[0].id })
    .getByRole("button", { name: "Move Group", exact: true }).click();
  return page.getByRole("dialog", { name: "Move Group" });
}

test("the configured CLI previews a move and explains a refused destination", async ({ page, request }) => {
  test.skip(!REAL_PLANNER || !!process.env.CONSOLE_URL, "requires the goat CLI and the isolated local console harness");
  const dialog = await openLaptopMove(page);
  const from = dialog.getByLabel("From group");
  const to = dialog.getByRole("combobox", { name: "To group" });
  const preview = dialog.getByRole("button", { name: "Preview Move" });
  const result = dialog.getByRole("status");
  await from.selectOption("ZAGROS-DEVELOPER");
  await to.fill("observer");
  await dialog.getByRole("option", { name: "ZAGROS-OBSERVER", exact: true }).click();
  await expect(preview).toBeEnabled();
  await expect(dialog.getByRole("button", { name: "Confirm Move" })).toBeHidden();
  const [consoleResponse] = await Promise.all([
    page.waitForResponse(response => response.url().includes("/mesh-move/preview")),
    preview.click(),
  ]);
  const profile = process.env.MOVE_PLANNER_PROFILE_DIR!;
  const cliPlan = JSON.parse(execFileSync(process.env.MOVE_PLANNER_GOAT_BIN!, [
    "--profile-dir=" + profile, "--context=zagros", "admin", "members", "move",
    "--env=zagros", "--planes=mesh", "--json", "--mgmt-url=" + IDENTITY_URL + "/zagros",
    "--peer-id=" + PEERS[0].id, "--from=ZAGROS-DEVELOPER", "--to=ZAGROS-OBSERVER",
  ], { cwd: profile, env: { GOAT_NB_API_TOKEN: "e2e-netbird-token", NO_COLOR: "1" }, encoding: "utf8" }));
  const consolePlan = await consoleResponse.json();
  expect(consolePlan).toMatchObject({
    from_group: cliPlan.from_group,
    to_group: cliPlan.to_group,
    already_at_destination: cliPlan.already_at_destination,
    movers: cliPlan.movers.map(({ peer_id, name, other_groups }) => ({
      peer_id, name, other_groups: other_groups.toSorted(),
    })),
  });
  await expect(dialog.getByRole("button", { name: "Confirm Move" })).toBeVisible();
  await expect(result.getByText("Preview succeeded", { exact: true })).toBeVisible();
  await expect(result).toHaveAttribute("data-state", "success");
  await expect(result).toContainText(`Move ${PEERS[0].name} from ZAGROS-DEVELOPER to ZAGROS-OBSERVER.`);
  await to.fill("infra");
  await dialog.getByRole("option", { name: "ZAGROS-INFRA", exact: true }).click();
  await expect(result).toBeHidden();
  await preview.click();
  await expect(result.getByText("Preview failed", { exact: true })).toBeVisible();
  await expect(result).toHaveAttribute("data-state", "failure");
  await expect(result).toContainText("No enabled network policy covers this destination group. Choose another group or ask a network administrator to review the policy.");
  await expect(from).toHaveValue("ZAGROS-DEVELOPER");
  await expect(to).toHaveValue("ZAGROS-INFRA");
  await expect(preview).toBeEnabled();
  await to.fill("developer");
  await dialog.getByRole("listbox").getByRole("option", { name: "ZAGROS-DEVELOPER", exact: true }).click();
  await preview.click();
  await expect(result.getByText("Preview failed", { exact: true })).toBeVisible();
  await expect(result).toContainText("Select a device and two different mesh groups.");
  const writes = await request.get(IDENTITY_URL + "/__fixture/netbird-writes");
  expect(writes.ok()).toBeTruthy();
  expect(await writes.json()).toBe(0);
});

test("a failed preview preserves selections and can be retried", async ({ page }) => {
  test.skip(!!process.env.CONSOLE_URL, "requires the isolated local console harness");
  const dialog = await openLaptopMove(page);
  const from = dialog.getByLabel("From group");
  const to = dialog.getByRole("combobox", { name: "To group" });
  const preview = dialog.getByRole("button", { name: "Preview Move" });
  const result = dialog.getByRole("status");
  await from.selectOption("ZAGROS-DEVELOPER");
  await to.fill("observer");
  await dialog.getByRole("option", { name: "ZAGROS-OBSERVER", exact: true }).click();

  const previewRoute = "**/mesh-move/preview?*";
  await page.route(previewRoute, route => route.abort(), { times: 1 });
  await preview.click();
  await expect(result.getByText("Preview failed", { exact: true })).toBeVisible();
  await expect(result).toContainText("Move preview could not be reached. Try again.");
  await expect(from).toHaveValue("ZAGROS-DEVELOPER");
  await expect(to).toHaveValue("ZAGROS-OBSERVER");
  await expect(preview).toBeEnabled();

  await page.route(previewRoute, route => route.fulfill({
    status: 503,
    contentType: "text/plain; charset=utf-8",
    body: "The console couldn’t access NetBird to validate this move. Ask a network administrator to check the console’s NetBird credentials. Reference: fixture-request.",
  }), { times: 1 });
  await preview.click();
  await expect(result.getByText("Preview failed", { exact: true })).toBeVisible();
  await expect(result).toContainText("check the console’s NetBird credentials. Reference: fixture-request.");
  await expect(preview).toBeEnabled();

  await preview.click();
  await expect(result.getByText("Preview succeeded", { exact: true })).toBeVisible();
  await expect(result).toContainText("to ZAGROS-OBSERVER.");
});

test("changing groups while a preview is pending clears the stale response", async ({ page }) => {
  test.skip(!!process.env.CONSOLE_URL, "requires the isolated local console harness");
  const dialog = await openLaptopMove(page);
  const to = dialog.getByRole("combobox", { name: "To group" });
  const preview = dialog.getByRole("button", { name: "Preview Move" });
  const result = dialog.getByRole("status");

  await dialog.getByLabel("From group").selectOption("ZAGROS-DEVELOPER");
  await to.fill("observer");
  await dialog.getByRole("option", { name: "ZAGROS-OBSERVER", exact: true }).click();
  const release = Promise.withResolvers<void>();
  const intercepted = Promise.withResolvers<void>();
  const dispatched = Promise.withResolvers<void>();
  const aborted = page.waitForEvent("requestfailed", {
    predicate: request => request.url().includes("/mesh-move/preview"),
  });
  await page.route("**/mesh-move/preview?*", async route => {
    intercepted.resolve();
    try {
      await release.promise;
      await route.fulfill({
        contentType: "application/json",
        json: { from_group: "ZAGROS-DEVELOPER", to_group: "ZAGROS-OBSERVER", movers: [{ name: PEERS[0].name }] },
      });
    } finally {
      dispatched.resolve();
    }
  }, { times: 1 });

  try {
    await preview.click();
    await intercepted.promise;
    await expect(result).toHaveText("Checking move…");
    await expect(preview).toBeDisabled();
    await to.fill("infra");
    await dialog.getByRole("option", { name: "ZAGROS-INFRA", exact: true }).click();
    await aborted;
    await expect(result).toBeHidden();
  } finally {
    release.resolve();
  }
  await dispatched.promise;
  await expect(result).toBeHidden();
  await expect(preview).toBeEnabled();
  await preview.click();
  await expect(result.getByText("Preview failed", { exact: true })).toBeVisible();
  await expect(result).toContainText("No enabled network policy covers");
});


test("confirmation rechecks changed plans and requires a second click", async ({ page }) => {
  test.skip(!!process.env.CONSOLE_URL, "requires the isolated local console harness");
  const dialog = await openLaptopMove(page);
  const to = dialog.getByRole("combobox", { name: "To group" });
  const preview = dialog.getByRole("button", { name: "Preview Move" });
  const confirm = dialog.getByRole("button", { name: "Confirm Move" });
  const result = dialog.getByRole("status");
  await dialog.getByLabel("From group").selectOption("ZAGROS-DEVELOPER");
  await to.fill("observer");
  await dialog.getByRole("listbox").getByRole("option", { name: "ZAGROS-OBSERVER", exact: true }).click();
  await expect(confirm).toBeHidden();
  await preview.click();
  await expect(result).toContainText("Preview succeeded");
  await expect(confirm).toBeVisible();

  await page.route("**/mesh-move/confirm?*", async route => {
    const body = new URLSearchParams(route.request().postData() || "");
    expect(body.get("confirmation")).toBeTruthy();
    await route.fulfill({
      status: 409,
      json: {
        from_group: "ZAGROS-DEVELOPER",
        to_group: "ZAGROS-OBSERVER",
        movers: [{ name: PEERS[0].name, other_groups: ["ZAGROS-P2P"] }],
        authority: "enforce",
        confirmation_state: "changed",
        confirmation_token: "fixture-updated-confirmation",
      },
    });
  }, { times: 1 });

  await confirm.click();
  await expect(result).toContainText("Plan changed");
  await expect(result).toContainText("Keep ZAGROS-P2P");
  await expect(result).toContainText("Review the updated plan and confirm again.");
  await expect(confirm).toBeEnabled();

  await page.route("**/mesh-move/confirm?*", async route => {
    expect(new URLSearchParams(route.request().postData() || "").get("confirmation"))
      .toBe("fixture-updated-confirmation");
    await route.fulfill({ json: { confirmation_state: "confirmed" } });
  }, { times: 1 });
  await confirm.click();
  await expect(result).toContainText("Plan confirmed");
  await expect(result).toContainText("No membership changes were applied.");
  await expect(confirm).toBeHidden();
});

test("unchanged plan confirms through the actual endpoint", async ({ page, request }) => {
  test.skip(!!process.env.CONSOLE_URL, "requires the isolated local console harness");
  const dialog = await openLaptopMove(page);
  const to = dialog.getByRole("combobox", { name: "To group" });
  await dialog.getByLabel("From group").selectOption("ZAGROS-DEVELOPER");
  await to.fill("observer");
  await dialog.getByRole("listbox").getByRole("option", { name: "ZAGROS-OBSERVER", exact: true }).click();
  await dialog.getByRole("button", { name: "Preview Move" }).click();
  await expect(dialog.getByRole("status")).toContainText("Preview succeeded");
  await dialog.getByRole("button", { name: "Confirm Move" }).click();
  await expect(dialog.getByRole("status")).toContainText("Plan confirmed");
  const writes = await request.get(IDENTITY_URL + "/__fixture/netbird-writes");
  expect(await writes.json()).toBe(0);
});


test("governed handoff shows copyable YAML and PR instructions", async ({ page, context }) => {
  test.skip(!!process.env.CONSOLE_URL, "requires the isolated local console harness");
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const dialog = await openLaptopMove(page);
  const to = dialog.getByRole("combobox", { name: "To group" });
  await dialog.getByLabel("From group").selectOption("ZAGROS-DEVELOPER");
  await to.fill("observer");
  await dialog.getByRole("listbox").getByRole("option", { name: "ZAGROS-OBSERVER", exact: true }).click();
  await dialog.getByRole("button", { name: "Preview Move" }).click();
  await expect(dialog.getByText("Preview succeeded", { exact: true })).toBeVisible();
  const yaml = "  - peer: synthetic-laptop\n    grant: [ZAGROS-OBSERVER]\n";
  const mockHandoff = () => page.route("**/mesh-move/confirm?*", route => route.fulfill({
    json: {
      confirmation_state: "declaration_ready",
      declaration: {
        fragment: "ops/warrant/membership/zagros/orgs/fixture.yaml",
        replaces_existing: true,
        yaml,
      },
    },
  }), { times: 1 });
  await mockHandoff();

  await dialog.getByRole("button", { name: "Confirm Move" }).click();
  await expect(dialog.getByText("Declaration ready for review", { exact: true })).toBeVisible();
  await expect(dialog.getByText("No membership changes were applied.")).toBeVisible();
  await expect(dialog.getByText("Update the device’s existing entry in:")).toBeVisible();
  await expect(dialog.getByText("ops/warrant/membership/zagros/orgs/fixture.yaml")).toBeVisible();
  await expect(dialog.getByText(/open a pull request for review/)).toBeVisible();
  await expect(dialog.getByText(/the warrant converger applies/)).toBeVisible();
  await expect(dialog.getByLabel("Proposed YAML")).toHaveValue(yaml);
  await expect(dialog.getByRole("button", { name: "Close", exact: true })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Cancel" })).toBeHidden();
  await expect(dialog.getByRole("button", { name: "Preview Move" })).toBeHidden();
  await expect(dialog.getByLabel("From group")).toBeHidden();
  await expect(to).toBeHidden();
  await expect(dialog.getByRole("button", { name: "Copy YAML" })).toBeFocused();
  await dialog.getByRole("button", { name: "Copy YAML" }).click();
  await expect(dialog.getByText("Copied", { exact: true })).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(yaml);
  await expect(dialog.getByRole("button", { name: "Confirm Move" })).toBeHidden();
  await dialog.getByRole("button", { name: "Edit move" }).click();
  await expect(dialog.getByLabel("Proposed YAML")).toBeHidden();
  await expect(dialog.getByText("Declaration ready for review", { exact: true })).toBeHidden();
  await expect(dialog.getByLabel("From group")).toBeFocused();
  await expect(dialog.getByLabel("From group")).toHaveValue("ZAGROS-DEVELOPER");
  await expect(to).toHaveValue("ZAGROS-OBSERVER");
  await expect(dialog.getByRole("button", { name: "Confirm Move" })).toBeHidden();
  await dialog.getByRole("button", { name: "Preview Move" }).click();
  await expect(dialog.getByText("Preview succeeded", { exact: true })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Confirm Move" })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Cancel" })).toBeVisible();
  await mockHandoff();
  await dialog.getByRole("button", { name: "Confirm Move" }).click();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(dialog).toBeHidden();
});

test("governed preview shows effective changes and confirmation saves a read-only declaration", async ({ page, request }) => {
  test.skip(!REAL_PLANNER || !!process.env.CONSOLE_URL, "requires the rebuilt goat CLI and the isolated local console harness");
  const url = GOVERNED_CONSOLE_URL + "/users/00000000-0000-4000-8000-000000000005";
  const existing = await request.get(url);
  expect(existing.ok(), await existing.text()).toBeTruthy();
  if (!(await existing.text()).includes("d3e4f5a60718293b4c5d6e7f")) {
    const invite = await request.post(GOVERNED_CONSOLE_URL + "/api/onboard/invite", {
      headers: { Authorization: `Bearer ${CONSOLE_API_KEY}` },
      data: { env: "zagros-dev", tier: "developer", affiliation: "partner" },
    });
    expect(invite.ok(), await invite.text()).toBeTruthy();
    const { token } = await invite.json();
    const submit = await request.post(GOVERNED_CONSOLE_URL + "/public/onboard/submit", {
      data: {
        token,
        submission: {
          name: "Governed Target", email: "governed-target@example.invalid",
          organization: "Console test", nationality: "CAN",
        },
      },
    });
    expect(submit.status(), await submit.text()).toBe(201);
  }

  await page.setExtraHTTPHeaders({
    "X-Forwarded-User": "aaaaaaaa-1111-4111-8111-111111111111",
    "X-Forwarded-Email": "platform-admin@example.invalid",
    "X-Forwarded-Groups": "PLATFORM-ADMIN",
  });
  await page.goto(url);
  const held = page.getByRole("row").filter({ hasText: "c2d3e4f5a60718293b4c5d6e" })
    .getByRole("button", { name: "Move Group", exact: true });
  await expect(held).toHaveAttribute("aria-disabled", "true");
  const move = page.getByRole("row").filter({ hasText: "d3e4f5a60718293b4c5d6e7f" })
    .getByRole("button", { name: "Move Group", exact: true });
  const dialog = page.getByRole("dialog", { name: "Move Group" });
  const preview = dialog.getByRole("button", { name: "Preview Move" });
  const confirm = dialog.getByRole("button", { name: "Confirm Move" });
  await move.click();
  const edit = dialog.getByRole("button", { name: "Edit move" });
  if (await edit.isVisible()) await edit.click();
  await dialog.getByLabel("From group").selectOption("ZAGROS-DEV-MONITOR");
  const destination = async (group: string) => {
    await dialog.getByRole("combobox", { name: "To group" }).fill(group);
    await dialog.getByRole("listbox").getByRole("option", { name: group, exact: true }).click();
  };
  await destination("ZAGROS-DEV-DEVELOPER");
  const [review] = await Promise.all([
    page.waitForResponse(response => response.url().includes("/mesh-move/preview")),
    preview.click(),
  ]);
  await expect(confirm).toBeEnabled();
  const { confirmation_token: proof } = await review.json();
  await expect(dialog.getByRole("status")).toContainText(
    "Warrant would add ZAGROS-DEV-DEVELOPER, ZAGROS-DEV-OBSERVER and remove ZAGROS-DEV-MONITOR.",
  );
  const [first, duplicate] = await dialog.locator("form").evaluate(async (form: HTMLFormElement, token) => {
    const body = new URLSearchParams(new FormData(form) as unknown as Record<string, string>);
    body.set("confirmation", token);
    const url = form.dataset.moveConfirmUrl!;
    return Promise.all([1, 2].map(async () => {
      const response = await fetch(url, { method: "POST", body });
      return { status: response.status, plan: await response.json() };
    }));
  }, proof);
  expect(first.status).toBe(200);
  expect(first.plan.operation_id).toMatch(/^move-/);
  expect(duplicate).toEqual(first);
  expect(first.plan.declaration.yaml).toContain('peer_id: "d3e4f5a60718293b4c5d6e7f"');
  expect(first.plan.declaration.yaml).toContain("grant: [ZAGROS-DEV-DEVELOPER]");

  await page.reload();
  await move.click();
  await expect(dialog.getByText("Declaration ready for review", { exact: true })).toBeVisible();
  await expect(dialog.getByLabel("Proposed YAML")).toHaveValue(first.plan.declaration.yaml);
  await expect(dialog.getByText(/open a pull request for review/)).toBeVisible();
  await expect(dialog.getByText(new RegExp(first.plan.operation_id))).toBeVisible();

  await dialog.getByRole("button", { name: "Edit move" }).click();
  // This live group passes the coverage gate but cannot be declared in this scope.
  await destination("ZAGROS-DEVELOPER");
  await preview.click();
  await expect(dialog.getByText("Preview failed", { exact: true })).toBeVisible();
  await expect(dialog.getByText(/verify the scope’s declaration and group rules/)).toBeVisible();
  await expect(confirm).toBeHidden();
  await expect(dialog.getByLabel("Proposed YAML")).toBeHidden();
  const writes = await request.get(IDENTITY_URL + "/__fixture/netbird-writes");
  expect(await writes.json()).toBe(0);
});
