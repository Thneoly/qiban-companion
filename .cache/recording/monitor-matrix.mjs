// monitor-matrix.mjs - resolve which capture path sees which monitor (2026-11-01 PM).
// Two 300x300 topmost no-activate markers: magenta at DISPLAY1 (100,100),
// yellow at DISPLAY2 (-500,100). During their lifetime: gdigrab desktop frame +
// 3s ddagrab recordings on output_idx 0 and 1. Frames then compared offline.
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = resolve(HERE, 'out');
const T0 = Date.now();
const log = (...a) => console.log(`[${((Date.now() - T0) / 1000).toFixed(1)}]`, ...a);
const sleep = ms => new Promise(r => setTimeout(r, ms));

function run(cmd, args) {
  return new Promise((res, rej) => {
    const p = spawn(cmd, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '', err = '';
    p.stdout.on('data', d => out += d); p.stderr.on('data', d => err += d);
    p.on('close', c => c === 0 ? res(out.trim()) : rej(new Error(`${cmd} exit ${c}: ${err.slice(0, 200)}`)));
  });
}

const m1 = spawn('powershell', ['-NoProfile', '-File', resolve(HERE, 'bgview.ps1'), '-Image', resolve(HERE, 'marker-magenta.png'), '-Topmost', '-Left', '100', '-Top', '100', '-W', '300', '-H', '300'], { stdio: 'ignore' });
const m2 = spawn('powershell', ['-NoProfile', '-File', resolve(HERE, 'bgview.ps1'), '-Image', resolve(HERE, 'marker-yellow.png'), '-Topmost', '-Left', '-500', '-Top', '100', '-W', '300', '-H', '300'], { stdio: 'ignore' });
await sleep(2500);

// enumerate my two marker windows + cloaked state
try {
  const enumOut = await run('powershell', ['-NoProfile', '-Command', `
Add-Type -TypeDefinition 'using System;using System.Text;using System.Runtime.InteropServices;public class W{public delegate bool EP(IntPtr h,IntPtr l);[DllImport(\"user32.dll\")]public static extern bool EnumWindows(EP cb,IntPtr l);[DllImport(\"user32.dll\")]public static extern int GetWindowText(IntPtr h,StringBuilder s,int m);[DllImport(\"user32.dll\")]public static extern bool GetWindowRect(IntPtr h,out R r);[DllImport(\"dwmapi.dll\")]public static extern int DwmGetWindowAttribute(IntPtr h,int a,out int v,int s);public struct R{public int L,T,Rt,B;}}'
[W]::EnumWindows({ param($h,$l) $t=New-Object System.Text.StringBuilder 256; [W]::GetWindowText($h,$t,256)|Out-Null; $ti=$t.ToString(); if($ti -like '*bgview*'){ $r=New-Object W+R; [W]::GetWindowRect($h,[ref]$r)|Out-Null; $c=0; [W]::DwmGetWindowAttribute($h,14,[ref]$c,4)|Out-Null; Write-Output (\"MARK \"+$ti.Substring(0,[Math]::Min(40,$ti.Length))+\" rect=\"+($r.Rt-$r.L)+\"x\"+($r.B-$r.T)+\"@\"+$r.L+\",\"+$r.T+\" cloaked=\"+$c) } $true }, [IntPtr]::Zero) | Out-Null`]);
  log(enumOut || 'NO_MARKERS_ENUMERATED');
} catch (e) { log('enum fail:', e.message.slice(0, 120)); }

// gdigrab desktop (primary) frame
try { await run('ffmpeg', ['-y', '-hide_banner', '-loglevel', 'error', '-f', 'gdigrab', '-framerate', '1', '-i', 'desktop', '-frames:v', '1', resolve(OUT, 'matrix-gdi.png')]); log('matrix-gdi.png ok'); } catch (e) { log('gdi FAIL', e.message.slice(0, 120)); }
// gdigrab with negative offset for left monitor
try { await run('ffmpeg', ['-y', '-hide_banner', '-loglevel', 'error', '-f', 'gdigrab', '-framerate', '1', '-offset_x', '-1920', '-offset_y', '0', '-i', 'desktop', '-frames:v', '1', resolve(OUT, 'matrix-gdi-left.png')]); log('matrix-gdi-left.png ok'); } catch (e) { log('gdi-left FAIL', e.message.slice(0, 120)); }

// ddagrab 3s on output 0 and output 1 (sequential - concurrent DDA sessions on one adapter are fine for different outputs, but keep it simple)
const dda = (idx) => new Promise(res => {
  const p = spawn('ffmpeg', ['-y', '-hide_banner', '-loglevel', 'error',
    '-f', 'lavfi', '-i', `ddagrab=output_idx=${idx}:framerate=30:video_size=400x300:offset_x=0:offset_y=0`,
    '-t', '3', '-vf', 'hwdownload,format=bgra', '-c:v', 'libx264', '-preset', 'ultrafast', '-crf', '20',
    resolve(OUT, `matrix-dda${idx}.mkv`)], { stdio: ['ignore', 'ignore', 'pipe'] });
  let err = ''; p.stderr.on('data', d => err += d);
  p.on('close', c => res(c === 0 ? `dda${idx} ok` : `dda${idx} FAIL ${err.slice(0, 150)}`));
});
log(await dda(0));
log(await dda(1));

for (const idx of ['0', '1']) {
  try { await run('ffmpeg', ['-y', '-hide_banner', '-loglevel', 'error', '-ss', '1.5', '-i', resolve(OUT, `matrix-dda${idx}.mkv`), '-frames:v', '1', resolve(OUT, `matrix-dda${idx}.png`)]); log(`matrix-dda${idx}.png extracted`); } catch (e) { log(`dda${idx} extract FAIL`, e.message.slice(0, 100)); }
}

try { m1.kill(); } catch {}
try { m2.kill(); } catch {}
log('MATRIX_DONE');
