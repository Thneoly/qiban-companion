// phone-shot6.mjs - standalone retake of shot 6 (phone continuation, B viewport).
// Take B filmed this beat with the phone window invisible (DWM-cloaked/offscreen after the
// executablePath launch), so this mini-session re-records ONLY shot 6 with checkpointed
// file signals so the operator can probe visibility between phases.
// Signals (files in this dir): p6-ready.txt (browser up, form on screen) -> wait p6-visible.txt
// (operator confirmed/corrected visibility) -> wait p6-roll.txt (recorder started) -> pair on film
// -> p6-done.txt -> exit.
import { chromium } from 'playwright';
import { writeFileSync, existsSync, unlinkSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const sleep = ms => new Promise(r => setTimeout(r, ms));
const waitFor = async f => { while (!existsSync(resolve(HERE, f))) await sleep(300); };
const sig = f => writeFileSync(resolve(HERE, f), String(Date.now()));

for (const f of ['p6-ready.txt', 'p6-done.txt']) { try { unlinkSync(resolve(HERE, f)); } catch {} }

async function cdpPanel(expr) {
  const list = await (await fetch('http://127.0.0.1:9222/json')).json();
  const page = list.find(t => t.url.includes('view=panel'));
  if (!page) throw new Error('panel page not found');
  const { default: WebSocket } = await import('ws');
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise(r => ws.on('open', r));
  const call = (method, params) => new Promise((res, rej) => {
    const id = Math.floor(Math.random() * 1e6);
    const onMsg = raw => { const m = JSON.parse(raw.toString()); if (m.id === id) { ws.off('message', onMsg); m.error ? rej(new Error(m.error.message)) : res(m.result); } };
    ws.on('message', onMsg);
    ws.send(JSON.stringify({ id, method, params }));
  });
  const r = await call('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
  ws.close();
  return r.result.value;
}

const ctx = await chromium.launchPersistentContext(resolve(HERE, 'phone-profile2'), {
  headless: false, viewport: { width: 400, height: 800 },
  args: ['--window-position=560,120', '--window-size=416,880', '--start-maximized=false'],
  executablePath: process.env.LOCALAPPDATA + '\\ms-playwright\\chromium-1243\\chrome-win64\\chrome.exe',
});
const page = ctx.pages()[0] ?? await ctx.newPage();
await page.goto('http://127.0.0.1:4318/', { waitUntil: 'networkidle', timeout: 15000 });
await sleep(800);
console.log('form:', (await page.evaluate(() => document.body.innerText.slice(0, 120))).replace(/\n/g, '|'));
sig('p6-ready.txt');

await waitFor('p6-visible.txt');
await waitFor('p6-roll.txt');
console.log('rolling: generating code');

// navigate panel to 设备 (persisted view may differ) then generate + read code
await cdpPanel(`(function(){var a=[...document.querySelectorAll('a.nav-link')].find(e=>e.textContent.trim()==='设备');if(a)a.click();return 'nav'})()`);
await sleep(1800);
const code = await cdpPanel(`(async()=>{var b=[...document.querySelectorAll('button')].find(e=>e.textContent.trim()==='生成五分钟配对码');if(!b)return 'NOBTN';b.click();await new Promise(r=>setTimeout(r,2200));var m=document.body.innerText.match(/(\\d{6})\\s*\\n\\s*有效至/);return m?m[1]:'NOCODE';})()`);
console.log('code:', code);
if (!/^\d{6}$/.test(String(code))) throw new Error('code failed: ' + code);

// on-film typing (pressSequentially = visible per-char)
await sleep(1500);
const inputs = page.locator('input');
await inputs.nth(0).pressSequentially('我的手机', { delay: 140 });
await sleep(500);
await inputs.nth(1).pressSequentially(String(code), { delay: 130 });
await sleep(700);
await page.getByRole('button', { name: /与电脑配对/ }).click();
for (let i = 0; i < 10; i++) {
  await sleep(1000);
  const t = await page.evaluate(() => document.body.innerText.slice(0, 260));
  if (t.includes('已接续')) { console.log('paired at', i + 1, 's'); break; }
}
await sleep(9500); // hold the paired view on film
sig('p6-done.txt');
await ctx.close();
process.exit(0);
