// 无头渲染背景文章页 PNG（别用 Edge 有头窗口做背景：引号会被吞、会开进用户会话）
import { chromium } from 'playwright';
import { pathToFileURL } from 'node:url';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const out = resolve(here, 'bg.png');

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 } });
await page.goto(pathToFileURL(resolve(here, 'article.html')).href);
await page.waitForTimeout(600);
await page.screenshot({ path: out, clip: { x: 0, y: 0, width: 1920, height: 1080 } });
await browser.close();
console.log('written: ' + out);
