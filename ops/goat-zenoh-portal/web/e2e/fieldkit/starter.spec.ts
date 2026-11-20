// SPDX-License-Identifier: Apache-2.0
//
// The Bleating Edge starter template works as shipped: copied into a new tool, it
// renders its banner, row and "Same as" strip from one declared action. Served
// from the mock's origin so its one action reaches the mock's status route.

import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const starter = readFileSync(fileURLToPath(new URL("../../../../fieldkit/bleating-edge/starter.html", import.meta.url)), "utf8");

test("starter template renders against a pod status", async ({ page }) => {
  await page.route("**/starter.html", (route) => route.fulfill({ status: 200, contentType: "text/html", body: starter }));
  await page.goto("/starter.html");
  await expect(page.locator("#banner")).toHaveAttribute("data-state", /connected|cutoff/);
  await expect(page.locator("#list .be-row")).toHaveCount(1);
  await expect(page.locator("#strip code")).toHaveText("goat pod status");
});
