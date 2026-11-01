// inject-2b.mjs - CDP directory injection for shot 2B import (fallback path per runbook:
// native picker commit stopped delivering files on 2026-11-01 afternoon sessions).
// Usage: node inject-2b.mjs          -> inject D:/Live2D/Hiyori into the webkitdirectory
//                                     input on the pet page, poll until 2D 数字人 is selected.
// Expects the appearance panel already open (take-driver opens it on film).
import { chromium } from 'playwright';

const MODEL_DIR = 'D:/Live2D/Hiyori';
const sleep = ms => new Promise(r => setTimeout(r, ms));

const browser = await chromium.connectOverCDP('http://127.0.0.1:9222');
const ctx = browser.contexts().find(c => c.pages().some(p => p.url() === 'http://tauri.localhost/'));
if (!ctx) { console.error('NO_PET_CONTEXT'); process.exit(1); }
const page = ctx.pages().find(p => p.url() === 'http://tauri.localhost/');

const input = page.locator('input[aria-label="导入模型文件夹"]');
await input.setInputFiles(MODEL_DIR); // webkitdirectory: pass the directory itself
console.log('INJECTED');

for (let i = 0; i < 20; i++) {
  await sleep(1000);
  const state = await page.evaluate(() => ({
    pressed: document.querySelector('.appearance-panel button[aria-pressed=true]')?.textContent ?? 'none',
    remove: !![...document.querySelectorAll('.appearance-panel button')].find(b => b.textContent.trim() === '移除本机模型'),
    hint: (document.querySelector('.appearance-panel')?.innerText ?? '').slice(0, 120)
  }));
  console.log(`[${i + 1}s]`, JSON.stringify(state));
  if (String(state.pressed).includes('2D 数字人') && state.remove) { console.log('IMPORT_OK'); process.exit(0); }
}
console.error('IMPORT_TIMEOUT');
process.exit(1);
