// SPDX-License-Identifier: Apache-2.0
//
// E2E spec for the candidate intake form (web/static/onboard.html), the page
// a candidate opens from their invite email. It exists because of 3 Sep 2026:
// a markup rename (#569) left the submit handler reading an element that no
// longer existed, the handler threw after preventDefault() and before its
// fetch, and "Submit for review" did nothing on screen for about 26 hours on
// ZAGROS (#601). Every Go test was green. Only a browser running the page's own
// script against the page's own markup sees that class of failure, so this
// spec drives the real page in Chromium and fails on ANY uncaught page error.
//
// Mocked mode (default): /api/onboard/preview + /api/onboard/submit are
// stubbed via page.route, so the assertions are about what the page SENDS.
// Live mode (INTAKE_URL set): no stubs; the real goatnet must answer 201.

import { test, expect, type Page, type Route, type Request } from "@playwright/test";

const LIVE_URL = process.env.INTAKE_URL || "";
const TOKEN = "oti_e2e0000000000000000000000000";
const NAME = process.env.INTAKE_NAME || "Intake E2E";
const EMAIL = process.env.INTAKE_EMAIL || "intake-e2e@example.invalid";

type Submission = Record<string, unknown>;

// Stub the two token-gated candidate endpoints and capture what the page posts.
async function mockIntakeApi(page: Page): Promise<Array<{ token: string; submission: Submission }>> {
  const posts: Array<{ token: string; submission: Submission }> = [];
  await page.route("**/api/onboard/preview?*", (route: Route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        valid: true,
        env: "e2e",
        sponsor: "E2E Sponsor",
        escalated: false,
        tier_locked: true,
        prebound: { tier: "developer" },
        machine_quota_locked: false,
        org_locked: false,
      }),
    }),
  );
  await page.route("**/api/onboard/submit", (route: Route, req: Request) => {
    posts.push(JSON.parse(req.postData() || "{}"));
    return route.fulfill({
      status: 201,
      contentType: "application/json",
      body: JSON.stringify({ id: "prop-e2e", status: "proposed" }),
    });
  });
  return posts;
}

// Fail the test on any uncaught error in the page's script. This is the
// assertion the 3 Sep regression needed: the handler threw, nothing else
// happened, and no server-side signal existed.
function failOnPageErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  return errors;
}

async function fillPerson(page: Page) {
  await page.getByLabel(/^Full name/).fill(NAME);
  await page.getByLabel(/^Email/).fill(EMAIL);
  await page.getByLabel(/^Nationality/).fill("CAN");
  await page.getByLabel(/^Organization/).fill("E2E Org");
  await page.getByLabel(/^Sector/).selectOption("commercial");
  // When the sponsor left the access level open, the candidate must choose
  // one (a blank tier can never be completed at review). Pick the lowest.
  const tierSel = page.locator("#tierRow-sel");
  if ((await tierSel.count()) && (await tierSel.isVisible())) {
    await tierSel.selectOption(process.env.INTAKE_TIER || "browse");
  }
}

test.describe("candidate intake form (mocked backend)", () => {
  test.skip(!!LIVE_URL, "INTAKE_URL is set — running live mode instead");

  test("a person can submit, and the request carries every renamed control", async ({ page }) => {
    const errors = failOnPageErrors(page);
    const posts = await mockIntakeApi(page);
    await page.goto(`/onboard.html?t=${TOKEN}`);
    await expect(page.getByRole("button", { name: "Submit for review" })).toBeVisible();

    await fillPerson(page);
    await page.getByRole("button", { name: "Submit for review" }).click();

    await expect(page.getByText("your details were submitted for review")).toBeVisible();
    expect(errors).toEqual([]);
    expect(posts).toHaveLength(1);
    expect(posts[0].token).toBe(TOKEN);
    const sub = posts[0].submission;
    expect(sub.name).toBe(NAME);
    expect(sub.email).toBe(EMAIL);
    expect(sub.nationality).toBe("CAN");
    expect(sub.sector).toBe("commercial");
    expect(sub.principal_type).toBe("person");
    // The #569 rename: the control is alliance_standing, the retired key is gone.
    expect(sub).toHaveProperty("alliance_standing");
    expect(sub).not.toHaveProperty("coalition");
  });

  test("a blank GitHub username on a repo-granting level warns once, then submits", async ({ page }) => {
    const errors = failOnPageErrors(page);
    const posts: Array<{ token: string; submission: Submission }> = [];
    await page.route("**/api/onboard/preview?*", (route: Route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ valid: true, tier_locked: true, prebound: { tier: "developer" }, repo_tiers: ["developer"] }),
      }),
    );
    await page.route("**/api/onboard/submit", (route: Route, req: Request) => {
      posts.push(JSON.parse(req.postData() || "{}"));
      return route.fulfill({
        status: 201,
        contentType: "application/json",
        body: JSON.stringify({ proposal_id: "prop-e2e", status: "submitted", warnings: ["github_username_missing"] }),
      });
    });
    await page.goto(`/onboard.html?t=${TOKEN}`);
    await fillPerson(page);

    // First click: the warning, and nothing sent.
    await page.getByRole("button", { name: "Submit for review" }).click();
    await expect(page.getByText("You have not given a GitHub username.")).toBeVisible();
    expect(posts).toHaveLength(0);

    // Second click goes on without it; the warning is not a refusal.
    await page.getByRole("button", { name: "Submit without a GitHub username" }).click();
    await expect(page.getByText("your details were submitted for review")).toBeVisible();
    await expect(page.getByText("No GitHub username was given.")).toBeVisible();
    expect(posts).toHaveLength(1);
    expect(posts[0].submission.github_username).toBe("");
    expect(errors).toEqual([]);
  });

  test("a GitHub username on a repo-granting level submits with no warning", async ({ page }) => {
    const errors = failOnPageErrors(page);
    const posts: Array<{ token: string; submission: Submission }> = [];
    await page.route("**/api/onboard/preview?*", (route: Route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ valid: true, tier_locked: true, prebound: { tier: "developer" }, repo_tiers: ["developer"] }),
      }),
    );
    await page.route("**/api/onboard/submit", (route: Route, req: Request) => {
      posts.push(JSON.parse(req.postData() || "{}"));
      return route.fulfill({ status: 201, contentType: "application/json", body: JSON.stringify({ proposal_id: "prop-e2e", status: "submitted" }) });
    });
    await page.goto(`/onboard.html?t=${TOKEN}`);
    await fillPerson(page);
    await page.getByLabel(/^GitHub username/).fill("octocat");
    await page.getByRole("button", { name: "Submit for review" }).click();

    await expect(page.getByText("your details were submitted for review")).toBeVisible();
    await expect(page.getByText("You have not given a GitHub username.")).toBeHidden();
    expect(posts).toHaveLength(1);
    expect(posts[0].submission.github_username).toBe("octocat");
    expect(errors).toEqual([]);
  });

  test("an invalid GitHub username can be corrected and resubmitted", async ({ page }) => {
    const errors = failOnPageErrors(page);
    const posts: Array<{ token: string; submission: Submission }> = [];
    await page.route("**/api/onboard/preview?*", (route: Route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ valid: true, tier_locked: true, prebound: { tier: "developer" }, repo_tiers: ["developer"] }),
      }),
    );
    await page.route("**/api/onboard/submit", (route: Route, req: Request) => {
      posts.push(JSON.parse(req.postData() || "{}"));
      if (posts.length === 1) {
        return route.fulfill({
          status: 422,
          contentType: "application/json",
          body: JSON.stringify({
            error: {
              code: "validation_failed",
              message: "one or more fields need correction before this request can be reviewed",
              fields: { github_username: "GitHub username may contain only letters, numbers, and single hyphens" },
            },
          }),
        });
      }
      return route.fulfill({
        status: 201,
        contentType: "application/json",
        body: JSON.stringify({ proposal_id: "prop-e2e", status: "submitted" }),
      });
    });

    await page.goto(`/onboard.html?t=${TOKEN}`);
    await fillPerson(page);
    const githubInput = page.getByLabel(/^GitHub username/);
    await githubInput.fill("invalid--login");
    await page.getByRole("button", { name: "Submit for review" }).click();

    await expect(page.locator("#err")).toContainText("GitHub username");
    await expect(githubInput).toHaveAttribute("aria-invalid", "true");
    await expect(githubInput).toHaveAttribute("aria-describedby", "err-msg-github_username");
    await expect(githubInput).toBeFocused();
    await expect(page.getByLabel(/^Full name/)).toHaveValue(NAME);
    await expect(page.getByLabel(/^Email/)).toHaveValue(EMAIL);
    await expect(page.getByLabel(/^Organization/)).toHaveValue("E2E Org");
    expect(posts).toHaveLength(1);

    await githubInput.fill("corrected-login");
    await page.getByRole("button", { name: "Submit for review" }).click();
    await expect(page.getByText("your details were submitted for review")).toBeVisible();
    expect(posts).toHaveLength(2);
    expect(posts[0].submission.github_username).toBe("invalid--login");
    expect(posts[1].submission.github_username).toBe("corrected-login");
    expect(errors).toEqual([]);
  });

  test("a person without a nationality is stopped before any request is sent", async ({ page }) => {
    const errors = failOnPageErrors(page);
    const posts = await mockIntakeApi(page);
    await page.goto(`/onboard.html?t=${TOKEN}`);
    await page.getByLabel(/^Full name/).fill(NAME);
    await page.getByLabel(/^Email/).fill(EMAIL);
    await page.getByRole("button", { name: "Submit for review" }).click();

    // The browser's own `required` validation holds the submit; nothing posts,
    // the button stays live, no script error.
    expect(posts).toHaveLength(0);
    await expect(page.locator("#nationality:invalid")).toHaveCount(1);
    await expect(page.locator("#organization:invalid")).toHaveCount(1);
    await expect(page.getByRole("button", { name: "Submit for review" })).toBeEnabled();
    expect(errors).toEqual([]);
  });

  test("a system without an organization is stopped before any request is sent", async ({ page }) => {
    const errors = failOnPageErrors(page);
    const posts = await mockIntakeApi(page);
    await page.goto(`/onboard.html?t=${TOKEN}`);
    await page.getByLabel(/^Full name/).fill("E2E Sensor");
    await page.getByLabel(/^Email/).fill(EMAIL);
    await page.getByLabel(/a person or a system/).selectOption("system");
    // Nationality is not required for system, but organization is.
    await page.getByRole("button", { name: "Submit for review" }).click();

    // The browser's own `required` validation holds the submit.
    expect(posts).toHaveLength(0);
    await expect(page.locator("#nationality:invalid")).toHaveCount(0);
    await expect(page.locator("#organization:invalid")).toHaveCount(1);
    await expect(page.getByRole("button", { name: "Submit for review" })).toBeEnabled();
    expect(errors).toEqual([]);
  });

  test("a system submits without a nationality", async ({ page }) => {
    const errors = failOnPageErrors(page);
    const posts = await mockIntakeApi(page);
    await page.goto(`/onboard.html?t=${TOKEN}`);
    await page.getByLabel(/^Full name/).fill("E2E Sensor");
    await page.getByLabel(/^Email/).fill(EMAIL);
    await page.getByLabel(/^Organization/).fill("E2E Org");
    await page.getByLabel(/a person or a system/).selectOption("system");
    // `required` on nationality is for people; the script relaxes it for a system.
    await page.getByRole("button", { name: "Submit for review" }).click();

    await expect(page.getByText("your details were submitted for review")).toBeVisible();
    expect(errors).toEqual([]);
    expect(posts).toHaveLength(1);
    expect(posts[0].submission.principal_type).toBe("system");
    expect(posts[0].submission.nationality).toBe("");
  });

  test("a rejected submit is shown to the candidate, and the button comes back", async ({ page }) => {
    const errors = failOnPageErrors(page);
    await page.route("**/api/onboard/preview?*", (route: Route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ valid: true, tier_locked: true, prebound: { tier: "browse" } }) }),
    );
    await page.route("**/api/onboard/submit", (route: Route) =>
      route.fulfill({ status: 409, contentType: "application/json", body: JSON.stringify({ error: { code: "duplicate_email", message: "that email is already in flight" } }) }),
    );
    await page.goto(`/onboard.html?t=${TOKEN}`);
    await fillPerson(page);
    await page.getByRole("button", { name: "Submit for review" }).click();

    await expect(page.getByText(/email already has a request.*sponsor/)).toBeVisible();
    await expect(page.getByRole("button", { name: "Submit for review" })).toBeEnabled();
    expect(errors).toEqual([]);
  });

  test("a multi-field refusal marks all fields and focuses the first one", async ({ page }) => {
    const errors = failOnPageErrors(page);
    await page.route("**/api/onboard/preview?*", (route: Route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ valid: true, tier_locked: false, org_locked: false, prebound: {} }) }),
    );
    await page.route("**/api/onboard/submit", (route: Route) =>
      route.fulfill({
        status: 422,
        contentType: "application/json",
        body: JSON.stringify({
          error: {
            code: "validation_failed",
            message: "one or more fields need correction before this request can be reviewed",
            fields: {
              nationality:
                "please provide your nationality as an ISO 3166-1 alpha-3 code (e.g. CAN, USA)",
              organization: "organization is required",
              tier: "choose your access level",
            },
          },
        }),
      }),
    );
    await page.goto(`/onboard.html?t=${TOKEN}`);
    await fillPerson(page);
    // Simulate a server-side multi-field refusal naming organization, tier, and nationality.
    await page.getByRole("button", { name: "Submit for review" }).click();

    const errBox = page.locator("#err");
    await expect(errBox).toBeVisible();
    await expect(errBox.locator("li").first()).toContainText("Organization");
    const orgInput = page.getByLabel(/^Organization/);
    await expect(orgInput).toHaveAttribute("aria-invalid", "true");
    await expect(orgInput).toBeFocused();
    await expect(page.getByLabel(/^Nationality/)).toHaveAttribute("aria-invalid", "true");
    await expect(page.getByRole("button", { name: "Submit for review" })).toBeEnabled();
    expect(errors).toEqual([]);
  });

  test("organization sponsor lock can be toggled", async ({ page }) => {
    const errors = failOnPageErrors(page);
    await page.route("**/api/onboard/preview?*", (route: Route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          valid: true,
          tier_locked: true,
          org_locked: true,
          org_name: "Sponsor Org",
          prebound: { org: "sponsor-org-id", tier: "developer" },
        }),
      }),
    );
    await page.route("**/api/onboard/submit", (route: Route) =>
      route.fulfill({
        status: 201,
        contentType: "application/json",
        body: JSON.stringify({ id: "prop-e2e", status: "proposed" }),
      }),
    );

    await page.goto(`/onboard.html?t=${TOKEN}`);

    const orgInput = page.getByLabel(/^Organization/);
    await expect(orgInput).toHaveValue("Sponsor Org");
    await expect(orgInput).toHaveAttribute("readonly", "");

    const requestLink = page.getByRole("button", { name: "Request a different organization" });
    const keepLink = page.getByRole("button", { name: "Keep the organization my sponsor chose" });
    await expect(requestLink).toBeVisible();
    await expect(keepLink).toBeHidden();

    await requestLink.click();
    await expect(orgInput).not.toHaveAttribute("readonly", "");
    await expect(orgInput).toHaveValue("");
    await expect(orgInput).toHaveClass(/petition/);

    await keepLink.click();
    await expect(orgInput).toHaveValue("Sponsor Org");
    await expect(orgInput).toHaveAttribute("readonly", "");
    await expect(orgInput).not.toHaveClass(/petition/);
    await expect(requestLink).toBeVisible();
    await expect(keepLink).toBeHidden();

    await page.getByLabel(/^Full name/).fill(NAME);
    await page.getByLabel(/^Email/).fill(EMAIL);
    await page.getByLabel(/^Nationality/).fill("CAN");

    await page.getByRole("button", { name: "Submit for review" }).click();
    await expect(page.getByText("your details were submitted for review")).toBeVisible();
    expect(errors).toEqual([]);
  });

  test("keeping the sponsor's organization clears only that field's error", async ({ page }) => {
    const errors = failOnPageErrors(page);
    await page.route("**/api/onboard/preview?*", (route: Route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          valid: true,
          tier_locked: true,
          org_locked: true,
          org_name: "Sponsor Org",
          prebound: { org: "sponsor-org-id", tier: "developer" },
        }),
      }),
    );
    await page.route("**/api/onboard/submit", (route: Route) =>
      route.fulfill({
        status: 422,
        contentType: "application/json",
        body: JSON.stringify({
          error: {
            code: "validation_failed",
            message: "one or more fields need correction before this request can be reviewed",
            fields: {
              organization: "organization is required",
              nationality: "please provide your nationality as an ISO 3166-1 alpha-3 code (e.g. CAN, USA)",
            },
          },
        }),
      }),
    );
    await page.goto(`/onboard.html?t=${TOKEN}`);
    await page.getByLabel(/^Full name/).fill(NAME);
    await page.getByLabel(/^Email/).fill(EMAIL);
    await page.getByLabel(/^Nationality/).fill("CAN");
    // Ask for a different organization, type one, and let the server refuse
    // both the organization and the nationality.
    await page.getByRole("button", { name: "Request a different organization" }).click();
    await page.getByLabel(/^Organization/).fill("Another Org");
    await page.getByRole("button", { name: "Submit for review" }).click();

    const errBox = page.locator("#err");
    const orgInput = page.getByLabel(/^Organization/);
    const natInput = page.getByLabel(/^Nationality/);
    await expect(errBox).toBeVisible();
    await expect(orgInput).toHaveAttribute("aria-invalid", "true");
    await expect(natInput).toHaveAttribute("aria-invalid", "true");

    await page.getByRole("button", { name: "Keep the organization my sponsor chose" }).click();
    await expect(orgInput).toHaveValue("Sponsor Org");
    await expect(orgInput).not.toHaveAttribute("aria-invalid", "true");
    // The nationality refusal is still standing, and the summary still names it.
    await expect(natInput).toHaveAttribute("aria-invalid", "true");
    await expect(errBox).toBeVisible();
    await expect(errBox.locator("li")).toHaveCount(1);
    await expect(errBox.locator("li").first()).toContainText("Nationality");
    expect(errors).toEqual([]);
  });

  test("an account-name refusal points the candidate at the email field", async ({ page }) => {
    const errors = failOnPageErrors(page);
    await page.route("**/api/onboard/preview?*", (route: Route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ valid: true, tier_locked: true, prebound: { tier: "browse" } }) }),
    );
    await page.route("**/api/onboard/submit", (route: Route) =>
      route.fulfill({
        status: 422,
        contentType: "application/json",
        body: JSON.stringify({
          error: {
            code: "validation_failed",
            message: "one or more fields need correction before this request can be reviewed",
            fields: { handle: "your email address cannot form an account name; use a different address" },
          },
        }),
      }),
    );
    await page.goto(`/onboard.html?t=${TOKEN}`);
    await fillPerson(page);
    await page.getByRole("button", { name: "Submit for review" }).click();

    const errBox = page.locator("#err");
    await expect(errBox).toBeVisible();
    await expect(errBox.locator("li").first()).toContainText("Email");
    const emailInput = page.getByLabel(/^Email/);
    await expect(emailInput).toHaveAttribute("aria-invalid", "true");
    await expect(emailInput).toBeFocused();
    await expect(page.getByLabel(/^GitHub username/)).not.toHaveAttribute("aria-invalid", "true");
    expect(errors).toEqual([]);
  });
});

test.describe("candidate intake form (live goatnet)", () => {
  test.skip(!LIVE_URL, "set INTAKE_URL to a minted invite link to run against a goatnet");

  test("the public intake page accepts a real submit", async ({ page }) => {
    const errors = failOnPageErrors(page);
    await page.goto(LIVE_URL);
    await expect(page.getByRole("button", { name: "Submit for review" })).toBeVisible({ timeout: 20_000 });
    await fillPerson(page);
    // A goatnet whose repo policy covers this level warns once about the
    // blank GitHub username and asks for a second click; go on through it.
    const respP = page.waitForResponse((r) => r.url().includes("/api/onboard/submit"));
    await page.getByRole("button", { name: "Submit for review" }).click();
    const again = page.getByRole("button", { name: "Submit without a GitHub username" });
    if (await again.isVisible().catch(() => false)) {
      await again.click();
    }
    const resp = await respP;
    expect(resp.status(), await resp.text()).toBe(201);
    await expect(page.getByText("your details were submitted for review")).toBeVisible();
    expect(errors).toEqual([]);
  });
});
