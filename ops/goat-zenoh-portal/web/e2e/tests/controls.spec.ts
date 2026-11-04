import { test, expect, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";

const token = "synthetic-controls-key";
const schema = (storage = "protobuf-fds") => ({
  key: "slot/topic/v1", format: storage === "xsd" ? "xml:Track" : "protobuf:Track",
  vendor: "slot", schema: { version: "v2", format_storage: storage, all_versions: ["v1", "v2"] },
  source_lang: storage === "xsd" ? "xsd" : "proto",
  rendered: { files: [], messages: [], enums: [] }, proto_source: {}, examples: [],
});

async function shell(page: Page, authenticated = true) {
  if (authenticated) await page.addInitScript(t => localStorage.setItem("portalApiKey", t), token);
  let signedIn = authenticated;
  await page.route("**/api/**", r => {
    const path = new URL(r.request().url()).pathname;
    if (path === "/api/browser-session") {
      if (r.request().method() === 'DELETE') { signedIn = false; return r.fulfill({status: 204}); }
      expect(r.request().headers().authorization).toBe('Bearer ' + token);
      signedIn = true;
      return r.fulfill({status: 204});
    }
    if (path === "/api/branding") return r.fulfill({ json: { product_name: "Fixture", org: "slot", orgs: "slots" } });
    if (path === "/api/whoami") return signedIn ? r.fulfill({ json: { operator: true, visible_orgs: 1 } }) : r.fulfill({status: 401, json: {error: {message: "missing session"}}});
    if (path === "/api/health") return r.fulfill({ json: { version: "test", uptime: "0s" } });
    if (path === "/api/orgs") return r.fulfill({ json: { orgs: [{ prefix: "producer", effective_access: "controlled" }] } });
    if (path === "/api/access/grants") return r.fulfill({ json: { grants: [{ id: "grant-1", producer: "producer", grantee_kind: "org", grantee_id: "reader", status: "active" }] } });
    if (path === "/api/access/requests") return r.fulfill({ json: { requests: [] } });
    if (path === "/api/access/directory") return r.fulfill({ json: { orgs: [], people: [] } });
    return r.fulfill({ status: 404, json: { error: { message: "Unmocked fixture endpoint" } } });
  });
}

for (const path of ["/index.html", "/progress.html", "/admin/trashgoat.html"]) {
  for (const enter of [false, true]) test(`save key on ${path} with ${enter ? "Enter" : "click"}`, async ({ page }) => {
    await shell(page, false);
    await page.goto(path);
    await page.locator("#api-key-input").fill(`  ${token}  `);
    const reloaded = page.waitForEvent("domcontentloaded");
    if (enter) await page.locator("#api-key-input").press("Enter");
    else await page.locator("#api-key-save").click();
    await reloaded;
    await expect.poll(() => page.evaluate(() => localStorage.getItem("portalApiKey"))).toBeNull();
    await expect(page.locator("#auth-banner")).toBeHidden();
  });
}

for (const [storage, extension, bytes] of [["protobuf-fds", "fds", "\u0000\u0001binary"], ["xsd", "xsd", "<schema>fixture</schema>"], ["future-format", "bin", "opaque"]]) {
  test(`download authenticated ${extension} preserves bytes and version`, async ({ page }) => {
    await shell(page);
    await page.route("**/api/schemas/render/**", r => r.fulfill({ json: schema(storage) }));
    await page.route("**/api/schemas/slot/**", r => {
      expect(r.request().headers().authorization).toBeUndefined();
      expect(new URL(r.request().url()).search).toBe("?version=v2");
      return r.fulfill({ contentType: "application/octet-stream", headers: { "Content-Disposition": `attachment; filename=slot_topic_v1-v2.${extension}` }, body: Buffer.from(bytes) });
    });
    await page.goto("/schema.html?key=slot/topic/v1");
    const downloaded = page.waitForEvent("download");
    await page.locator("#m-download").click();
    const download = await downloaded;
    expect(download.suggestedFilename()).toBe(`slot_topic_v1-v2.${extension}`);
    expect(await readFile((await download.path())!)).toEqual(Buffer.from(bytes));
  });
}

test("raw JSON opens as inert text with session auth and the selected version", async ({ page }) => {
  await shell(page);
  let reads = 0;
  const body = { ...schema(), example: "<script>window.fixtureInjected = true</script>" };
  await page.route("**/api/schemas/render/**", r => {
    expect(r.request().headers().authorization).toBeUndefined();
    if (++reads > 1) expect(new URL(r.request().url()).searchParams.get("version")).toBe("v2");
    return r.fulfill({ json: body });
  });
  await page.goto("/schema.html?key=slot/topic/v1");
  const opened = page.waitForEvent("popup");
  await page.locator("#m-json").click();
  const popup = await opened;
  await expect(popup.locator("pre")).toContainText(body.example);
  expect(await popup.evaluate(() => (window as any).fixtureInjected)).toBeUndefined();
  expect(reads).toBe(2);
  expect(popup.url()).not.toContain(token);
  expect(await popup.evaluate(() => window.opener)).toBeNull();
});

for (const status of [401, 403, 500]) test(`schema download ${status} remains an inline failure`, async ({ page }) => {
  await shell(page);
  await page.route("**/api/schemas/render/**", r => r.fulfill({ json: schema() }));
  await page.route("**/api/schemas/slot/**", r => r.fulfill({ status, contentType: "text/plain", body: "fixture unavailable" }));
  let downloads = 0;
  page.on("download", () => downloads++);
  await page.goto("/schema.html?key=slot/topic/v1");
  await page.locator("#m-download").click();
  await expect(page.locator("#error")).toContainText(String(status));
  expect(downloads).toBe(0);
  await expect(page.locator("#m-download")).not.toHaveAttribute("aria-disabled", "true");
});

test("schema controls refuse missing credentials without a request", async ({ page }) => {
  await shell(page);
  let downloads = 0;
  await page.route("**/api/schemas/render/**", r => r.fulfill({ json: schema() }));
  await page.route("**/api/schemas/slot/**", r => { downloads++; return r.abort(); });
  await page.goto("/schema.html?key=slot/topic/v1");
  await expect(page.locator("#m-download")).toBeVisible();
  await page.evaluate(async () => { const {clearKey} = await import("/app.js"); await clearKey(); });
  await page.locator("#m-download").click();
  await expect(page.locator("#error")).toContainText("Sign in");
  expect(downloads).toBe(0);
});

test("raw JSON failure closes its reserved tab and reports the error", async ({ page }) => {
  await shell(page);
  let reads = 0;
  await page.route("**/api/schemas/render/**", r => ++reads === 1
    ? r.fulfill({ json: schema() }) : r.fulfill({ status: 403, body: "denied" }));
  await page.goto("/schema.html?key=slot/topic/v1");
  const opened = page.waitForEvent("popup");
  await page.locator("#m-json").click();
  const popup = await opened;
  await expect(page.locator("#error")).toContainText("403");
  await expect.poll(() => popup.isClosed()).toBe(true);
});

test("blocked JSON popup reports recovery without making the protected read", async ({ page }) => {
  await shell(page);
  let reads = 0;
  await page.route("**/api/schemas/render/**", r => { reads++; return r.fulfill({ json: schema() }); });
  await page.goto("/schema.html?key=slot/topic/v1");
  await page.evaluate(() => { window.open = () => null; });
  await page.locator("#m-json").click();
  await expect(page.locator("#error")).toContainText("Allow a new tab");
  expect(reads).toBe(1);
});

test("request confirmation survives form removal", async ({ page }) => {
  await shell(page);
  await page.goto("/data-access.html");
  await page.locator("[data-request]").click();
  await page.locator("#rq-grantee").fill("reader");
  await page.getByRole("button", { name: "Submit request" }).click();
  await expect(page.locator("#rq")).toHaveCount(0);
  await expect(page.getByRole("status")).toContainText("Requested");
});

test("revoke confirmation cancels without posting; failure is inline and retry succeeds", async ({ page }) => {
  await shell(page);
  let posts = 0;
  await page.route("**/api/access/grants/grant-1/revoke", r => {
    posts++;
    return r.fulfill({ status: posts === 1 ? 500 : 200, json: posts === 1 ? { error: { message: "fixture refusal" } } : {} });
  });
  await page.goto("/data-access.html");
  page.once("dialog", d => d.dismiss());
  await page.locator("[data-revoke]").click();
  expect(posts).toBe(0);
  page.once("dialog", d => d.accept());
  await page.locator("[data-revoke]").click();
  await expect(page.getByRole("alert")).toHaveText("The portal is having trouble. Please retry.");
  await expect(page.locator("[data-revoke]")).toBeEnabled();
  page.once("dialog", d => d.accept());
  await page.locator("[data-revoke]").click();
  await expect.poll(() => posts).toBe(2);
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("a pending revoke cannot post twice", async ({ page }) => {
  await shell(page);
  let finish!: () => void;
  const pending = new Promise<void>(resolve => { finish = resolve; });
  let posts = 0;
  await page.route("**/api/access/grants/grant-1/revoke", async r => {
    posts++;
    await pending;
    await r.fulfill({ json: {} });
  });
  await page.goto("/data-access.html");
  page.once("dialog", d => d.accept());
  await page.locator("[data-revoke]").click();
  await expect(page.locator("[data-revoke]")).toBeDisabled();
  await page.locator("[data-revoke]").dispatchEvent("click");
  await expect.poll(() => posts).toBe(1);
  finish();
  await expect(page.locator("[data-revoke]")).toBeEnabled();
});

for (const rawJSON of [false, true]) test(`schema ${rawJSON ? 'JSON' : 'download'} network failure recovers`, async ({ page }) => {
  await shell(page);
  let reads = 0;
  await page.route('**/api/schemas/render/**', r => ++reads === 1 || !rawJSON ? r.fulfill({ json: schema() }) : r.abort('failed'));
  await page.route('**/api/schemas/slot/**', r => r.abort('failed'));
  await page.goto('/schema.html?key=slot/topic/v1');
  const control = page.locator(rawJSON ? '#m-json' : '#m-download');
  const opened = rawJSON ? page.waitForEvent('popup') : null;
  await control.click();
  await expect(page.locator('#error')).toBeVisible();
  await expect(control).not.toHaveAttribute('aria-disabled', 'true');
  if (opened) { const popup = await opened; await expect.poll(() => popup.isClosed()).toBe(true); }
});
test('schema version switch controls the next protected download', async ({ page }) => {
  await shell(page);
  await page.route('**/api/schemas/render/**', r => r.fulfill({ json: { ...schema(), schema: { ...schema().schema, version: new URL(r.request().url()).searchParams.get('version') || 'v2' } } }));
  await page.route('**/api/schemas/slot/**', r => {
    expect(new URL(r.request().url()).searchParams.get('version')).toBe('v1');
    expect(r.request().headers().authorization).toBeUndefined();
    return r.fulfill({ body: 'version-one' });
  });
  await page.goto('/schema.html?key=slot/topic/v1');
  await page.locator('#version-select').selectOption('v1');
  await expect(page.locator('#m-version')).toHaveText('v1');
  const downloaded = page.waitForEvent('download');
  await page.locator('#m-download').click();
  const download = await downloaded;
  expect(download.suggestedFilename()).toBe('slot_topic_v1-v1.fds');
  expect(await readFile((await download.path())!, 'utf8')).toBe('version-one');
});
test('successful revoke refreshes to a terminal row without a revoke button', async ({ page }) => {
  await shell(page);
  let revoked = false;
  await page.route('**/api/access/grants', r => r.fulfill({ json: { grants: [{ id: 'grant-1', producer: 'producer', grantee_kind: 'org', grantee_id: 'reader', status: revoked ? 'revoked' : 'active' }] } }));
  await page.route('**/api/access/grants/grant-1/revoke', r => {
    expect(r.request().method()).toBe('POST');
    expect(r.request().headers().authorization).toBeUndefined();
    revoked = true; return r.fulfill({ json: { grant: 'grant-1', status: 'revoked' } });
  });
  await page.goto('/data-access.html');
  page.once('dialog', d => d.accept()); await page.locator('[data-revoke]').click();
  await expect(page.locator('#grants')).toContainText('revoked');
  await expect(page.locator('[data-revoke]')).toHaveCount(0);
});

for (const theme of ["light", "dark"]) {
  for (const width of [1366, 390]) test(`revoke retry error is separate in ${theme} at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 768 });
    await shell(page);
    await page.addInitScript(t => localStorage.setItem("portalTheme", t), theme);
    await page.route("**/api/access/grants/grant-1/revoke", r => r.fulfill({ status: 500, json: { error: { message: "Fixture refusal; retry is available." } } }));
    await page.goto("/data-access.html");
    page.once("dialog", d => d.accept());
    const button = page.locator("[data-revoke]");
    await button.click();
    const alert = page.getByRole("alert");
    await expect(alert).toHaveText("The portal is having trouble. Please retry.");
    await expect(button).toBeEnabled();
    const buttonBox = (await button.boundingBox())!;
    const alertBox = (await alert.boundingBox())!;
    expect(alertBox.y - (buttonBox.y + buttonBox.height)).toBeGreaterThanOrEqual(4);
  });
}

for (const disposition of [undefined, 'attachment; filename="../../unsafe.bin"', 'attachment; filename="server_chosen.bin"']) {
  test(`unknown schema download filename with ${disposition || "no header"}`, async ({ page }) => {
    await shell(page);
    await page.route("**/api/schemas/render/**", r => r.fulfill({ json: schema("future-format") }));
    await page.route("**/api/schemas/slot/**", r => r.fulfill({ body: "opaque", headers: disposition ? { "Content-Disposition": disposition } : {} }));
    await page.goto("/schema.html?key=slot/topic/v1");
    const downloaded = page.waitForEvent("download");
    await page.locator("#m-download").click();
    const download = await downloaded;
    expect(download.suggestedFilename()).toBe(disposition?.includes("server_chosen") ? "server_chosen.bin" : "slot_topic_v1-v2.bin");
  });
}
