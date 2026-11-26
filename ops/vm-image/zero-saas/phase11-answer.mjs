// phase11-answer.mjs — LAB ONLY. Stands in for the person at a goatlite's phase 11
// in the zero-SaaS lab run (ops/vm-image/zero-saas/README.md). It is not part of
// goatbuilder, no product path calls it, and it must not become one: the product's
// phase-11 step stays a person's (goatlite.md §5 row 6).
//
// It does what tutorial 00 section 5 "The browser step" tells the learner to do,
// against the box's own dashboard and management API only:
//   login   sign in at the dashboard with the one-time password (creates the account)
//   token   sign in again, create the service user svc-<name>-netbird-infra (admin) and
//           a token on it, and write the token to OUT_FILE (mode 0600). The token is
//           never printed.
// Between the two, the caller re-runs phase 10 once, which promotes the account.
//
// Env: DASHBOARD_URL, MGMT_URL, ADMIN_EMAIL, ADMIN_PASSWORD_FILE, SERVICE_USER,
//      OUT_FILE (token mode), E2E_DIR (the Playwright install, ops/goat-id-console/web/e2e).
// The goatnet's certificates are its own CA's; the learner accepts the browser's
// warning, and so does this (ignoreHTTPSErrors).
import { createRequire } from 'node:module';
import { readFileSync, writeFileSync } from 'node:fs';

const need = (n) => {
  const v = process.env[n];
  if (!v) { console.error(`phase11-answer: ${n} is not set`); process.exit(2); }
  return v;
};
const mode = process.argv[2];
if (mode !== 'login' && mode !== 'token') {
  console.error('usage: node phase11-answer.mjs login|token');
  process.exit(2);
}
const require = createRequire(`${need('E2E_DIR')}/package.json`);
const { chromium } = require('@playwright/test');

const DASHBOARD = need('DASHBOARD_URL').replace(/\/$/, '');
const MGMT = need('MGMT_URL').replace(/\/$/, '');
const EMAIL = need('ADMIN_EMAIL');
const PASSWORD = readFileSync(need('ADMIN_PASSWORD_FILE'), 'utf8').trim();

const browser = await chromium.launch({ args: ['--ignore-certificate-errors'] });
const ctx = await browser.newContext({ ignoreHTTPSErrors: true });
const page = await ctx.newPage();
let code = 1;
let token = '';
try {
  await page.goto(`${DASHBOARD}/`, { waitUntil: 'domcontentloaded', timeout: 60000 });
  await page.waitForSelector('input[name="identifier"]', { timeout: 60000 });
  await page.fill('input[name="identifier"]', EMAIL);
  await page.fill('input[name="password"]', PASSWORD);
  await page.click('button[type="submit"]');
  token = await page.waitForFunction(() => {
    try { return JSON.parse(sessionStorage.getItem('oidc.default') || '{}').tokens?.accessToken || false; }
    catch { return false; }
  }, null, { timeout: 90000 }).then((h) => h.jsonValue());
  console.log(`phase11-answer: signed in as ${EMAIL} at ${DASHBOARD}`);
  const api = async (method, path, body) => {
    const r = await ctx.request.fetch(`${MGMT}${path}`, {
      method, data: body, ignoreHTTPSErrors: true,
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    });
    if (!r.ok()) throw new Error(`${method} ${path}: HTTP ${r.status()} ${(await r.text()).slice(0, 300)}`);
    return r.json();
  };
  // The management service makes the person's user row on their first API call,
  // which the dashboard may not have made yet; phase 10's re-run promotes that row.
  const me = await api('GET', '/api/users/current');
  console.log(`phase11-answer: the management service knows ${EMAIL} (role ${me.role})`);
  if (mode === 'token') {
    const name = need('SERVICE_USER');
    const users = await api('GET', '/api/users?service_user=true');
    let user = users.find((u) => u.name === name);
    if (!user) user = await api('POST', '/api/users', { name, role: 'admin', is_service_user: true, auto_groups: [] });
    const t = await api('POST', `/api/users/${user.id}/tokens`, { name: `${name}-lab`, expires_in: 365 });
    writeFileSync(need('OUT_FILE'), t.plain_token, { mode: 0o600 });
    console.log(`phase11-answer: service user ${name} has a new token (written, not printed)`);
  }
  code = 0;
} catch (e) {
  // The first line only, with the known secrets blanked: Playwright's call log
  // can quote a typed value or a request's headers.
  let msg = String(e.message || e).split('\n')[0];
  for (const [secret, mark] of [[PASSWORD, '<one-time password>'], [token, '<access token>']]) {
    if (secret) msg = msg.split(secret).join(mark);
  }
  console.error(`phase11-answer: ${msg}`);
  const at = new URL(page.url());
  console.error(`phase11-answer: the page was ${at.origin}${at.pathname}`);
} finally {
  await browser.close();
}
process.exit(code);
