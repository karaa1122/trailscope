// Captures UI screenshots from a running Logbox with demo data.
//   BASE_URL=http://localhost:8080 OUT=../docs/images node screenshots.mjs
import { chromium } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';

const BASE = process.env.BASE_URL ?? 'http://localhost:8080';
const OUT = process.env.OUT ?? '../docs/images';
const EMAIL = process.env.LOGBOX_EMAIL ?? 'owner@example.com';
const PASSWORD = process.env.LOGBOX_PASSWORD ?? 'correct horse battery';
const THEME = process.env.THEME ?? 'dark';

mkdirSync(OUT, { recursive: true });
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 1200 }, deviceScaleFactor: 1 });
page.on('console', (m) => m.type() === 'error' && console.error('[browser]', m.text()));
page.on('pageerror', (e) => console.error('[pageerror]', e.message));

await page.goto(`${BASE}/login`);
await page.evaluate((t) => localStorage.setItem('logbox.theme', t), THEME);
await page.getByLabel('Email').fill(EMAIL);
await page.getByLabel('Password').fill(PASSWORD);
await page.getByRole('button', { name: 'Sign in' }).click();
await page.waitForURL('**/live**');

const shot = async (name) => {
  await page.waitForTimeout(2500);
  await page.screenshot({ path: join(OUT, `${name}.png`), fullPage: true });
  console.log('saved', name);
};

await page.waitForSelector('text=Route heatmap');
await shot('live-tail');

await page.goto(`${BASE}/explore?mode=search&q=status%3A5xx&range=6h`);
await page.waitForSelector('text=Requests');
await shot('explore');

const row = page.locator('.table-row').first();
await row.click();
await page.waitForURL('**/requests/**');
await shot('inspector');

await page.goto(`${BASE}/explore?view=group&group=route&range=6h`);
await shot('explore-group');

await page.setViewportSize({ width: 390, height: 900 });
await page.goto(`${BASE}/live`);
await shot('live-tail-mobile');

await browser.close();
