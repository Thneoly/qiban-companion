// phone-render.mjs - shot-6 phone window render test: does
// --disable-features=CalculateNativeWinOcclusion keep the --app window
// rendering under DDA/GDI while the topmost wallpaper is up?
// Also prints the window's DWM cloaked state (14=DWMWA_CLOAKED) before/after raise.
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { chromium } from 'playwright';

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = resolve(HERE, 'out');
const sleep = ms => new Promise(r => setTimeout(r, ms));
const log = (...a) => console.log(...a);

function run(cmd, args) {
  return new Promise((res, rej) => {
    const p = spawn(cmd, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '', err = '';
    p.stdout.on('data', d => out += d); p.stderr.on('data', d => err += d);
    p.on('close', c => c === 0 ? res(out.trim()) : rej(new Error(`${cmd} exit ${c}: ${err.slice(0, 200)}`)));
  });
}
const ps = (file, args) => run('powershell', ['-NoProfile', '-File', resolve(HERE, file), ...args]);

const cloakedCheck = () => run('powershell', ['-NoProfile', '-Command', `
Add-Type -TypeDefinition 'using System;using System.Text;using System.Runtime.InteropServices;using System.Collections.Generic;public class W{public delegate bool EP(IntPtr h,IntPtr l);[DllImport("user32.dll")]public static extern bool EnumWindows(EP cb,IntPtr l);[DllImport("user32.dll")]public static extern bool IsWindowVisible(IntPtr h);[DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);[DllImport("user32.dll")]public static extern bool GetWindowRect(IntPtr h,out R r);[DllImport("dwmapi.dll")]public static extern int DwmGetWindowAttribute(IntPtr h,int a,out int v,int s);public struct R{public int L,T,Rt,B;}public static List<string> Found=new List<string>();public static void Go(){Found.Clear();EnumWindows((h,l)=>{if(!IsWindowVisible(h))return true;R r;GetWindowRect(h,out r);if((r.Rt-r.L)<250||(r.B-r.T)<500)return true;uint p=0;GetWindowThreadProcessId(h,out p);string pn="";try{pn=System.Diagnostics.Process.GetProcessById((int)p).ProcessName;}catch{};if(pn.ToLower()!="chrome")return true;int c=0;DwmGetWindowAttribute(h,14,out c,4);Found.Add("chrome rect="+(r.Rt-r.L)+"x"+(r.B-r.T)+"@"+r.L+","+r.T+" cloaked="+c);return true;},IntPtr.Zero);}}'
[W]::Go(); if([W]::Found.Count -eq 0){'NO_CHROME_WIN'}else{[W]::Found -join ' ; '}`]);

// wallpaper first (phone must be born under it, like the session)
const bg = spawn('powershell', ['-NoProfile', '-File', resolve(HERE, 'bgview.ps1'), '-Image', resolve(HERE, 'wallpaper.png'), '-Topmost'], { stdio: 'ignore' });
await sleep(1500);
await ps('petwin.ps1', ['-Action', 'top', '-Which', 'pet']).catch(() => log('(no pet running - fine)'));

const ctx = await chromium.launchPersistentContext(resolve(HERE, 'phone-profile'), {
  headless: false, viewport: { width: 400, height: 800 },
  args: ['--app=http://127.0.0.1:4318/', '--window-position=560,120', '--window-size=416,880',
         '--disable-gpu'],
  executablePath: process.env.LOCALAPPDATA + '\\ms-playwright\\chromium-1243\\chrome-win64\\chrome.exe',
});
const phone = ctx.pages()[0] ?? await ctx.newPage();
try { await phone.goto('http://127.0.0.1:4318/', { waitUntil: 'networkidle', timeout: 15000 }); } catch (e) { log('goto warn:', e.message); }
await sleep(1500);
log('after launch:', await cloakedCheck());
await ps('phone-top.ps1', ['-Mode', 'pos']).then(s => log('pos:', s)).catch(e => log('pos WARN', e.message));
await ps('phone-top.ps1', ['-Mode', 'top']).then(s => log('top:', s)).catch(e => log('top WARN', e.message));
await sleep(1200);
log('after raise:', await cloakedCheck());

// gdigrab probe (primary only) + 6s ddagrab, then local pixel check of the phone region
const probe = resolve(OUT, 'p6r-gdi.png');
try { await run('ffmpeg', ['-y', '-hide_banner', '-loglevel', 'error', '-f', 'gdigrab', '-framerate', '1', '-offset_x', '0', '-offset_y', '0', '-video_size', '1920x1080', '-i', 'desktop', '-frames:v', '1', '-vf', 'crop=1920:1032:0:0', probe]); log('p6r-gdi.png ok'); } catch (e) { log('gdi FAIL', e.message.slice(0, 120)); }
const mkv = resolve(OUT, 'p6r-dda.mkv');
await run('ffmpeg', ['-y', '-hide_banner', '-loglevel', 'error', '-f', 'lavfi',
  '-i', 'ddagrab=output_idx=0:framerate=30:video_size=1920x1032:offset_x=0:offset_y=0',
  '-t', '6', '-vf', 'hwdownload,format=bgra', '-c:v', 'libx264', '-preset', 'ultrafast', '-crf', '20', '-pix_fmt', 'yuv420p', mkv]);
const ddaframe = resolve(OUT, 'p6r-dda.png');
try { await run('ffmpeg', ['-y', '-hide_banner', '-loglevel', 'error', '-ss', '3', '-i', mkv, '-frames:v', '1', ddaframe]); log('p6r-dda.png ok'); } catch (e) { log('dda extract FAIL', e.message.slice(0, 120)); }

for (const f of [probe, ddaframe]) {
  const r = await run('powershell', ['-NoProfile', '-Command', `
Add-Type -AssemblyName System.Drawing
$b = New-Object System.Drawing.Bitmap('${f.replace(/\\/g, '\\\\')}')
$r=0;$g=0;$bl=0;$n=0
for($x=570;$x -lt 966;$x+=5){ for($y=130;$y -lt 990;$y+=5){ $c=$b.GetPixel($x,$y); $r+=$c.R;$g+=$c.G;$bl+=$c.B;$n++ } }
"PHONE_REGION avgRGB=($([int]($r/$n)),$([int]($g/$n)),$([int]($bl/$n)))"
$b.Dispose()`]);
  log(f.split('\\').pop(), '=>', r);
}

await ctx.close().catch(() => {});
await sleep(600);
try { bg.kill(); } catch {}
log('PHONE_RENDER_TEST_DONE');
