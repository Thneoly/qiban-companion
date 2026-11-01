// take-driver.mjs - master orchestrator for the one-long-take recording session.
// Drives: ffmpeg(ddagrab) + OBS(audio) + pet-page CDP interactions + cursor glide + phone window.
//
// 2026-11-01 rework (TUWE-5 v2): wallpaper background + taskbar-free capture.
// - ddagrab region 1920x1032 at (0,0): taskbar (48px, screen 1920x1080@100%) is never captured.
// - shot 2 (drag) is now recorded INSIDE this take (old separate attachment had the article bg).
//   Pet is dragged left for the on-film gesture, then petwin-move'd back home before shot 3.
// - shot 7 closeup source = idle tail at the end of the take (crop happens in compose).
// Prereqs before running: app started with --remote-debugging-port=9222, wallpaper bg window up
//   (bgview.ps1 -Image wallpaper.png -Topmost), OBS running with obs-websocket.
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { chromium } from 'playwright';

const HERE = dirname(fileURLToPath(import.meta.url));
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

// page-side progressive typing (real per-char rendering, ASCII-safe transport via code points)
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
const TODO5 = '周六下午提醒我去公园走走';
const PHONE_NAME = '我的手机';

async function main() {
  // ---- phase 0: phone browser (hidden under topmost bg) ----
  log('launching phone browser');
  const ctx = await chromium.launchPersistentContext(resolve(HERE, 'phone-profile'), {
    headless: false, viewport: { width: 400, height: 800 }, args: ['--window-position=560,120', '--window-size=416,880'],
  });
  const phone = ctx.pages()[0] ?? await ctx.newPage();
  await phone.goto('http://127.0.0.1:4318/', { waitUntil: 'networkidle', timeout: 15000 });
  await sleep(600);
  const pairingShown = (await phone.evaluate(() => document.body.innerText)).includes('配对码');
  log('phone pairing screen:', pairingShown);
  await ps('phone-top.ps1', ['-Mode', 'pos']).catch(e => log('phone pos WARN', e.message));

  // sanity: pet idle
  const st = await cdp(`JSON.stringify({disabled:document.querySelector('button.pet-character')?.disabled, expanded:document.querySelector('button.pet-character')?.getAttribute('aria-expanded')})`);
  log('pet state:', st);
  if (!st.includes('"disabled":false')) throw new Error('pet not interactive — abort before recording');

  // pet must sit ABOVE the wallpaper bg (bg is raised topmost first, then this);
  // stray floaters (e.g. PixPin pins) get parked off-screen for the whole take
  await ps('petwin.ps1', ['-Action', 'top', '-Which', 'pet']).then(s => log('pet top', s)).catch(e => log('pet top WARN', e.message));
  await ps('stray-windows.ps1', ['-Action', 'hide']).then(s => log('strays', s)).catch(e => log('strays WARN', e.message));

  // ---- phase 1: recorders + marker ----
  await node([resolve(HERE, 'obs-ctl.mjs'), 'start']).then(s => log('OBS', s)).catch(e => { throw new Error('OBS start failed: ' + e.message); });
  const ff = spawn('ffmpeg', ['-y', '-f', 'lavfi', '-i', 'ddagrab=output_idx=0:framerate=30:video_size=1920x1032:offset_x=0:offset_y=0', '-t', '300',
    '-vf', 'hwdownload,format=bgra', '-c:v', 'libx264', '-preset', 'medium', '-crf', '19', '-pix_fmt', 'yuv420p',
    resolve(HERE, 'out/take.mkv')], { stdio: ['pipe', 'ignore', 'pipe'] });
  ff.stderr.on('data', () => {});
  ff.on('close', c => log('ffmpeg exited', c));
  await sleep(1200);

  // sync marker: white flash (video) + 880Hz tone (audio)
  spawn('powershell', ['-NoProfile', '-File', resolve(HERE, 'bgview.ps1'), '-Image', resolve(HERE, 'white.png'), '-Topmost', '-Seconds', '1'], { stdio: 'ignore' });
  await psCmd(`(New-Object Media.SoundPlayer '${resolve(HERE, 'tone.wav').replace(/\\/g, '/')}').PlaySync()`).catch(() => {});
  log('marker fired');

  // park cursor in corner, idle shot 1
  await glide(960, 540, 1900, 1062, 900);
  log('shot1 idle start');
  await sleep(8500);

  // ---- shot 2: drag across screen (on-film), then move pet back home ----
  log('shot2 drag');
  const petInfo = await ps('petwin.ps1', ['-Action', 'rect']).catch(e => 'ERR ' + e.message);
  log('pet rect:', petInfo);
  const pm = /(\d+)x(\d+)@(-?\d+),(-?\d+)/.exec(String(petInfo));
  if (!pm) throw new Error('pet window not found for drag');
  const petL = +pm[3], petT = +pm[4], petW = +pm[1], petH = +pm[2];
  const grabX = Math.min(petL + Math.round(petW * 0.55), 1900);
  const grabY = Math.min(petT + Math.round(petH * 0.62), 1020);
  await glide(1900, 1062, grabX, grabY, 1100);
  await sleep(400);
  await ps('input.ps1', ['-Action', 'drag', '-X', String(grabX), '-Y', String(grabY), '-X2', '460', '-Y2', '540', '-Ms', '2300']);
  await sleep(4200); // pet idles at the new spot on film
  await ps('petwin.ps1', ['-Action', 'move', '-Which', 'pet', '-X', String(petL), '-Y', String(petT)]).then(s => log('pet home', s));
  await sleep(1500); // settle before the bubble opens

  // ---- shot 2B: avatar switch - import own Live2D model (2026-11-01 smoke-verified flow) ----
  // real UX: click label -> native folder picker (raised TOPMOST above wallpaper) -> type path
  // -> Enter x2 -> in-page permission bubble「上传到此站点？」-> click 上传 -> Hiyori renders.
  log('shot2B avatar switch');
  await cdp(`document.querySelector('button.pet-character').click();'open'`);
  await sleep(1100);
  await cdp(`(function(){var d=document.querySelector('details.pet-more');if(d&&!d.open){d.querySelector('summary').click()}return 'more'})()`);
  await sleep(600);
  const rPanel = await cdp(`(function(){var b=[...document.querySelectorAll('button')].find(x=>x.textContent.trim()==='角色与场景');if(!b)return 'NOBTN';if(b.disabled)return 'DISABLED';b.click();return 'clicked'})()`);
  log('shot2B panel:', rPanel);
  if (rPanel !== 'clicked') throw new Error('appearance panel failed: ' + rPanel);
  await sleep(1600); // panel readable on film
  const petInfo2 = await ps('petwin.ps1', ['-Action', 'rect']).catch(() => 'ERR');
  const pm2 = /(\d+)x(\d+)@(-?\d+),(-?\d+)/.exec(String(petInfo2));
  if (!pm2) throw new Error('pet window not found for 2B');
  const winL = +pm2[3], winT = +pm2[4];
  const lab = JSON.parse(await cdp(`(function(){var l=document.querySelector('label.model-import');if(!l)return JSON.stringify({x:-1,y:-1});var b=l.getBoundingClientRect();return JSON.stringify({x:Math.round(b.x+b.width/2),y:Math.round(b.y+b.height/2)})})()`));
  if (lab.x < 0) throw new Error('import label not found');
  const labX = winL + lab.x, labY = winT + lab.y;
  await glide(460, 540, labX, labY, 900);
  await ps('input.ps1', ['-Action', 'click', '-X', String(labX), '-Y', String(labY)]);
  await sleep(2200); // native folder picker opens
  await ps('dialog-probe.ps1', ['-Topmost']).then(s => log('picker raised:', s)).catch(e => log('picker probe WARN', e.message));
  await sleep(900); // picker visible beat on film
  await ps('input.ps1', ['-Action', 'text', '-Text', 'D:\\Live2D\\Hiyori']);
  await ps('input.ps1', ['-Action', 'key', '-Key', 'enter']);
  await sleep(2000);
  await ps('input.ps1', ['-Action', 'key', '-Key', 'enter']); // 选择文件夹
  await sleep(1500); // picker closes; permission bubble docks top-left of the pet window
  const upX = winL + 212, upY = winT + 101; // 上传 button offset, measured 2026-11-01
  await glide(labX, labY, upX, upY, 700);
  await ps('input.ps1', ['-Action', 'click', '-X', String(upX), '-Y', String(upY)]);
  let r2b = 'pending';
  for (let i = 0; i < 14; i++) {
    await sleep(1000);
    r2b = await cdp(`(document.querySelector('.appearance-panel button[aria-pressed=true]')?.textContent)||'none'`);
    if (String(r2b).includes('2D 数字人')) break;
  }
  log('shot2B renderer after import:', r2b);
  await sleep(3000); // Hiyori idles on film
  // off-film cleanup: remove imported model, back to interaction, close bubble (shot3 expects it closed)
  await cdp(`(async function(){var b=[...document.querySelectorAll('.appearance-panel button')].find(x=>x.textContent.trim()==='移除本机模型');if(b){b.click();await new Promise(r=>setTimeout(r,1500))}var back=[...document.querySelectorAll('.appearance-panel button')].find(x=>x.textContent.trim()==='返回互动');if(back)back.click();await new Promise(r=>setTimeout(r,700));var x=[...document.querySelectorAll('button')].find(e=>e.getAttribute('aria-label')==='收起气泡');if(x)x.click();return 'cleaned'})()`);
  await sleep(1200);

  // ---- shot 3: typing chat ----
  log('shot3 open bubble');
  await glide(1900, 1062, 1608, 770, 1300);
  await cdp(`document.querySelector('button.pet-character').click();'open'`);
  await sleep(1300);
  await cdp(`[...document.querySelectorAll('.pet-mode > button')][1].click();'chat'`);
  await sleep(1100);
  await cdp(`[...document.querySelectorAll('.chat-state-line button')].find(b=>b.textContent==='展开阅读')?.click()??'NOBTN';'reading'`);
  await sleep(1900);
  await glide(1610, 770, 1390, 800, 900);
  log('shot3 typing');
  await cdp(typeJs('#chat-draft', TEXT3, 'textarea'));
  await sleep(700);
  await glide(1390, 800, 1716, 871, 800);
  const r3 = await cdp(sendAndWaitJs);
  log('shot3 reply:', r3);
  await sleep(3200);

  // ---- shot 5: todo ----
  log('shot5 todo');
  await cdp(`[...document.querySelectorAll('.pet-mode > button')][0].click();'todo'`);
  await sleep(1900);
  await glide(1716, 871, 1580, 724, 900);
  await cdp(typeJs('#pet-draft', TODO5, 'input'));
  await sleep(500);
  await glide(1580, 724, 1712, 724, 600);
  const r5 = await cdp(`var b=document.querySelector('form .pet-save');if(b&&!b.disabled){b.click();'saved'}else 'DISABLED'`);
  log('shot5 todo:', r5);
  await sleep(2800);

  // ---- shot 4: voice turn ----
  log('shot4 voice');
  const s4a = await cdp(`[...document.querySelectorAll('.pet-mode > button')][1].click();'chat'`);
  await sleep(1000);
  await cdp(`[...document.querySelectorAll('.chat-state-line button')].find(b=>b.textContent==='展开阅读')?.click();'reading'`);
  await sleep(1900);
  await glide(1712, 724, 1637, 871, 1000);
  await sleep(400);
  let r4 = await voiceTurn();
  log('shot4 voice result:', r4);
  if (!String(r4).includes('phase=已完成')) {
    log('shot4 retry after settle');
    await sleep(12000);
    r4 = await voiceTurn();
    log('shot4 voice retry:', r4);
  }
  await sleep(2500);

  // ---- shot 6: phone continuation (B) ----
  log('shot6 pairing code');
  const code = await cdpPanel(`(async()=>{var b=[...document.querySelectorAll('button')].find(e=>e.textContent.trim()==='生成五分钟配对码');if(!b)return 'NOBTN';b.click();await new Promise(r=>setTimeout(r,2200));var m=document.body.innerText.match(/(\\d{6})\\s*\\n\\s*有效至/);return m?m[1]:'NOCODE:'+document.body.innerText.slice(700,900);})()`);
  log('pairing code:', code);
  await cdp(`[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')==='收起气泡')?.click();'closed'`);
  await sleep(1200);
  await glide(1637, 871, 960, 900, 900);
  await ps('phone-top.ps1', ['-Mode', 'top']).then(s => log('phone', s)).catch(e => log('phone top WARN', e.message));
  await sleep(1200);
  // type name + code in the phone window
  const inputs = phone.locator('input');
  await inputs.nth(0).fill(PHONE_NAME);
  await inputs.nth(1).fill(String(code));
  await sleep(400);
  await phone.getByRole('button', { name: /与电脑配对/ }).click();
  await sleep(2500);
  const ptext = await phone.evaluate(() => document.body.innerText.slice(0, 300));
  log('phone after pair:', ptext.replace(/\n/g, '|').slice(0, 160));
  await sleep(7500);

  // ---- shot 7 source: closeup idle tail (phone gone, pet motion arc) ----
  await ctx.close().catch(() => {});
  await glide(960, 900, 1900, 1062, 900);
  log('shot7 idle tail start');
  await sleep(11000);

  // ---- wrap ----
  ff.stdin.write('q');
  await new Promise(r => setTimeout(r, 2500));
  try { ff.kill(); } catch {}
  await node([resolve(HERE, 'obs-ctl.mjs'), 'stop']).then(s => log('OBS', s)).catch(e => log('OBS stop WARN', e.message));
  await ps('stray-windows.ps1', ['-Action', 'restore']).then(s => log('strays', s)).catch(e => log('strays restore WARN', e.message));
  log('TAKE_DONE');
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

main().catch(e => { console.error('DRIVER_ERROR', e.message); process.exit(1); });
