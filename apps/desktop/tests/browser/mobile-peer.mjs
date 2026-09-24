// Test-only mobile peer for the Rust native account integration. No real SMTP.
import { chromium, expect } from '@playwright/test';
import { createServer } from 'node:net';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
import { createWebServer } from '../../../mobile-web/server/server.mjs';
const upstream = process.env.QIBAN_TEST_UPSTREAM;
assert.match(upstream ?? '', /^http:\/\/127\.0\.0\.1:\d+$/);
console.log('Native login completed; preserving the real 60-second OTP cooldown before independent mobile login.');
await new Promise(resolve => setTimeout(resolve, 61000));
const port = await new Promise(resolve => { const probe = createServer(); probe.listen(0, '127.0.0.1', () => { const port = probe.address().port; probe.close(() => resolve(port)); }); });
const origin = `http://127.0.0.1:${port}`;
const server = createWebServer({ origin, upstream, dist: fileURLToPath(new URL('../../../mobile-web/dist', import.meta.url)) });
await new Promise(resolve => server.listen(port, '127.0.0.1', resolve));
const browser = await chromium.launch({ channel: 'msedge', headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  await page.goto(origin);
  await page.getByLabel('邮箱', { exact: true }).fill('alice@example.com');
  await page.getByRole('button', { name: '获取验证码', exact: true }).click();
  await expect(page.getByLabel('8 位验证码')).toBeVisible();
  const { code } = await (await fetch(`${upstream}/__test/code`)).json();
  await page.getByLabel('8 位验证码').fill(code);
  await page.getByRole('button', { name: '与栖栖会合', exact: true }).click();
  await expect(page.getByRole('heading', { name: /我们的待办/ })).toBeVisible();
  await page.locator('summary').click();
  await expect(page.getByTestId('account-id')).toHaveText(process.env.QIBAN_EXPECT_ACCOUNT);
  await expect(page.getByTestId('companion-id')).toHaveText(process.env.QIBAN_EXPECT_COMPANION);
  await expect(page.locator('.task-list')).toContainText('来自原生桌面的共享待办');
  await page.getByLabel('想让栖栖记住什么？').fill('来自手机的共享待办');
  await page.getByRole('button', { name: '记下来', exact: true }).click();
  await expect(page.locator('.task-list li')).toHaveCount(2);
  await page.getByRole('button', { name: '取消 来自原生桌面的共享待办', exact: true }).click();
  await expect(page.locator('.cancelled')).toContainText('来自原生桌面的共享待办');
  console.log('PASS: independent mobile login matches native account/companion, reads native task, creates task and cancels native task.');
} finally { await browser.close(); server.close(); server.closeAllConnections(); }
