// rework-smoke.mjs - TUWE-5 v3 rework smoke (2026-11-01 PM, PM 15:09 order):
// per-shot must-appear composition probes for shots 2B / 3 / 4 / 6 before asking
// Thne to green-light the recording session.
//   2B: Hiyori FULL BODY in frame, not occluded by the appearance panel
//       (new beat: click 返回互动 + collapse bubble, film the bare pet window).
//   3 : wide chat window (760x720) with the input box visible as the subject.
//   4 : compact-mode voice turn composition - pet character visible WITH the
//       bubble/mic (wide mode hides .pet-character per pet.css:138 - v2 defect).
//   6 : phone window form standing in frame (--app mode: no address bar, so the
//       full window can be filmed without leaking the pairing URL) AND the DDA
//       render check: short ddagrab run + gdigrab probe DURING it + frame from
//       the ddagrab output itself.
// Probes are gdigrab single frames cropped to the session's 1920x1032 region.
// Run: node rework-smoke.mjs   (expects .cache/recording deps installed)
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { mkdirSync } from 'node:fs';
import { chromium } from 'playwright';

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = resolve(HERE, 'out');
mkdirSync(OUT, { recursive: true });
const T0 = Date.now();
const log = (...a) => console.log(`[${((Date.now() - T0) / 1000).toFixed(1)}]`, ...a);
const sleep = ms => new Promise(r => setTimeout(r, ms));

function run(cmd, args, opts = {}) {
  return new Promise((res, rej) => {
    const p = spawn(cmd, args, { stdio: ['ignore', 'pipe', 'pipe'], ...opts });
    let out = '', err = '';
    p.stdout.on('data', d => out += d);
    p.stderr.on('data', d => err += d);
    p.on('close', code => code === 0 ? res(out.trim()) : rej(new Error(`${cmd} exit ${code}: ${err.trim().slice(0, 300)}`)));
    p.on('error', rej);
  });
}
const node = (args) => run('node', args);
const cdp = (js) => node([resolve(HERE, 'cdp-eval.mjs'), '=http://tauri.localhost/', js]).then(s => s.replace(/^"|"$/g, '').replace(/\\"/g, '"'));
const ps = (file, args) => run('powershell', ['-NoProfile', '-File', resolve(HERE, file), ...args]);

// gdigrab single-frame probe: PRIMARY monitor only (video_size+offset), then crop
// to the session's 1920x1032 region. v1 lesson: without video_size gdigrab returns
// the 3840-wide virtual-desktop stitch and a crop at x=0 lands on the LEFT monitor.
// 30s kill timer: a hung capture must not wedge the smoke.
function probe(name) {
  return new Promise(res => {
    const file = resolve(OUT, `probe-${name}.png`);
    const p = spawn('ffmpeg', ['-y', '-hide_banner', '-loglevel', 'error',
      '-f', 'gdigrab', '-framerate', '1', '-offset_x', '0', '-offset_y', '0', '-video_size', '1920x1080', '-i', 'desktop',
      '-frames:v', '1', '-vf', 'crop=1920:1032:0:0', file],
      { stdio: ['ignore', 'ignore', 'pipe'] });
    let err = '';
    p.stderr.on('data', d => err += d);
    const timer = setTimeout(() => { try { p.kill(); } catch {} res(`PROBE_TIMEOUT ${name}`); }, 30000);
    p.on('close', code => { clearTimeout(timer); res(code === 0 ? `probe-${name}.png` : `PROBE_FAIL ${name}: ${err.slice(0, 160)}`); });
    p.on('error', e => { clearTimeout(timer); res(`PROBE_FAIL ${name}: ${e.message}`); });
  });
}

let appProc = null, bgProc = null, phoneCtx = null;
async function emergency() {
  try { await phoneCtx?.close(); } catch {}
  try { appProc?.kill(); } catch {}
  try { bgProc?.kill(); } catch {}
  try { await ps('stray-windows.ps1', ['-Action', 'restore']); } catch {}
  log('TEARDOWN_DONE (emergency)');
}

async function main() {
  // ---- launch app with CDP ----
  log('launching companion-desktop with CDP 9222');
  appProc = spawn('D:/Game/target/release/companion-desktop.exe', [], {
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=9222' },
    stdio: 'ignore',
  });
  let petUp = false;
  for (let i = 0; i < 30; i++) {
    await sleep(1000);
    try {
      const list = await (await fetch('http://127.0.0.1:9222/json')).json();
      if (list.some(t => t.type === 'page' && t.url === 'http://tauri.localhost/')) { petUp = true; break; }
    } catch {}
  }
  if (!petUp) throw new Error('pet page never appeared on CDP 9222');
  log('pet page up');
  for (let i = 0; i < 20; i++) {
    const st = await cdp(`(document.querySelector('button.pet-character')?.disabled===false)?'ready':'wait'`);
    if (st.includes('ready')) break;
    await sleep(1000);
  }
  log('pet interactive');

  // pet home + wallpaper + z-order + strays (runbook order: bg first, then pet on top)
  let rect = await ps('petwin.ps1', ['-Action', 'rect']);
  log('pet rect initial:', rect.split('|').pop());
  const rm = /(\d+)x(\d+)@(-?\d+),(-?\d+)/.exec(rect);
  if (!rm) throw new Error('pet window not found');
  if (+rm[3] !== 1450 || +rm[4] !== 592) {
    await ps('petwin.ps1', ['-Action', 'move', '-Which', 'pet', '-X', '1450', '-Y', '592']);
    log('pet moved home 1450,592');
  }
  bgProc = spawn('powershell', ['-NoProfile', '-File', resolve(HERE, 'bgview.ps1'), '-Image', resolve(HERE, 'wallpaper.png'), '-Topmost'], { stdio: 'ignore' });
  await sleep(1800);
  // bgview liveness check (v1 lesson: a dead bg window means probes film the real desktop)
  const bgAlive = await run('powershell', ['-NoProfile', '-Command',
    `(Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" | Where-Object { $_.CommandLine -like '*bgview.ps1*' -and $_.ProcessId -ne $PID } | Measure-Object).Count`]);
  if (String(bgAlive).trim() !== '1') throw new Error(`bgview process count=${bgAlive} - ABORT (probe would film the real desktop)`);
  log('bgview alive (topmost wallpaper)');
  await ps('petwin.ps1', ['-Action', 'top', '-Which', 'pet']).then(s => log('pet top:', s.split('|').pop()));
  await ps('stray-windows.ps1', ['-Action', 'hide']).then(s => log('strays hidden:', s.split('\n').length - 1, 'entries'));
  log('P0:', await probe('p0-baseline'));

  // ---- shot 2B: panel beat -> inject -> 返回互动 -> collapse -> FULL BODY beat ----
  log('2B: open bubble + appearance panel');
  await cdp(`document.querySelector('button.pet-character').click();'open'`);
  await sleep(1100);
  await cdp(`(function(){var d=document.querySelector('details.pet-more');if(d&&!d.open){d.querySelector('summary').click()}return 'more'})()`);
  await sleep(600);
  const rPanel = await cdp(`(function(){var b=[...document.querySelectorAll('button')].find(x=>x.textContent.trim()==='角色与场景');if(!b)return 'NOBTN';b.click();return 'clicked'})()`);
  if (rPanel !== 'clicked') throw new Error('appearance panel: ' + rPanel);
  await sleep(1500);
  log('P2B-panel:', await probe('p2b-panel'));
  log('2B: inject model');
  const rInj = await node([resolve(HERE, 'inject-2b.mjs')]).catch(e => 'INJECT_FAIL ' + e.message);
  log('2B inject:', String(rInj).trim().split('\n').pop());
  await sleep(1200);
  // NEW beat: leave the cramped panel, collapse the bubble -> bare window = Hiyori full body
  log('2B: 返回互动 + 收起气泡 (new full-body beat)');
  await cdp(`(function(){var b=[...document.querySelectorAll('.appearance-panel button')].find(x=>x.textContent.trim()==='返回互动');if(b){b.click();return 'back'}return 'NOBTN'})()`);
  await sleep(900);
  await cdp(`[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')==='收起气泡')?.click();'closed'`);
  await sleep(1600); // Hiyori idle settles
  const rend = await cdp(`document.querySelector('.pet-shell')?.dataset.renderer`);
  log('2B renderer now:', rend);
  log('P2B-fullbody:', await probe('p2b-fullbody'));
  // off-film cleanup: remove imported model, verify renderer back to svg
  await cdp(`document.querySelector('button.pet-character').click();'open'`);
  await sleep(1000);
  await cdp(`(function(){var d=document.querySelector('details.pet-more');if(d&&!d.open){d.querySelector('summary').click()}return 'more'})()`);
  await sleep(500);
  await cdp(`(function(){var b=[...document.querySelectorAll('button')].find(x=>x.textContent.trim()==='角色与场景');if(b)b.click();return 'panel'})()`);
  await sleep(1300);
  await cdp(`(async function(){var b=[...document.querySelectorAll('.appearance-panel button')].find(x=>x.textContent.trim()==='移除本机模型');if(b){b.click();await new Promise(r=>setTimeout(r,2000))}var back=[...document.querySelectorAll('.appearance-panel button')].find(x=>x.textContent.trim()==='返回互动');if(back)back.click();await new Promise(r=>setTimeout(r,700));var x=[...document.querySelectorAll('button')].find(e=>e.getAttribute('aria-label')==='收起气泡');if(x)x.click();return 'cleaned'})()`);
  await sleep(1500);
  log('2B cleanup renderer:', await cdp(`document.querySelector('.pet-shell')?.dataset.renderer`));

  // ---- shot 3: wide chat window, input box as subject ----
  log('S3: bubble -> chat -> 展开阅读 (wide window)');
  await cdp(`document.querySelector('button.pet-character').click();'open'`);
  await sleep(1300);
  await cdp(`[...document.querySelectorAll('.pet-mode > button')][1].click();'chat'`);
  await sleep(1000);
  await cdp(`[...document.querySelectorAll('.chat-state-line button')].find(b=>b.textContent==='展开阅读')?.click()??'NOBTN';'reading'`);
  await sleep(2400); // expand_chat resize settles
  const rectWide = await ps('petwin.ps1', ['-Action', 'rect']);
  log('pet rect WIDE:', rectWide.split('|').pop());
  // expanded window drops always-on-top (pet.rs:154) - re-assert above the topmost bg
  await ps('petwin.ps1', ['-Action', 'top', '-Which', 'pet']).then(s => log('pet re-top after expand'));
  await sleep(800);
  const inputRect = await cdp(`(function(){var t=document.querySelector('#chat-draft');if(!t)return 'NOINPUT';var b=t.getBoundingClientRect();return JSON.stringify({x:Math.round(b.x),y:Math.round(b.y),w:Math.round(b.width),h:Math.round(b.height)})})()`);
  log('S3 #chat-draft rect (in-window):', inputRect);
  log('P3-wide:', await probe('p3-wide'));

  // ---- shot 4: compact composition (pet visible + bubble + mic) ----
  log('S4: collapse to compact, reopen chat (compact voice composition)');
  await cdp(`[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')==='收起气泡')?.click();'closed'`);
  await sleep(1600); // compact_chat restores 320x440 @saved origin + re-tops
  log('pet rect compact:', (await ps('petwin.ps1', ['-Action', 'rect'])).split('|').pop());
  await ps('petwin.ps1', ['-Action', 'top', '-Which', 'pet']).catch(() => {});
  await cdp(`document.querySelector('button.pet-character').click();'open'`);
  await sleep(1200);
  await cdp(`[...document.querySelectorAll('.pet-mode > button')][1].click();'chat'`);
  await sleep(1100);
  const mic = await cdp(`(document.querySelector('.pet-voice')?.textContent)||'null'`);
  const petVis = await cdp(`(function(){var c=document.querySelector('.pet-character');var b=c.getBoundingClientRect();var cs=getComputedStyle(c);return JSON.stringify({w:Math.round(b.width),h:Math.round(b.height),display:cs.display})})()`);
  log('S4 mic label:', mic, '| .pet-character:', petVis);
  log('P4-compact:', await probe('p4-compact'));
  await cdp(`[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')==='收起气泡')?.click();'closed'`);

  // ---- shot 6: phone window form + DDA render check ----
  log('S6: phone browser (--app mode, no address bar)');
  phoneCtx = await chromium.launchPersistentContext(resolve(HERE, 'phone-profile'), {
    headless: false, viewport: { width: 400, height: 800 },
    args: ['--app=http://127.0.0.1:4318/', '--window-position=560,120', '--window-size=416,880'],
    executablePath: process.env.LOCALAPPDATA + '\\ms-playwright\\chromium-1243\\chrome-win64\\chrome.exe',
  });
  const phone = phoneCtx.pages()[0] ?? await phoneCtx.newPage();
  try { await phone.goto('http://127.0.0.1:4318/', { waitUntil: 'networkidle', timeout: 15000 }); } catch (e) { log('phone goto warn:', e.message); }
  await sleep(1200);
  const ptext = await phone.evaluate(() => document.body.innerText.slice(0, 200));
  log('phone page text head:', ptext.replace(/\n/g, '|').slice(0, 100));
  await ps('phone-top.ps1', ['-Mode', 'pos']).then(s => log('phone pos:', s)).catch(e => log('phone pos WARN', e.message));
  await ps('phone-top.ps1', ['-Mode', 'top']).then(s => log('phone top:', s)).catch(e => log('phone top WARN', e.message));
  await sleep(1000);
  log('P6-gdi (window form):', await probe('p6-gdi'));

  // ddagrab 12s test: phone + pet + wallpaper, probe gdigrab DURING it, then read a frame back
  log('S6: ddagrab 12s test start');
  const ff = spawn('ffmpeg', ['-y', '-hide_banner', '-loglevel', 'error',
    '-f', 'lavfi', '-i', 'ddagrab=output_idx=0:framerate=30:video_size=1920x1032:offset_x=0:offset_y=0',
    '-t', '12', '-vf', 'hwdownload,format=bgra', '-c:v', 'libx264', '-preset', 'medium', '-crf', '19',
    '-pix_fmt', 'yuv420p', resolve(OUT, 'ddatest.mkv')], { stdio: ['pipe', 'ignore', 'pipe'] });
  let ffErr = '';
  ff.stderr.on('data', d => ffErr += d);
  await sleep(5000);
  log('P6-dda-live (gdigrab during ddagrab):', await probe('p6-dda-live'));
  await sleep(4000);
  ff.stdin.write('q');
  await new Promise(r => { ff.on('close', r); setTimeout(r, 6000); });
  log('ddatest exited');
  try {
    await run('ffmpeg', ['-y', '-hide_banner', '-loglevel', 'error', '-ss', '6', '-i', resolve(OUT, 'ddatest.mkv'), '-frames:v', '1', resolve(OUT, 'probe-p6-dda-frame.png')]);
    log('P6-dda-frame extracted');
  } catch (e) { log('P6-dda-frame EXTRACT_FAIL:', e.message.slice(0, 160)); }
  log('ddatest stderr tail:', ffErr.trim().split('\n').slice(-2).join(' | ').slice(0, 200));

  // ---- teardown ----
  await phoneCtx.close().catch(() => {});
  phoneCtx = null;
  await sleep(600);
  appProc.kill();
  await sleep(1200);
  bgProc.kill();
  await ps('stray-windows.ps1', ['-Action', 'restore']).then(() => log('strays restored'));
  log('SMOKE_DONE');
}
main().catch(async e => { console.error('SMOKE_ERROR', e.message); await emergency(); process.exit(1); });
