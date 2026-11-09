// Captures UI screenshots from a running Trailscope with demo data (live traffic
// from `trailscope demo` makes Live tail and the heatmap come alive).
//   BASE_URL=http://localhost:8080 OUT=../docs/images node screenshots.mjs
// Optional: PROJECT_ID (project to show), THEME (dark|light), CHROMIUM_PATH,
// JOURNEY (key:value) and JOURNEY_RANGE, TRAILSCOPE_EMAIL / TRAILSCOPE_PASSWORD.
import { chromium } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';

const BASE = process.env.BASE_URL ?? 'http://localhost:8080';
const OUT = process.env.OUT ?? '../docs/images';
const EMAIL = process.env.TRAILSCOPE_EMAIL ?? 'owner@example.com';
const PASSWORD = process.env.TRAILSCOPE_PASSWORD ?? 'correct horse battery';
const THEME = process.env.THEME ?? 'dark';
const PROJECT = process.env.PROJECT_ID;

mkdirSync(OUT, { recursive: true });
const browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {});
const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 });
page.on('console', (m) => m.type() === 'error' && console.error('[browser]', m.text()));
page.on('pageerror', (e) => console.error('[pageerror]', e.message));

await page.goto(`${BASE}/login`);
await page.evaluate(
  ([theme, project]) => {
    localStorage.setItem('trailscope.theme', theme);
    if (project) localStorage.setItem('trailscope.project', project);
  },
  [THEME, PROJECT],
);
await page.getByLabel('Email').fill(EMAIL);
await page.getByLabel('Password').fill(PASSWORD);
await page.getByRole('button', { name: 'Sign in', exact: true }).click();
await page.waitForURL('**/live**');
if (PROJECT) {
  // The session reads the stored project when the app loads.
  await page.evaluate((p) => localStorage.setItem('trailscope.project', p), PROJECT);
  await page.goto(`${BASE}/live`);
}

const shot = async (name, { full = false } = {}) => {
  await page.waitForTimeout(2500);
  await page.screenshot({ path: join(OUT, `${name}.png`), fullPage: full });
  console.log('saved', name);
};

await page.waitForSelector('text=Route heatmap');
await shot('live-tail');

await page.goto(`${BASE}/explore?mode=search&q=status%3A5xx&range=6h`);
await page.waitForSelector('text=Requests');
await shot('explore');

await page.locator('.table-row').first().click();
await page.waitForURL('**/requests/**');
await shot('inspector', { full: true });

await page.goto(`${BASE}/explore?view=group&group=route&range=6h`);
await shot('explore-group');

await page.goto(`${BASE}/errors?range=24h`);
await shot('errors');

await page.goto(`${BASE}/routes`);
await shot('routes');

const journey = process.env.JOURNEY ?? 'account_id:acc_108';
await page.goto(`${BASE}/journeys?q=${encodeURIComponent(journey)}&range=${process.env.JOURNEY_RANGE ?? '1h'}`);
await shot('journey');

await page.goto(`${BASE}/alerts?tab=email`);
await shot('alerts-email');

await page.setViewportSize({ width: 390, height: 844 });
await page.goto(`${BASE}/live`);
await shot('live-tail-mobile');

await browser.close();
