// SPDX-License-Identifier: Apache-2.0
//
// Responsive layout spec for the goat-zenoh-portal shared shell and
// topics action row. Verifies the CSS-only fixes for:
//   * form-control min-width (now fluid with a 20rem cap),
//   * header grid stacking at narrow widths (nav on its own row,
//     theme-switcher + auth status kept visible), and
//   * the topics-page #vendor-picker bulk-actions wrap geometry.
//
// Backend remains fully mocked via page.route, matching
// tests/topic-form.spec.ts: /api/orgs + /api/orgs/*/topics are stubbed
// so /topics.html can initialize without a live portal.
//
// #topics-table (topics.html) and #vendors-table (index.html) are now
// wrapped in .table-scroll, so a wide table scrolls inside its own box
// instead of inflating document scrollWidth. Assertions here bound the
// story's surfaces (header, org picker, topic form) plus, for the
// topics page, whole-document scrollWidth.

import { test, expect, type Route, type Request } from "@playwright/test";

async function mockPortalApi(page: import("@playwright/test").Page): Promise<void> {
  await page.route("**/api/orgs", (route: Route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ vendors: [{ prefix: "acme", approved: true }] }),
    }),
  );

  await page.route("**/api/orgs/*/topics", (route: Route, req: Request) => {
    if (req.method() === "POST") {
      return route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ id: 1 }),
      });
    }
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ topics: [] }),
    });
  });
}

async function mockPortalApiWithTopic(page: import("@playwright/test").Page): Promise<void> {
  await page.route("**/api/orgs", (route: Route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ vendors: [{ prefix: "acme", approved: true }] }),
    }),
  );

  await page.route("**/api/orgs/*/topics", (route: Route, req: Request) => {
    if (req.method() === "POST") {
      return route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ id: 1 }),
      });
    }
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        topics: [
          {
            id: 1,
            key_pattern: "acme/radar1/RADAR-EXAMPLE/position_attitude",
            format: "json",
            publish_to_storage: false,
            last_seen_at: null,
            publish_count: 0,
            schema_ref: "",
          },
        ],
      }),
    });
  });
}

async function openTopics(page: import("@playwright/test").Page): Promise<void> {
  await page.goto("/topics.html");
  await expect(page.locator("#vendor-select")).toHaveValue("acme");
}

test.describe("portal responsive layout", () => {
  test("390x844 story surfaces (header, org picker, topic form) fit the 390px viewport", async ({ page }) => {
    await mockPortalApi(page);
    await page.setViewportSize({ width: 390, height: 844 });
    await openTopics(page);

    const header = page.locator("header");
    const vendorPicker = page.locator("#vendor-picker");
    const createTopicForm = page.locator("#create-topic-form");

    const [headerBox, vendorPickerBox, createTopicFormBox] = await Promise.all([
      header.boundingBox(),
      vendorPicker.boundingBox(),
      createTopicForm.boundingBox(),
    ]);

    if (!headerBox || !vendorPickerBox || !createTopicFormBox) {
      throw new Error("boundingBox() returned null for a story surface");
    }

    for (const box of [headerBox, vendorPickerBox, createTopicFormBox]) {
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(390);
    }
  });

  test("390px mobile operator can browse topics, toggle theme, open nav, and bulk-upload with story surfaces fitting the viewport", async ({ page }) => {
    await mockPortalApiWithTopic(page);
    await page.setViewportSize({ width: 390, height: 844 });
    await openTopics(page);

    async function expectStorySurfacesFitViewport() {
      const header = page.locator("header");
      const vendorPicker = page.locator("#vendor-picker");
      const createTopicForm = page.locator("#create-topic-form");

      const [headerBox, vendorPickerBox, createTopicFormBox] = await Promise.all([
        header.boundingBox(),
        vendorPicker.boundingBox(),
        createTopicForm.boundingBox(),
      ]);

      if (!headerBox || !vendorPickerBox || !createTopicFormBox) {
        throw new Error("boundingBox() returned null for a story surface");
      }

      for (const box of [headerBox, vendorPickerBox, createTopicFormBox]) {
        expect(box.x).toBeGreaterThanOrEqual(0);
        expect(box.x + box.width).toBeLessThanOrEqual(390);
      }
    }

    // Initial load: the mocked topic row really rendered, story surfaces fit.
    // Without this the mockPortalApiWithTopic fixture is unverified -- a throwing
    // refreshTopics leaves an "Error:" row in tbody and every geometry assertion
    // below still passes.
    await expect(page.locator("#topics-table tbody tr")).toHaveCount(1);

    // The .table-scroll wrapper must stop a 7-column table from widening the
    // page. This is the assertion the containment checks below cannot make:
    // before the wrapper, one real row put scrollWidth at 735 against a 390
    // viewport while every surface box still "fit".
    const docScrollWidth = await page.evaluate(() => document.documentElement.scrollWidth);
    expect(docScrollWidth).toBeLessThanOrEqual(390);

    await expectStorySurfacesFitViewport();

    // Toggle theme via the pinned segment and verify layout stability.
    const darkButton = page.locator('#theme-toggle button[data-mode="dark"]');
    await darkButton.click();
    await expectStorySurfacesFitViewport();

    // Expand a nav group; its dropdown should overlay content without forcing scroll.
    const catalogDetails = page.locator("header nav details.nav-group").first();
    const summary = catalogDetails.locator("summary");
    await summary.click();
    await expectStorySurfacesFitViewport();

    // Pick an org from the 100% width select.
    await page.selectOption("#vendor-select", "acme");
    await expectStorySurfacesFitViewport();

    // Reveal the bulk-import form via the Upload button.
    await page.click("#import-open");
    const bulkSection = page.locator("#bulk-import");
    await expect(bulkSection).toBeVisible();
    await expectStorySurfacesFitViewport();
  });

  test("390x844 header stacks without overlap and keeps controls visible", async ({ page }) => {
    await mockPortalApi(page);
    await page.setViewportSize({ width: 390, height: 844 });
    await openTopics(page);

    const header = page.locator("header");
    const brand = header.locator("h1");
    const nav = header.locator("nav");
    const themeToggle = header.locator("#theme-toggle");
    const who = header.locator("#who");

    const [headerBox, brandBox, navBox, themeBox, whoBox] = await Promise.all([
      header.boundingBox(),
      brand.boundingBox(),
      nav.boundingBox(),
      themeToggle.boundingBox(),
      who.boundingBox(),
    ]);

    if (!headerBox || !brandBox || !navBox || !themeBox || !whoBox) {
      throw new Error("boundingBox() returned null for a header element");
    }

    // The first row centers a shorter title beside 44px touch controls.
    const themeCenter = themeBox.y + themeBox.height / 2;
    const brandCenter = brandBox.y + brandBox.height / 2;
    expect(Math.abs(themeCenter - brandCenter)).toBeLessThanOrEqual(1);
    expect(themeBox.height).toBeGreaterThanOrEqual(44);

    // Nav sits below both first-row elements without overlap.
    expect(navBox.y).toBeGreaterThan(
      Math.max(brandBox.y + brandBox.height, themeBox.y + themeBox.height) - 1,
    );

    // #who drops below the nav row. Strict: equality is exactly the
    // pre-fix layout where #who still shares the nav row.
    expect(whoBox.y).toBeGreaterThan(navBox.y + navBox.height - 1);

    // Theme toggle and auth status must be fully within the viewport.
    expect(themeBox.x).toBeGreaterThanOrEqual(0);
    expect(themeBox.x + themeBox.width).toBeLessThanOrEqual(390);
    expect(whoBox.x).toBeGreaterThanOrEqual(0);
    expect(whoBox.x + whoBox.width).toBeLessThanOrEqual(390);

    // Nav and #who span the header width on their own rows.
    expect(navBox.x).toBeGreaterThanOrEqual(headerBox.x - 1);
    expect(navBox.x + navBox.width).toBeLessThanOrEqual(headerBox.x + headerBox.width + 1);
    expect(whoBox.x).toBeGreaterThanOrEqual(headerBox.x - 1);
    expect(whoBox.x + whoBox.width).toBeLessThanOrEqual(headerBox.x + headerBox.width + 1);

    // Header switches to a 2-column grid below 700px.
    const gridInfo = await page.evaluate(() => {
      const el = document.querySelector("header");
      if (!el) return null;
      const style = window.getComputedStyle(el);
      return {
        display: style.display,
        templateColumns: style.gridTemplateColumns,
      };
    });

    if (!gridInfo) {
      throw new Error("header element not found");
    }
    expect(gridInfo.display).toBe("grid");
    const colCount = gridInfo.templateColumns.split(" ").filter(Boolean).length;
    expect(colCount).toBe(2);
  });

  test("390x844 topics action row buttons and controls fit the viewport", async ({ page }) => {
    await mockPortalApi(page);
    await page.setViewportSize({ width: 390, height: 844 });
    await openTopics(page);

    const ids = ["#refresh-topics", "#export-yaml", "#export-json", "#import-open"] as const;
    for (const id of ids) {
      const box = await page.locator(id).boundingBox();
      if (!box) {
        throw new Error(`boundingBox() returned null for ${id}`);
      }
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(390);
    }

    const vendorSelectBox = await page.locator("#vendor-select").boundingBox();
    if (!vendorSelectBox) {
      throw new Error("boundingBox() returned null for #vendor-select");
    }
    expect(vendorSelectBox.x).toBeGreaterThanOrEqual(0);
    expect(vendorSelectBox.x + vendorSelectBox.width).toBeLessThanOrEqual(390);
  });

  test("390x844 form controls and org select stay within their sections", async ({ page }) => {
    await mockPortalApi(page);
    await page.setViewportSize({ width: 390, height: 844 });
    await openTopics(page);

    const controls = [
      "#vendor-select",
      "#t-key",
      "#t-format",
    ];

    for (const selector of controls) {
      const control = page.locator(selector);
      const section = control.locator("xpath=ancestor::section[1]");

      const [sectionBox, controlBox] = await Promise.all([
        section.boundingBox(),
        control.boundingBox(),
      ]);

      if (!sectionBox || !controlBox) {
        throw new Error(`boundingBox() returned null for ${selector} or its section`);
      }

      expect(controlBox.x).toBeGreaterThanOrEqual(sectionBox.x - 1);
      expect(controlBox.x + controlBox.width).toBeLessThanOrEqual(sectionBox.x + sectionBox.width + 1);
    }
  });

  test("300px viewport lets form controls shrink with their sections", async ({ page }) => {
    await mockPortalApi(page);
    await page.setViewportSize({ width: 300, height: 844 });
    await openTopics(page);

    const controls = [
      "#t-key",
      "#t-format",
      "#t-schema-ref",
      "#t-desc",
    ];

    for (const selector of controls) {
      const control = page.locator(selector);
      const section = control.locator("xpath=ancestor::section[1]");

      const [sectionBox, controlBox] = await Promise.all([
        section.boundingBox(),
        control.boundingBox(),
      ]);

      if (!sectionBox || !controlBox) {
        throw new Error(`boundingBox() returned null for ${selector} or its section`);
      }

      // Controls should shrink with their container, never forcing horizontal scroll.
      expect(controlBox.x).toBeGreaterThanOrEqual(sectionBox.x - 1);
      expect(controlBox.x + controlBox.width).toBeLessThanOrEqual(sectionBox.x + sectionBox.width + 1);
    }
  });

  test("390x844 #who auth status aligns left and shows unauthenticated pre-auth", async ({ page }) => {
    await mockPortalApi(page);
    await page.route('**/api/whoami', route => route.fulfill({status: 401, json: {error: {message: 'missing session'}}}));
    await page.setViewportSize({ width: 390, height: 844 });
    await openTopics(page);

    const who = page.locator("#who");
    const whoBox = await who.boundingBox();
    expect(whoBox).not.toBeNull();

    await expect(who).toHaveText("unauthenticated");

    const textAlign = await page.evaluate(() => {
      const el = document.querySelector("#who");
      if (!el) return null;
      return window.getComputedStyle(el).textAlign;
    });

    expect(textAlign).toBe("left");
  });

  test("390x844 vendor picker bulk actions wrap and flex-grow", async ({ page }) => {
    await mockPortalApi(page);
    await page.setViewportSize({ width: 390, height: 844 });
    await openTopics(page);

    const layout = await page.evaluate(() => {
      const bulk = document.querySelector("#vendor-picker .bulk-actions");
      if (!bulk) return null;
      const bulkStyle = window.getComputedStyle(bulk);
      const buttons = Array.from(bulk.querySelectorAll("button"));
      const buttonStyles = buttons.map((btn) => window.getComputedStyle(btn));
      const rects = buttons.map((btn) => btn.getBoundingClientRect());
      return {
        wrap: bulkStyle.flexWrap,
        growValues: buttonStyles.map((s) => s.flexGrow),
        tops: rects.map((r) => r.top),
        widths: rects.map((r) => r.width),
      };
    });

    if (!layout) {
      throw new Error("#vendor-picker .bulk-actions not found");
    }

    expect(layout.wrap).toBe("wrap");
    for (const grow of layout.growValues) {
      expect(grow).toBe("1");
    }

    const uniqueTops = Array.from(new Set(layout.tops.map((t) => Math.round(t))));
    expect(uniqueTops.length).toBeGreaterThanOrEqual(2);
  });

  test("390x844 vendor picker select expands to full section width", async ({ page }) => {
    await mockPortalApi(page);
    await page.setViewportSize({ width: 390, height: 844 });
    await openTopics(page);

    const section = page.locator("#vendor-picker");
    const select = page.locator("#vendor-select");

    const [sectionBox, selectBox] = await Promise.all([
      section.boundingBox(),
      select.boundingBox(),
    ]);

    if (!sectionBox || !selectBox) {
      throw new Error("boundingBox() returned null for #vendor-picker or #vendor-select");
    }

    expect(selectBox.x).toBeGreaterThanOrEqual(sectionBox.x - 1);
    expect(selectBox.x + selectBox.width).toBeLessThanOrEqual(sectionBox.x + sectionBox.width + 1);
  });

  test("390x844 nav dropdown menu opens below its summary and stays on-screen", async ({ page }) => {
    await mockPortalApi(page);
    await page.setViewportSize({ width: 390, height: 844 });
    await openTopics(page);

    const catalogDetails = page.locator("header nav details.nav-group").first();
    const summary = catalogDetails.locator("summary");
    const menu = catalogDetails.locator(".nav-menu");

    await summary.click();

    const [summaryBox, menuBox] = await Promise.all([
      summary.boundingBox(),
      menu.boundingBox(),
    ]);

    if (!summaryBox || !menuBox) {
      throw new Error("boundingBox() returned null for the nav summary or its menu");
    }

    // Menu should drop under the summary.
    expect(menuBox.y).toBeGreaterThan(summaryBox.y + summaryBox.height - 1);

    // Menu should stay fully within the viewport.
    expect(menuBox.x).toBeGreaterThanOrEqual(0);
    expect(menuBox.x + menuBox.width).toBeLessThanOrEqual(390);
  });

  test("1440x900 vendor picker bulk actions align to the right of the org select", async ({ page }) => {
    await mockPortalApi(page);
    await page.setViewportSize({ width: 1440, height: 900 });
    await openTopics(page);

    const section = page.locator("#vendor-picker");
    const label = section.locator("label").first();
    const bulkActions = section.locator(".bulk-actions");

    const [labelBox, bulkBox] = await Promise.all([
      label.boundingBox(),
      bulkActions.boundingBox(),
    ]);

    if (!labelBox || !bulkBox) {
      throw new Error("boundingBox() returned null for the org label or .bulk-actions");
    }

    // Desktop layout: bulk actions pushed to the right via margin-left:auto.
    expect(bulkBox.x).toBeGreaterThan(labelBox.x + labelBox.width - 1);
  });

  test("1440x900 desktop layout keeps header single-row and inputs at 20rem", async ({ page }) => {
    await mockPortalApi(page);
    await page.setViewportSize({ width: 1440, height: 900 });
    await openTopics(page);

    const header = page.locator("header");
    const brand = header.locator("h1");
    const nav = header.locator("nav");
    const themeToggle = header.locator("#theme-toggle");
    const who = header.locator("#who");

    const [brandBox, navBox, themeBox, whoBox] = await Promise.all([
      brand.boundingBox(),
      nav.boundingBox(),
      themeToggle.boundingBox(),
      who.boundingBox(),
    ]);

    if (!brandBox || !navBox || !themeBox || !whoBox) {
      throw new Error("boundingBox() returned null for a header element");
    }

    const tops = [brandBox.y, navBox.y, themeBox.y, whoBox.y];
    const minTop = Math.min(...tops);
    const maxTop = Math.max(...tops);
    expect(maxTop - minTop).toBeLessThanOrEqual(8);

    const firstText = page.locator("#create-topic-form input[type=\"text\"]").first();
    const firstSelect = page.locator("#t-format");

    const [textBox, selectBox] = await Promise.all([
      firstText.boundingBox(),
      firstSelect.boundingBox(),
    ]);

    if (!textBox || !selectBox) {
      throw new Error("boundingBox() returned null for a form control");
    }

    // 20rem cap ≈ 320px at the default root font size.
    for (const box of [textBox, selectBox]) {
      expect(box.width).toBeGreaterThanOrEqual(310);
      expect(box.width).toBeLessThanOrEqual(330);
    }
  });

  test("1440x900 desktop header keeps auth status right-aligned", async ({ page }) => {
    await mockPortalApi(page);
    await page.setViewportSize({ width: 1440, height: 900 });
    await openTopics(page);

    const who = page.locator("#who");
    const whoBox = await who.boundingBox();
    expect(whoBox).not.toBeNull();

    const textAlign = await page.evaluate(() => {
      const el = document.querySelector("#who");
      if (!el) return null;
      return window.getComputedStyle(el).textAlign;
    });

    expect(textAlign).toBe("right");
  });

  test("390x844 conditional protobuf/xml rows inherit fluid width when shown", async ({ page }) => {
    await mockPortalApi(page);
    await page.setViewportSize({ width: 390, height: 844 });
    await openTopics(page);

    // Protobuf row
    await page.selectOption("#t-format", "protobuf:");
    const protoInput = page.locator("#t-format-msgname");
    const protoSection = protoInput.locator("xpath=ancestor::section[1]");

    const [protoSectionBox, protoInputBox] = await Promise.all([
      protoSection.boundingBox(),
      protoInput.boundingBox(),
    ]);

    if (!protoSectionBox || !protoInputBox) {
      throw new Error("boundingBox() returned null for the protobuf message-name row");
    }

    expect(protoInputBox.x).toBeGreaterThanOrEqual(protoSectionBox.x - 1);
    expect(protoInputBox.x + protoInputBox.width).toBeLessThanOrEqual(protoSectionBox.x + protoSectionBox.width + 1);

    // XML row
    await page.selectOption("#t-format", "xml:");
    const xmlInput = page.locator("#t-format-elname");
    const xmlSection = xmlInput.locator("xpath=ancestor::section[1]");

    const [xmlSectionBox, xmlInputBox] = await Promise.all([
      xmlSection.boundingBox(),
      xmlInput.boundingBox(),
    ]);

    if (!xmlSectionBox || !xmlInputBox) {
      throw new Error("boundingBox() returned null for the xml element-name row");
    }

    expect(xmlInputBox.x).toBeGreaterThanOrEqual(xmlSectionBox.x - 1);
    expect(xmlInputBox.x + xmlInputBox.width).toBeLessThanOrEqual(xmlSectionBox.x + xmlSectionBox.width + 1);
  });

  test("390x844 theme switcher stays visible and is reachable by keyboard", async ({ page }) => {
    await mockPortalApi(page);
    await page.setViewportSize({ width: 390, height: 844 });
    await openTopics(page);

    const themeToggle = page.locator("#theme-toggle");
    await expect(themeToggle).toBeVisible();

    const firstButton = themeToggle.locator("button").first();
    await expect(firstButton).toBeVisible();

    // Walk the document's real tab order. A programmatic .focus() would pass
    // even for a tabindex="-1" element -- precisely the regression this test
    // exists to catch -- so press Tab until the theme button takes focus. The
    // bound is a generous ceiling over the header's tabbable count (nav links
    // plus collapsed nav-group summaries), not a claim about its exact index.
    let reached = false;
    for (let i = 0; i < 30 && !reached; i += 1) {
      await page.keyboard.press("Tab");
      reached = await firstButton.evaluate((el) => el === document.activeElement);
    }

    expect(reached).toBe(true);
    await expect(firstButton).toBeFocused();
  });
});
