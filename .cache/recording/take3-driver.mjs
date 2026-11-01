// take3-driver.mjs - TUWE-5 v3 rework session (PM 15:09 order, 2026-11-01 PM).
// Re-records ONLY shots 2B / 3 / 4 / 6 with per-shot must-appear composition;
// shots 1/2/5/7 are reused from the delivered draft-v2.mp4 at compose time.
//
// Per-shot must-appear (PM contract) and how each is enforced here:
//   2B: Hiyori FULL BODY, not occluded by the appearance panel
//       -> new beat: inject -> 返回互动 -> 收起气泡 -> film the bare pet window
//          (panel closed = nothing left to occlude; renderer must read 'live2d').
//   3 : chat input box/panel as the visual subject
//       -> wide window (760x720@1010,312) with #chat-draft visible; compose zooms.
//   4 : pet character on screen during the voice turn
//       -> COMPACT window the whole turn (pet.css:138 hides .pet-character in
//          wide mode - that was the v2 defect); mic label must read 语音说话.
//   6 : phone window form standing in frame
//       -> --app window (no address bar, URL never enters frame) + --disable-gpu
//          (GPU-presented chrome windows are invisible to gdigrab/ddagrab on
//          this dual-GPU machine - measured 2026-11-01 PM); pixel-gated before
//          the pairing action, abort instead of shooting blind.
//
// Probe discipline (PM red line,常态化): a gdigrab single-frame probe (PRIMARY
// monitor only: video_size 1920x1080 offset 0,0) runs before every shot as a
// capture-health gate - wallpaper + pet window content must be present, else
// EMERGENCY teardown (the capture layer dropped ALL session windows for ~6 min
// on 2026-11-01 15:21-15:27; do not shoot through an outage). Evidence frames
// are saved per shot for the delivery self-check; they stay LOCAL.
//
// Prereqs (runbook v3 session steps): app up with CDP 9222 at pet home
// 1450,592, wallpaper bg topmost, OBS running. Driver does: phone launch ->
// pet top -> strays -> OBS start -> ddagrab + marker -> shots -> stop.
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
const cdpPanel = (js) => node([resolve(HERE, 'cdp-eval.mjs'), 'view=panel', js]).then(s => s.replace(/^"|"$/g, '').replace(/\\"/g, '"'));
const ps = (file, args) => run('powershell', ['-NoProfile', '-File', resolve(HERE, file), ...args]);
const psCmd = (cmd) => run('powershell', ['-NoProfile', '-Command', cmd]);
const glide = (x0, y0, x1, y1, ms = 1100) => ps('input.ps1', ['-Action', 'glide', '-X', String(x0), '-Y', String(y0), '-X2', String(x1), '-Y2', String(y1), '-Ms', String(ms)]);

const typeJs = (sel, text, kind) => {
  const codes = [...text].map(c => c.codePointAt(0)).join(',');
  const proto = kind === 'textarea' ? 'HTMLTextAreaElement' : 'HTMLInputElement';
  return `(async()=>{var codes=[${codes}];var t=document.querySelector('${sel}');if(!t)return 'NOEL';var desc=Object.getOwnPropertyDescriptor(window.${proto}.prototype,'value').set;for(var i=1;i<=codes.length;i++){desc.call(t,String.fromCodePoint.apply(null,codes.slice(0,i)));t.dispatchEvent(new Event('input',{bubbles:true}));await new Promise(r=>setTimeout(r,95+Math.random()*80));}return 'typed';})()`;
};
const sendAndWaitJs = `(async()=>{var b=document.querySelector('form .pet-save');if(!b||b.disabled)return 'DISABLED';b.click();for(var i=0;i<45;i++){await new Promise(r=>setTimeout(r,700));var ph=document.querySelector('.chat-phase')?.textContent;if(ph==='已完成'||ph==='未完成')return 'phase='+ph+' status='+(document.querySelector('.chat-status')?.textContent||'').slice(0,60);}return 'TIMEOUT';})()`;
const micStateJs = `(document.querySelector('.pet-voice')?.textContent)||'null'`;
const micClickJs = `var m=document.querySelector('.pet-voice');if(m&&!m.disabled){m.click();'CLICKED'}else 'BAD_'+(m?m.textContent:'null')`;
const micWaitDoneJs = `(async()=>{var l='';for(var i=0;i<75;i++){await new Promise(r=>setTimeout(r,800));l=(document.querySelector('.pet-voice')?.textContent)||'';if(l==='语音说话')return 'done heard='+(document.querySelector('.chat-subtitle')?.textContent||'')+' phase='+(document.querySelector('.chat-phase')?.textContent||'');}return 'TIMEOUT_'+l;})()`;

const TEXT3 = '今晚想吃点清淡的，给个建议';
const PHONE_NAME = '我的手机';

// ---- probes: gdigrab single frame, PRIMARY monitor, cropped to 1920x1032 ----
function frame(name) {
  return new Promise(res => {
    const file = resolve(OUT, `take3-${name}.png`);
    const p = spawn('ffmpeg', ['-y', '-hide_banner', '-loglevel', 'error',
      '-f', 'gdigrab', '-framerate', '1', '-offset_x', '0', '-offset_y', '0', '-video_size', '1920x1080', '-i', 'desktop',
      '-frames:v', '1', '-vf', 'crop=1920:1032:0:0', file], { stdio: ['ignore', 'ignore', 'pipe'] });
    let err = '';
    p.stderr.on('data', d => err += d);
    const timer = setTimeout(() => { try { p.kill(); } catch {} res(null); }, 30000);
    p.on('close', code => { clearTimeout(timer); res(code === 0 ? file : null); });
    p.on('error', () => { clearTimeout(timer); res(null); });
  });
}

// region stats via System.Drawing (local only)
function regionAvg(file, x0, y0, x1, y1) {
  const f = file.replace(/\\/g, '\\\\');
  return psCmd(`
Add-Type -AssemblyName System.Drawing
$b = New-Object System.Drawing.Bitmap('${f}')
$r=0;$g=0;$bl=0;$n=0
for($x=${x0};$x -lt ${x1};$x+=6){ for($y=${y0};$y -lt ${y1};$y+=6){ if($x -ge $b.Width -or $y -ge $b.Height){continue}; $c=$b.GetPixel($x,$y); $r+=$c.R;$g+=$c.G;$bl+=$c.B;$n++ } }
Write-Output ("$([int]($r/$n)),$([int]($g/$n)),$([int]($bl/$n))")
$b.Dispose()`).then(s => s.trim().split(',').map(Number));
}

// capture-health gate: wallpaper on, pet window has content. ABORT on failure -
// shooting through a capture outage wastes the session and Thne's screen time.
async function healthGate(tag) {
  const f = await frame(`gate-${tag}`);
  if (!f) throw new Error(`HEALTH_GATE ${tag}: probe capture failed`);
  const wall = await regionAvg(f, 150, 150, 600, 450);
  const pet = await regionAvg(f, 1470, 610, 1750, 1020);
  const wallOk = wall[2] > wall[0] && wall[1] > wall[0] && wall[2] < 130;   // dark blue wallpaper hue
  const petOk = (pet[0] + pet[1] + pet[2]) > (wall[0] + wall[1] + wall[2]) + 25; // content in pet window
  log(`gate-${tag}: wall=(${wall}) pet=(${pet}) -> ${wallOk && petOk ? 'PASS' : 'FAIL'}`);
  if (!wallOk || !petOk) throw new Error(`HEALTH_GATE ${tag}: capture outage or pet missing (wall=${wall} pet=${pet}) - ABORT per runbook`);
  return f;
}

// shot-6 phone pixel gate: the phone window region must be light content.
async function phoneGate(tag, minAvg = 150) {
  const f = await frame(`gate-${tag}`);
  if (!f) throw new Error(`PHONE_GATE ${tag}: probe capture failed`);
  const avg = await regionAvg(f, 570, 130, 966, 990);
  const ok = (avg[0] + avg[1] + avg[2]) / 3 > minAvg;
  log(`phone-gate-${tag}: phone-region avg=(${avg}) -> ${ok ? 'PASS' : 'FAIL'}`);
  if (!ok) throw new Error(`PHONE_GATE ${tag}: phone window not rendering (avg=${avg}) - ABORT, do not shoot blind`);
  return f;
}

async function voiceTurn() {
  const c1 = await cdp(micClickJs);
  if (!c1.includes('CLICKED')) return 'mic-fail ' + c1;
  await sleep(3000); // acquisition lead-in
  await psCmd(`(New-Object Media.SoundPlayer '${resolve(HERE, 'q4_loud.wav').replace(/\\/g, '/')}').PlaySync()`).catch(() => {});
  await sleep(1300);
  const label = await cdp(micStateJs);
  if (label === '结束录音') await cdp(micClickJs);
  else log('mic label before stop:', label);
  return await cdp(micWaitDoneJs);
}

let ff = null, phoneCtx = null;
async function emergency() {
  try { ff?.stdin?.write('q'); } catch {}
  try { await new Promise(r => setTimeout(r, 1500)); ff?.kill(); } catch {}
  try { await node([resolve(HERE, 'obs-ctl.mjs'), 'stop']); } catch {}
  try { await phoneCtx?.close(); } catch {}
  try { await ps('stray-windows.ps1', ['-Action', 'restore']); } catch {}
  log('EMERGENCY_TEARDOWN_DONE');
}

async function main() {
  // ---- phase 0: phone browser under the topmost bg (app+wallpaper already up) ----
  log('launching phone browser (--app, --disable-gpu)');
  phoneCtx = await chromium.launchPersistentContext(resolve(HERE, 'phone-profile'), {
    headless: false, viewport: { width: 400, height: 800 },
    args: ['--app=http://127.0.0.1:4318/', '--window-position=560,120', '--window-size=416,880', '--disable-gpu'],
    executablePath: process.env.LOCALAPPDATA + '\\ms-playwright\\chromium-1243\\chrome-win64\\chrome.exe',
  });
  const phone = phoneCtx.pages()[0] ?? await phoneCtx.newPage();
  try { await phone.goto('http://127.0.0.1:4318/', { waitUntil: 'networkidle', timeout: 15000 }); } catch (e) { log('phone goto warn:', e.message.slice(0, 80)); }
  await sleep(800);
  const pairingShown = (await phone.evaluate(() => document.body.innerText)).includes('配对码');
  log('phone pairing screen:', pairingShown);
  await ps('phone-top.ps1', ['-Mode', 'pos']).catch(e => log('phone pos WARN', e.message));

  // pet above wallpaper; strays parked
  await ps('petwin.ps1', ['-Action', 'top', '-Which', 'pet']).then(s => log('pet top', s)).catch(e => log('pet top WARN', e.message));
  await ps('stray-windows.ps1', ['-Action', 'hide']).then(s => log('strays hidden')).catch(e => log('strays WARN', e.message));

  // ---- phase 1: recorders + marker ----
  await node([resolve(HERE, 'obs-ctl.mjs'), 'start']).then(s => log('OBS', s)).catch(e => { throw new Error('OBS start failed: ' + e.message); });
  ff = spawn('ffmpeg', ['-y', '-f', 'lavfi', '-i', 'ddagrab=output_idx=0:framerate=30:video_size=1920x1032:offset_x=0:offset_y=0', '-t', '260',
    '-vf', 'hwdownload,format=bgra', '-c:v', 'libx264', '-preset', 'medium', '-crf', '19', '-pix_fmt', 'yuv420p',
    resolve(OUT, 'take3.mkv')], { stdio: ['pipe', 'ignore', 'pipe'] });
  ff.stderr.on('data', () => {});
  ff.on('close', c => log('ffmpeg exited', c));
  await sleep(1200);
  spawn('powershell', ['-NoProfile', '-File', resolve(HERE, 'bgview.ps1'), '-Image', resolve(HERE, 'white.png'), '-Topmost', '-Seconds', '1'], { stdio: 'ignore' });
  await psCmd(`(New-Object Media.SoundPlayer '${resolve(HERE, 'tone.wav').replace(/\\/g, '/')}').PlaySync()`).catch(() => {});
  log('marker fired');

  await healthGate('start');
  await glide(1900, 1062, 1700, 950, 900);
  await sleep(1200);

  // ---- shot 2B: panel beat -> inject -> back-to-interaction -> FULL BODY beat ----
  log('shot2B start');
  await cdp(`document.querySelector('button.pet-character').click();'open'`);
  await sleep(1100);
  await cdp(`(function(){var d=document.querySelector('details.pet-more');if(d&&!d.open){d.querySelector('summary').click()}return 'more'})()`);
  await sleep(600);
  const rPanel = await cdp(`(function(){var b=[...document.querySelectorAll('button')].find(x=>x.textContent.trim()==='角色与场景');if(!b)return 'NOBTN';if(b.disabled)return 'DISABLED';b.click();return 'clicked'})()`);
  log('shot2B panel:', rPanel);
  if (rPanel !== 'clicked') throw new Error('appearance panel failed: ' + rPanel);
  await sleep(1600); // panel readable on film (beat 1)
  const petInfo2 = await ps('petwin.ps1', ['-Action', 'rect']).catch(() => 'ERR');
  const pm2 = /(\d+)x(\d+)@(-?\d+),(-?\d+)/.exec(String(petInfo2));
  if (!pm2) throw new Error('pet window not found for 2B');
  const winL = +pm2[3], winT = +pm2[4];
  const lab = JSON.parse(await cdp(`(function(){var l=document.querySelector('label.model-import');if(!l)return JSON.stringify({x:-1,y:-1});var b=l.getBoundingClientRect();return JSON.stringify({x:Math.round(b.x+b.width/2),y:Math.round(b.y+b.height/2)})})()`));
  if (lab.x < 0) throw new Error('import label not found');
  await glide(1700, 950, winL + lab.x, winT + lab.y, 1000);
  await sleep(1400); // cursor at the import entry (beat 1 tail)
  await frame('ev-2b-panel'); // evidence: panel + import entry in frame
  const r2b = await node([resolve(HERE, 'inject-2b.mjs')]).catch(e => 'INJECT_FAIL ' + e.message);
  log('shot2B inject:', String(r2b).trim().split('\n').pop());
  await sleep(1000);
  // NEW beat: leave the cramped panel, collapse the bubble -> bare window = full body
  await glide(winL + lab.x, winT + lab.y, winL + 150, winT + 200, 700);
  const rBack = await cdp(`(function(){var b=[...document.querySelectorAll('.appearance-panel button')].find(x=>x.textContent.trim()==='返回互动');if(b){b.click();return 'back'}return 'NOBTN'})()`);
  log('shot2B 返回互动:', rBack);
  await sleep(900);
  await cdp(`[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')==='收起气泡')?.click();'closed'`);
  await sleep(1600); // beat 3: Hiyori full-body idle
  const rend = await cdp(`document.querySelector('.pet-shell')?.dataset.renderer`);
  log('shot2B renderer:', rend);
  if (rend !== 'live2d') throw new Error('2B must-appear failed: renderer=' + rend + ' (Hiyori not on screen)');
  await sleep(2600); // Hiyori motion on film
  await frame('ev-2b-fullbody'); // evidence: Hiyori full body, no panel
  // off-film cleanup: remove imported model, back to svg
  await cdp(`document.querySelector('button.pet-character').click();'open'`);
  await sleep(1000);
  await cdp(`(function(){var d=document.querySelector('details.pet-more');if(d&&!d.open){d.querySelector('summary').click()}return 'more'})()`);
  await sleep(500);
  await cdp(`(function(){var b=[...document.querySelectorAll('button')].find(x=>x.textContent.trim()==='角色与场景');if(b)b.click();return 'panel'})()`);
  await sleep(1300);
  await cdp(`(async function(){var b=[...document.querySelectorAll('.appearance-panel button')].find(x=>x.textContent.trim()==='移除本机模型');if(b){b.click();await new Promise(r=>setTimeout(r,2000))}var back=[...document.querySelectorAll('.appearance-panel button')].find(x=>x.textContent.trim()==='返回互动');if(back)back.click();await new Promise(r=>setTimeout(r,700));var x=[...document.querySelectorAll('button')].find(e=>e.getAttribute('aria-label')==='收起气泡');if(x)x.click();return 'cleaned'})()`);
  await sleep(1500);
  log('shot2B cleanup renderer:', await cdp(`document.querySelector('.pet-shell')?.dataset.renderer`));

  // ---- shot 3: typing chat in the wide window (input box as subject) ----
  await healthGate('s3');
  log('shot3 open bubble');
  await glide(1900, 1062, 1608, 770, 1300);
  await cdp(`document.querySelector('button.pet-character').click();'open'`);
  await sleep(1300);
  await cdp(`[...document.querySelectorAll('.pet-mode > button')][1].click();'chat'`);
  await sleep(1100);
  await cdp(`[...document.querySelectorAll('.chat-state-line button')].find(b=>b.textContent==='展开阅读')?.click()??'NOBTN';'reading'`);
  await sleep(2400);
  const rectWide = await ps('petwin.ps1', ['-Action', 'rect']);
  log('pet rect WIDE:', rectWide.split('|').pop());
  await ps('petwin.ps1', ['-Action', 'top', '-Which', 'pet']).then(() => log('pet re-top after expand')); // expand drops always-on-top (pet.rs)
  await sleep(800);
  const inputOk = await cdp(`(function(){var t=document.querySelector('#chat-draft');if(!t)return 'NOINPUT';var b=t.getBoundingClientRect();return (b.width>400&&b.height>30)?'input-visible':'input-odd-'+'+'+Math.round(b.width)+'x'+Math.round(b.height)})()`);
  log('shot3 input check:', inputOk);
  await frame('ev-3-wide'); // evidence: wide window + input box in frame
  log('shot3 typing');
  await glide(1610, 770, 1390, 800, 900);
  await cdp(typeJs('#chat-draft', TEXT3, 'textarea'));
  await sleep(700);
  await glide(1390, 800, 1716, 871, 800);
  const r3 = await cdp(sendAndWaitJs);
  log('shot3 reply:', r3);
  await sleep(3200);
  await cdp(`[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')==='收起气泡')?.click();'closed'`);
  await sleep(1800); // window compacts back home

  // ---- shot 4: voice turn, COMPACT composition (pet visible with the bubble) ----
  await healthGate('s4');
  log('shot4 voice (compact)');
  await ps('petwin.ps1', ['-Action', 'top', '-Which', 'pet']).catch(() => {});
  await cdp(`document.querySelector('button.pet-character').click();'open'`);
  await sleep(1300);
  await cdp(`[...document.querySelectorAll('.pet-mode > button')][1].click();'chat'`);
  await sleep(1100);
  const mic = await cdp(micStateJs);
  const petVis = await cdp(`(function(){var c=document.querySelector('.pet-character');var cs=getComputedStyle(c);var b=c.getBoundingClientRect();return cs.display+' '+Math.round(b.width)+'x'+Math.round(b.height)})()`);
  log('shot4 mic:', mic, '| pet:', petVis);
  if (mic !== '语音说话') throw new Error('shot4 must-appear failed: mic=' + mic);
  if (!petVis.startsWith('block')) throw new Error('shot4 must-appear failed: pet hidden (' + petVis + ')');
  await glide(1700, 950, 1637, 871, 900);
  await sleep(400);
  await frame('ev-4-pre'); // evidence: pet + bubble + mic visible before the turn
  let r4 = await voiceTurn();
  log('shot4 voice result:', r4);
  if (!String(r4).includes('phase=已完成')) {
    log('shot4 retry after settle');
    await sleep(12000);
    r4 = await voiceTurn();
    log('shot4 voice retry:', r4);
  }
  await sleep(2600); // TTS lip-sync tail on film
  await cdp(`[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')==='收起气泡')?.click();'closed'`);
  await sleep(1200);

  // ---- shot 6: phone continuation (B viewport, --app window form) ----
  await healthGate('s6');
  log('shot6 pairing code');
  await cdpPanel(`(function(){var a=[...document.querySelectorAll('a.nav-link')].find(e=>e.textContent.trim()==='设备');if(a)a.click();return a?'nav':'NO'})()`);
  await sleep(2000);
  const genCode = () => cdpPanel(`(async()=>{var b=[...document.querySelectorAll('button')].find(e=>e.textContent.trim()==='生成五分钟配对码');if(!b)return 'NOBTN';b.click();await new Promise(r=>setTimeout(r,2200));var m=document.body.innerText.match(/(\\d{6})\\s*\\n\\s*有效至/);return m?m[1]:'NOCODE:'+document.body.innerText.slice(700,900);})()`);
  let code = await genCode();
  if (!/^\d{6}$/.test(String(code))) { await sleep(1500); code = await genCode(); }
  log('pairing code:', code);
  if (!/^\d{6}$/.test(String(code))) throw new Error('pairing code failed: ' + code);
  await cdp(`[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')==='收起气泡')?.click();'closed'`);
  await sleep(1200);
  await glide(1637, 871, 960, 900, 900);
  await ps('phone-top.ps1', ['-Mode', 'top']).then(s => log('phone', s)).catch(e => log('phone top WARN', e.message));
  await sleep(1200);
  await phoneGate('raised'); // MUST-APPEAR GATE: phone window form in frame before acting
  await frame('ev-6-phone'); // evidence: phone window standing in frame (+ pet right)
  const inputs = phone.locator('input');
  await inputs.nth(0).fill(PHONE_NAME);
  await inputs.nth(1).fill(String(code));
  await sleep(400);
  await phone.getByRole('button', { name: /与电脑配对/ }).click();
  await sleep(2500);
  const ptext = await phone.evaluate(() => document.body.innerText.slice(0, 300));
  log('phone after pair:', ptext.replace(/\n/g, '|').slice(0, 160));
  await phoneGate('paired', 120); // window still standing after pairing
  await sleep(6500);

  // ---- wrap ----
  await phoneCtx.close().catch(() => {});
  phoneCtx = null;
  await glide(960, 900, 1900, 1062, 900);
  ff.stdin.write('q');
  await new Promise(r => setTimeout(r, 2500));
  try { ff.kill(); } catch {}
  await node([resolve(HERE, 'obs-ctl.mjs'), 'stop']).then(s => log('OBS', s)).catch(e => log('OBS stop WARN', e.message));
  await ps('stray-windows.ps1', ['-Action', 'restore']).then(() => log('strays restored'));
  log('TAKE3_DONE');
}
main().catch(async e => { console.error('DRIVER_ERROR', e.message); await emergency(); process.exit(1); });
