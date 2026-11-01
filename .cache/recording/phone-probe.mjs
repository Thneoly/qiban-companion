// phone-probe.mjs - headless probe of the phone continuation page (PRIVATE inspection only).
import { chromium } from 'playwright';

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 390, height: 780 } });
await page.goto('http://127.0.0.1:4318/', { waitUntil: 'networkidle', timeout: 15000 }).catch(e => console.log('goto: ' + e.message));
await page.waitForTimeout(1200);
const text = await page.evaluate(() => document.body.innerText.slice(0, 1200));
console.log('--- PHONE PAGE TEXT ---');
console.log(text);
await page.screenshot({ path: new URL('./phone-probe.png', import.meta.url).pathname.replace(/^\//, '') });
await browser.close();
