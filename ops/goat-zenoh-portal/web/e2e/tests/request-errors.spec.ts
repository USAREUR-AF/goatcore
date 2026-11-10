import { test, expect, Page } from '@playwright/test';

test('catalog initializes without a stored API key', async ({ page }) => {
  const errors: string[] = [];
  let catalogCalls = 0;
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => localStorage.clear());
  await page.route('**/api/**', route => {
    const path = new URL(route.request().url()).pathname;
    if (path === '/api/whoami') return route.fulfill({status: 401, json: {error: {message: 'missing session'}}});
    if (path === '/api/catalog') catalogCalls++;
    return route.fulfill({ json: path === '/api/catalog'
      ? { orgs: [], totals: { orgs: 0 } } : {} });
  });
  await page.goto('/vendor.html');
  await expect(page.locator('#auth-banner')).toBeVisible();
  await expect.poll(() => errors.length || catalogCalls).toBeGreaterThan(0);
  expect(errors).toEqual([]);
  expect(catalogCalls).toBeGreaterThan(0);
  await expect(page.locator('#cards')).toContainText('Nothing matches');
});

async function routes(page: Page, failPath: string, status: number | 'network' | 'malformed', recover = false) {
  await page.addInitScript(() => localStorage.setItem('portalApiKey', 'fixture-key'));
  let failed = false;
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname;
    if (path === failPath && (!recover || !failed)) {
      failed = true;
      if (status === 'network') return route.abort('failed');
      if (status === 'malformed') return route.fulfill({ contentType: 'application/json', body: '{' });
      return route.fulfill({ status, contentType: 'text/plain', body: 'upstream text' });
    }
    const body = path === '/api/whoami' ? { operator: false, visible_orgs: 0 }
      : path === '/api/orgs' || path === '/api/catalog' ? { orgs: [], totals: { orgs: 0 } }
      : path === '/api/health' ? { version: 'fixture', uptime: 10 } : {};
    return route.fulfill({ json: body });
  });
}

for (const [url, path] of [['/index.html', '/api/orgs'], ['/vendor.html', '/api/catalog']]) {
  for (const [status, message] of [[401, 'Authentication failed'], [403, 'Permission denied'],
    [429, 'Wait and retry'], [500, 'portal is having trouble'], ['network', 'Connection failed'], ['malformed', 'invalid response']] as const) {
    test(`${url} distinguishes ${status}`, async ({ page }) => {
      await routes(page, path, status);
      await page.goto(url);
      await expect(page.getByRole('alert').filter({ hasText: message })).toBeVisible();
      if (status === 401) await expect(page.locator('#auth-banner')).toBeVisible();
      else await expect(page.locator('#auth-banner')).toBeHidden();
      expect(await page.evaluate(() => localStorage.getItem('portalApiKey'))).toBeNull();
    });
  }
  test(`${url} retry recovers from server failure`, async ({ page }) => {
    await routes(page, path, 500, true);
    await page.goto(url);
    await page.getByRole('alert').getByRole('button', { name: 'Retry' }).click();
    await expect(page.getByRole('alert').filter({ hasText: 'portal is having trouble' })).toHaveCount(0);
  });
}

for (const width of [390, 1440]) {
  test(`Home delayed response has a loading then empty state at ${width}`, async ({ page }) => {
    await page.setViewportSize({width, height: 900});
    await routes(page, '/never', 500);
    let release!: () => void;
    const ready = new Promise<void>(resolve => { release = resolve; });
    await page.route('**/api/orgs', async route => { await ready; await route.fulfill({ json: {orgs: []} }); });
    await page.goto('/index.html');
    await expect(page.getByRole('status').filter({hasText: 'Loading slots'})).toBeVisible();
    release();
    await expect(page.locator('#vendors-empty')).toBeVisible();
    await expect(page.getByRole('status').filter({hasText: 'Loading slots'})).toHaveCount(0);
  });
}

test('decoder retains status, retry metadata and accepts empty success', async ({ page }) => {
  await routes(page, '/never', 500);
  await page.goto('/index.html');
  const result = await page.evaluate(async () => {
    const { readResponse } = await import('/app.js');
    const empty = await readResponse(new Response(null, {status: 204}));
    try { await readResponse(new Response('slow down', { status: 429, headers: {'Retry-After': '15'} })); }
    catch(e) { return {empty, status: e.status, category: e.category, retryAfter: e.retryAfter}; }
  });
  expect(result).toEqual({empty: null, status: 429, category: 'rate-limit', retryAfter: '15'});
});

 test('JSON media types are case insensitive and retry supports touch and keyboard', async ({ page }) => {
  await routes(page, '/api/orgs', 500);
  await page.goto('/index.html');
  const retry = page.getByRole('alert').getByRole('button', {name: 'Retry'});
  await retry.focus();
  await expect(retry).toBeFocused();
  const box = await retry.boundingBox();
  expect(box!.height).toBeGreaterThanOrEqual(44);
  const result = await page.evaluate(async () => {
   const {readResponse} = await import('/app.js');
   return readResponse(new Response('{"ok":true}', {headers:{'Content-Type':'Application/JSON'}}));
  });
  expect(result).toEqual({ok:true});
 });

for (const theme of ['light', 'dark']) for (const width of [390, 1280]) {
  test(`forbidden operation retains identity and form values in ${theme} at ${width}`, async ({ page }, testInfo) => {
    await routes(page, '/never', 500);
    await page.setViewportSize({width, height: 900});
    await page.route(/\.html(?:\?|$)/, async route => {
      const response = await route.fetch();
      const lex = {ORG: 'vendor', ORGS: 'vendors', ORG_TITLE: 'Vendor', ORGS_TITLE: 'Vendors', ORG_UPPER: 'VENDOR'};
      const body = (await response.text()).replace(/__LEX_([A-Z_]+)__/g, (token, key) => lex[key] ?? token);
      await route.fulfill({response, body});
    });
    await page.route('**/api/orgs', route => route.fulfill({json: {vendors: [{prefix: 'fixture', approved: true}]}}));
    let allow = false;
    let posts = 0;
    const detail = '<img src=x onerror="window.permissionInjected=true"> vendor scope mismatch';
    await page.route('**/api/orgs/fixture/topics', route => {
      if (route.request().method() !== 'POST') return route.fulfill({json: {topics: []}});
      posts++;
      expect(route.request().headers().authorization).toBeUndefined();
      return allow ? route.fulfill({json: {id: 1}}) : route.fulfill({status: 403, json: {error: {message: detail}}});
    });
    await page.goto('/topics.html');
    await page.locator(`#theme-toggle button[data-mode="${theme}"]`).click();
    await expect(page.locator('#vendor-select')).toHaveValue('fixture');
    await expect(page.locator('#who')).toContainText('visible');
    const identity = await page.locator('#who').textContent();
    await page.locator('#t-key').fill('fixture/topic');
    await page.locator('#t-desc').fill('Keep this input');
    const submit = page.locator('#create-topic-form button[type="submit"]');
    await submit.click();
    await expect(page.locator('#topic-create-result')).toHaveRole('status');
    await expect(page.locator('#topic-create-result')).toHaveAttribute('aria-live', 'polite');
    await expect(page.locator('#topic-create-result')).toContainText(detail);
    await expect(page.locator('#topic-create-result img')).toHaveCount(0);
    expect(await page.evaluate(() => window['permissionInjected'])).toBeUndefined();
    await expect(page.locator('#auth-banner')).toBeHidden();
    await expect(page.locator('#who')).toHaveText(identity!);
    await expect(page.locator('#t-key')).toHaveValue('fixture/topic');
    await expect(page.locator('#t-desc')).toHaveValue('Keep this input');
    expect(await page.evaluate(() => localStorage.getItem('portalApiKey'))).toBeNull();
    expect(posts).toBe(1);
    await page.screenshot({path: testInfo.outputPath(`permission-${theme}-${width}.png`), fullPage: true});
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
    allow = true;
    await submit.focus();
    await submit.press('Enter');
    await expect(page.locator('#topic-create-result')).toContainText('"id": 1');
    expect(posts).toBe(2);
  });
}
