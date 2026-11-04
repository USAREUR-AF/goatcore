import { test, expect, type Page } from '@playwright/test';
import { readdirSync } from 'node:fs';

const peering = readdirSync(new URL('../../static/peering/', import.meta.url)).filter(p => p.endsWith('.html'));

async function mockIdentity(page: Page, operator: boolean, status = 200) {
  await page.addInitScript(() => localStorage.setItem('portalApiKey', 'navigation-test'));
  await page.route('**/api/**', route => {
    const path = new URL(route.request().url()).pathname;
    if (path === '/api/whoami') return route.fulfill({ status, json: { operator, visible_orgs: 1 } });
    if (path === '/api/admin/vendor-activity' && !operator) {
      return route.fulfill({ status: 403, json: { error: { message: 'operator required' } } });
    }
    return route.fulfill({ json: path === '/api/branding' ? {} : { orgs: [], vendors: [], slots: [] } });
  });
}

for (const name of peering) {
  test(`${name}: menus start closed and dismiss by keyboard and outside click`, async ({ page }) => {
    await mockIdentity(page, true);
    await page.goto(`/peering/${name}`);
    const groups = page.locator('header nav details.nav-group');
    await expect(page.locator('header nav details[open]')).toHaveCount(0);
    const first = groups.first();
    await first.locator('summary').click();
    await expect(first).toHaveAttribute('open', '');
    await first.locator('a').first().focus();
    await page.keyboard.press('Escape');
    await expect(first).not.toHaveAttribute('open');
    await expect(first.locator('summary')).toBeFocused();
    await first.locator('summary').click();
    await page.locator('main').click({ position: { x: 5, y: 5 } });
    await expect(first).not.toHaveAttribute('open');
    await first.locator('summary').click();
    await groups.nth(1).locator('summary').click();
    await expect(first).not.toHaveAttribute('open');
  });
}

for (const path of ['/index.html', '/data-access.html', '/peering/best-practices.html', '/admin/vendor-activity.html']) {
  for (const operator of [false, true]) {
    test(`${path}: navigation respects operator=${operator}`, async ({ page }) => {
      await mockIdentity(page, operator);
      await page.goto(path);
      const admin = page.locator('header nav details').filter({ has: page.locator('summary', { hasText: /^Admin$/ }) });
      if (operator) await expect(admin).toBeVisible();
      else await expect(admin).toBeHidden();
      await expect(page.locator('header nav a[href="/data-access.html"]')).toHaveCount(1);
      if (path === '/index.html') {
        if (operator) await expect(page.locator('#operator-actions')).toBeVisible();
        else await expect(page.locator('#operator-actions')).toBeHidden();
      }
    });
  }
}

test('failed identity lookup keeps operator controls hidden', async ({ page }) => {
  await mockIdentity(page, true, 503);
  await page.goto('/index.html');
  await expect(page.locator('#who')).toContainText('portal is having trouble');
  await expect(page.locator('#operator-actions')).toBeHidden();
  await expect(page.locator('header nav summary').filter({ hasText: /^Admin$/ })).toBeHidden();
});

test('missing credentials never reveal operator controls', async ({ page }) => {
  await page.goto('/index.html');
  await expect(page.locator('#operator-actions')).toBeHidden();
  await expect(page.locator('header nav summary').filter({ hasText: /^Admin$/ })).toBeHidden();
});

test('Best Practices theme picker changes the displayed theme', async ({ page }) => {
  await page.goto('/peering/best-practices.html');
  await page.getByRole('button', { name: 'Dark', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.getByRole('button', { name: 'Light', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
});

for (const slowResponse of ['identity', 'activity']) {
  for (const operator of [false, true]) {
    test(`Activity recovery survives slow ${slowResponse}, operator=${operator}`, async ({ page }) => {
      await page.addInitScript(() => {
        if (!localStorage.getItem('portalApiKey')) localStorage.setItem('portalApiKey', 'activity-test');
      });
      let identityCalls = 0;
      let replacement = false;
      await page.route('**/api/**', async route => {
        const path = new URL(route.request().url()).pathname;
        if (path === '/api/browser-session') {
          replacement = route.request().headers().authorization === 'Bearer replacement-test';
          return route.fulfill({status: replacement ? 204 : 401});
        }
        if (path === '/api/whoami') {
          identityCalls++;
          if (slowResponse === 'identity') await new Promise(resolve => setTimeout(resolve, 200));
          return route.fulfill({ json: { operator: operator || replacement, visible_orgs: 1 } });
        }
        if (path === '/api/admin/vendor-activity') {
          if (slowResponse === 'activity') await new Promise(resolve => setTimeout(resolve, 200));
          return route.fulfill(operator || replacement
            ? { json: { vendors: [] } }
            : { status: 403, json: { error: { message: 'operator required' } } });
        }
        return route.fulfill({ json: {} });
      });
      const activity = page.waitForResponse('**/api/admin/vendor-activity');
      await page.goto('/admin/vendor-activity.html');
      await activity;
      await expect(page.locator('#who')).toContainText('visible');
      if (operator) {
        await expect(page.locator('#auth-banner')).toBeHidden();
        await expect(page.locator('#content')).toBeVisible();
      } else {
        await expect(page.locator('#auth-banner')).toBeHidden();
        await expect(page.locator('#content')).toBeVisible();
        await expect(page.locator('#loading')).toContainText('operator required');
        await expect(page.getByRole('alert').filter({hasText: 'operator required'})).toHaveCount(1);
        await expect(page.locator('#request-error')).toHaveCount(0);
        expect(await page.evaluate(() => localStorage.getItem('portalApiKey'))).toBeNull();
      }
      expect(identityCalls).toBe(1);
      if (!operator) {
        await page.evaluate(async () => { const {setKey} = await import('/app.js'); await setKey('replacement-test'); });
        await page.reload();
        await expect(page.locator('#auth-banner')).toBeHidden();
        await expect(page.locator('#loading')).toBeHidden();
        await expect(page.locator('#content')).toBeVisible();
        expect(identityCalls).toBe(2);
      }
    });
  }
}

for (const failure of ['rejected', 'network']) {
  test(`Activity preserves recovery when identity is ${failure}`, async ({ page }) => {
    await page.addInitScript(() => localStorage.setItem('portalApiKey', 'activity-test'));
    let activityCalls = 0;
    await page.route('**/api/**', route => {
      const path = new URL(route.request().url()).pathname;
      if (path === '/api/whoami') {
        return failure === 'network' ? route.abort('failed')
          : route.fulfill({ status: 401, json: { error: { message: 'invalid key' } } });
      }
      if (path === '/api/admin/vendor-activity') activityCalls++;
      return route.fulfill({ json: {} });
    });
    await page.goto('/admin/vendor-activity.html');
    await expect(page.locator('#who')).toContainText(failure === 'network' ? 'Connection failed' : 'unauthenticated');
    if (failure === 'network') await expect(page.locator('#auth-banner')).toBeHidden();
    else await expect(page.locator('#auth-banner')).toBeVisible();
    await expect(page.locator('#content')).toBeHidden();
    expect(activityCalls).toBe(0);
  });
}
