// diag-desktop.mjs v2 - static-list enumeration (petwin pattern; pipeline output
// inside a PS delegate callback aborts the enum silently - v1 lesson).
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const sleep = ms => new Promise(r => setTimeout(r, ms));
const log = (...a) => console.log(...a);

function run(cmd, args) {
  return new Promise((res, rej) => {
    const p = spawn(cmd, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '', err = '';
    p.stdout.on('data', d => out += d); p.stderr.on('data', d => err += d);
    p.on('close', c => c === 0 ? res(out.trim()) : rej(new Error(`${cmd} exit ${c}: ${err.slice(0, 300)}`)));
  });
}

const PS_ENUM = `
Add-Type -TypeDefinition 'using System;using System.Text;using System.Runtime.InteropServices;using System.Collections.Generic;public class W{public delegate bool EP(IntPtr h,IntPtr l);[DllImport("user32.dll")]public static extern bool EnumWindows(EP cb,IntPtr l);[DllImport("user32.dll")]public static extern int GetWindowText(IntPtr h,StringBuilder s,int m);[DllImport("user32.dll")]public static extern bool IsWindowVisible(IntPtr h);[DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);[DllImport("user32.dll")]public static extern bool GetWindowRect(IntPtr h,out R r);[DllImport("dwmapi.dll")]public static extern int DwmGetWindowAttribute(IntPtr h,int a,out int v,int s);public struct R{public int L,T,Rt,B;}public static List<string> Found=new List<string>();public static void Go(){Found.Clear();EnumWindows((h,l)=>{if(!IsWindowVisible(h))return true;StringBuilder sb=new StringBuilder(256);GetWindowText(h,sb,256);string t=sb.ToString();R r;GetWindowRect(h,out r);if((r.Rt-r.L)<40||(r.B-r.T)<40)return true;int c=0;DwmGetWindowAttribute(h,14,out c,4);uint p=0;GetWindowThreadProcessId(h,out p);string pn="";try{pn=System.Diagnostics.Process.GetProcessById((int)p).ProcessName;}catch{}Found.Add("WIN t=["+t.Substring(0,Math.Min(48,t.Length))+"] proc="+pn+" rect="+(r.Rt-r.L)+"x"+(r.B-r.T)+"@"+r.L+","+r.T+" cloaked="+c);return true;},IntPtr.Zero);}}'
[W]::Go(); [W]::Found | ForEach-Object { Write-Output $_ }`;

log('--- all visible >=40x40 windows as seen from THIS process (static list) ---');
log(await run('powershell', ['-NoProfile', '-Command', PS_ENUM]));

log('--- spawning magenta marker at (100,100) ---');
const m = spawn('powershell', ['-NoProfile', '-File', resolve(HERE, 'bgview.ps1'), '-Image', resolve(HERE, 'marker-magenta.png'), '-Topmost', '-Left', '100', '-Top', '100', '-W', '300', '-H', '300'], { stdio: 'ignore' });
await sleep(2500);
log('--- enumerate again (marker should be listed; cloaked?) ---');
log(await run('powershell', ['-NoProfile', '-Command', PS_ENUM]));
try { m.kill(); } catch {}
log('DIAG_DONE');
