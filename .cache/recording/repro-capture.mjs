// repro-capture.mjs - minimal local-only reproduction #3 of the capture regression
// (no uploads anywhere; pixel scan via System.Drawing in-process).
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { existsSync, unlinkSync } from 'node:fs';

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

const frame = resolve(OUT, 'repro-gdi.png');
const m = spawn('powershell', ['-NoProfile', '-File', resolve(HERE, 'bgview.ps1'), '-Image', resolve(HERE, 'marker-magenta.png'), '-Topmost', '-Left', '100', '-Top', '100', '-W', '300', '-H', '300'], { stdio: 'ignore' });
await sleep(2500);
await run('ffmpeg', ['-y', '-hide_banner', '-loglevel', 'error', '-f', 'gdigrab', '-framerate', '1', '-i', 'desktop', '-frames:v', '1', frame]);
try { m.kill(); } catch {}

const scan = await run('powershell', ['-NoProfile', '-Command', `
Add-Type -AssemblyName System.Drawing
$b = New-Object System.Drawing.Bitmap('${frame.replace(/\\/g, '\\\\')}')
"FRAME=$($b.Width)x$($b.Height)"
$mag=0
for($x=0;$x -lt $b.Width;$x+=5){ for($y=0;$y -lt $b.Height;$y+=5){ $c=$b.GetPixel($x,$y); if($c.R -gt 200 -and $c.B -gt 200 -and $c.G -lt 60){ $mag++ } } }
"MAGENTA_PIXELS=$mag"
$b.Dispose()`]);
log(scan);
if (existsSync(frame)) unlinkSync(frame);
log('REPRO3_DONE (frame deleted)');
