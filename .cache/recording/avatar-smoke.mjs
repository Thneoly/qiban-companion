// avatar-smoke.mjs - de-risk shot 2B (avatar switch) BEFORE the recording session.
// Probes, in order:
//   A. panel reachability via CDP eval (bubble -> 更多 -> 角色与场景)
//   B. Playwright connectOverCDP setInputFiles on the webkitdirectory input:
//      does webkitRelativePath survive? does importModel succeed? does Hiyori render?
//      then 移除本机模型 -> back to SVG.
//   C. native folder dialog: real-click the label, report dialog hwnd/rect/foreground
//      (driving + z-order vs topmost wallpaper is probed in a separate PS step).
// Usage: node avatar-smoke.mjs <phase>   (phases: panel | inject | dialog)
import { readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

const HERE = new URL('.', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');
const MODEL_DIR = 'D:/Live2D/Hiyori';
const phase = process.argv[2];

function walk(dir, base = '') {
  const out = [];
  for (const e of readdirSync(dir)) {
    const p = join(dir, e).replace(/\\/g, '/');
    if (statSync(p).isDirectory()) out.push(...walk(p, base + e + '/'));
    else out.push({ path: p, rel: base + e });
  }
  return out;
}

async function cdpEval(expr) {
  const list = await (await fetch('http://127.0.0.1:9222/json')).json();
  const page = list.find(t => t.type === 'page' && t.url === 'http://tauri.localhost/');
  if (!page) throw new Error('pet page not found');
  const { default: WebSocket } = await import('ws');
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise(r => ws.on('open', r));
  const call = (method, params) => new Promise((res, rej) => {
    const id = Math.floor(Math.random() * 1e6);
    const onMsg = raw => { const m = JSON.parse(raw.toString()); if (m.id === id) { ws.off('message', onMsg); m.error ? rej(new Error(m.error.message)) : res(m.result); } };
    ws.on('message', onMsg);
    ws.send(JSON.stringify({ id, method, params }));
  });
  await call('Runtime.enable');
  const r = await call('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
  ws.close();
  if (r.exceptionDetails) throw new Error('eval: ' + (r.exceptionDetails.exception?.description ?? r.exceptionDetails.text));
  return r.result.value;
}

const sleep = ms => new Promise(r => setTimeout(r, ms));

async function openPanel() {
  // bubble (skip if appearance panel already open)
  let s = await cdpEval(`(function(){var p=document.querySelector('.appearance-panel');return p?'PANEL_ALREADY':(document.querySelector('button.pet-character')?.getAttribute('aria-expanded'))})()`);
  console.log('pre-state:', s);
  if (s !== 'PANEL_ALREADY') {
    if (s !== 'true') { await cdpEval(`document.querySelector('button.pet-character').click();'opened'`); await sleep(1200); }
    await cdpEval(`var d=document.querySelector('details.pet-more');if(d&&!d.open){d.querySelector('summary').click()}'more'`); await sleep(700);
    const r = await cdpEval(`(function(){var b=[...document.querySelectorAll('button')].find(x=>x.textContent.trim()==='角色与场景');if(!b)return 'NOBTN';if(b.disabled)return 'DISABLED';b.click();return 'clicked'})()`);
    console.log('角色与场景 click:', r);
    if (r !== 'clicked' && r !== 'PANEL_ALREADY') throw new Error('panel open failed: ' + r);
    await sleep(1000);
  }
  const st = await cdpEval(`(function(){var i=document.querySelector('input[aria-label="导入模型文件夹"]');return {panel:!!document.querySelector('.appearance-panel'),input:!!i,modelP:document.querySelector('.appearance-panel fieldset p')?.textContent}})()`);
  console.log('panel state:', JSON.stringify(st));
  return st;
}

if (phase === 'panel') {
  await openPanel();
  process.exit(0);
}

if (phase === 'inject') {
  await openPanel();
  const { chromium } = await import('playwright');
  const browser = await chromium.connectOverCDP('http://127.0.0.1:9222');
  const ctx = browser.contexts()[0];
  const page = ctx.pages().find(p => p.url() === 'http://tauri.localhost/');
  if (!page) throw new Error('page not found over CDP');
  const files = walk(MODEL_DIR);
  console.log(`injecting ${files.length} files, sample rel:`, files.map(f => f.rel).slice(0, 4));
  const input = page.locator('input[aria-label="导入模型文件夹"]');
  await input.setInputFiles(MODEL_DIR);
  await sleep(800);
  // what did the input actually receive?
  const rel = await page.evaluate(() => [...document.querySelector('input[aria-label="导入模型文件夹"]').files].slice(0, 4).map(f => f.webkitRelativePath || '(' + f.name + ')'));
  console.log('input received relativePaths:', JSON.stringify(rel));
  // importModel runs on the change event; watch busy + result
  for (let i = 0; i < 20; i++) {
    await sleep(1000);
    const st = await page.evaluate(`(function(){return {busyLabel:document.querySelector('label.model-import')?.textContent.trim(),modelP:document.querySelector('.appearance-panel fieldset p')?.textContent,err:document.querySelector('.pet-error')?.textContent||'',svgPressed:document.querySelector('.appearance-panel button[aria-pressed="true"]')?.textContent,canvas:!!document.querySelector('canvas')}})()`);
    console.log(`t+${i + 1}s`, JSON.stringify(st));
    if (!String(st.busyLabel).includes('正在保存')) break;
  }
  await browser.close();
  process.exit(0);
}

if (phase === 'dialog') {
  // real-click at the label: report its screen coords for input.ps1
  const r = await cdpEval(`(function(){var l=document.querySelector('label.model-import');if(!l)return 'NOLABEL';var b=l.getBoundingClientRect();return JSON.stringify({x:b.x+b.width/2,y:b.y+b.height/2,w:b.width,h:b.height,text:l.textContent.trim()})})()`);
  console.log('label rect (window coords):', r);
  process.exit(0);
}

console.error('unknown phase'); process.exit(1);
