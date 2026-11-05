import { test, expect } from '@playwright/test';

test('sign in exchanges the key for an HttpOnly session across navigation and sign out', async ({ page }) => {
  const key = 'synthetic-session-fixture-key';
  const session = 'opaque-browser-session-fixture';
  await page.route('**/api/**', async route => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (path === '/api/browser-session') {
      if (request.method() === 'DELETE') {
        return route.fulfill({status: 204, headers: {'Set-Cookie': 'goat_portal_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0'}});
      }
      expect(request.headers().authorization).toBe('Bearer ' + key);
      return route.fulfill({status: 204, headers: {'Set-Cookie': 'goat_portal_session=' + session + '; Path=/; HttpOnly; SameSite=Strict'}});
    }
    if (path === '/api/whoami') {
      const headers = await request.allHeaders();
      expect(headers.authorization).toBeUndefined();
      return (headers.cookie || '').includes(session)
        ? route.fulfill({json: {operator: true, visible_orgs: 1}})
        : route.fulfill({status: 401, json: {error: {message: 'missing session'}}});
    }
    return route.fulfill({json: {orgs: [], vendors: [], slots: []}});
  });
  await page.goto('/index.html');
  await expect(page.locator('#auth-banner')).toBeVisible();
  await page.locator('#api-key-input').fill(key);
  await page.locator('#api-key-save').click();
  await expect(page.locator('#who')).toContainText('visible');
  expect(await page.evaluate(() => localStorage.getItem('portalApiKey'))).toBeNull();
  expect(await page.evaluate(() => sessionStorage.length)).toBe(0);
  expect(await page.evaluate(() => document.cookie)).not.toContain(session);
  await page.goto('/progress.html');
  await expect(page.locator('#who')).toContainText('visible');
  expect((await page.context().cookies()).find(cookie => cookie.name === 'goat_portal_session')?.httpOnly).toBe(true);
  await page.evaluate(async () => { const {clearKey} = await import('/app.js'); await clearKey(); });
  await page.reload();
  await expect(page.locator('#auth-banner')).toBeVisible();
});
