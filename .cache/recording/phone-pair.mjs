// phone-pair.mjs - pair a headless phone page and dump the continuation view (PRIVATE).
import { chromium } from 'playwright';

const code = process.argv[2];
if (!code || !/^\d{6}$/.test(code)) { console.error('need 6-digit code'); process.exit(1); }

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 390, height: 780 } });
await page.goto('http://127.0.0.1:4318/', { waitUntil: 'networkidle', timeout: 15000 });
await page.waitForTimeout(800);
// device name input then code input
const nameInput = page.locator('input').first();
await nameInput.fill(String.fromCharCode(0x8BD5, 0x7528, 0x624B, 0x673A)); // 试用手机
const inputs = page.locator('input');
const n = await inputs.count();
let codeInput = null;
for (let i = 0; i < n; i++) {
  const el = inputs.nth(i);
  const ph = await el.getAttribute('placeholder') ?? '';
  const type = await el.getAttribute('type') ?? '';
  const val = await el.inputValue();
  if (el !== nameInput || val === '') codeInput = codeInput ?? el;
}
// second input is the code field in practice
const codeEl = inputs.nth(1);
await codeEl.fill(code);
await page.getByRole('button', { name: /与电脑配对/ }).click();
await page.waitForTimeout(3000);
console.log('--- AFTER PAIR TEXT ---');
console.log((await page.evaluate(() => document.body.innerText.slice(0, 1500))));
const shot = new URL('./phone-paired.png', import.meta.url).pathname.replace(/^\//, '');
await page.screenshot({ path: shot, fullPage: false });
console.log('screenshot: ' + shot);
await browser.close();
