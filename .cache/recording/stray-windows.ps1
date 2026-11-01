# stray-windows.ps1 - keep stray topmost floaters (e.g. PixPin pins) out of the take.
# Records original positions to strays.json so restore is exact.
#   powershell -File stray-windows.ps1 -Action list               -> windows actually rendered inside the capture area
#   powershell -File stray-windows.ps1 -Action hide [-Exclude a|b] -> move them to x=2500, save positions
#   powershell -File stray-windows.ps1 -Action restore            -> move back from strays.json
# Capture area = 0,0 - 1920,1032 (taskbar-free region).
# A window counts as a stray only if it is visible, NOT DWM-cloaked (cloaked UWP ghosts
# like suspended Settings never render into DDA - moving them breaks the user's layout
# for nothing, which the first version of this script did on 2026-11-01), overlaps the
# capture area with >=12px in both axes, and is not a fullscreen cover or a skip-proc
# (recording toolchain, companion app, shells, explorer bits, tray helpers <=24px).
param(
  [Parameter(Mandatory=$true)][string]$Action,
  [string[]]$Exclude = @()
)

$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
using System.Collections.Generic;
public static class Stray {
  public delegate bool EnumProc(IntPtr h, IntPtr lp);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lp);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder sb, int max);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out int val, int size);
  public struct RECT { public int L,T,R,B; }
  public static List<string> Hits = new List<string>();
  public static List<long> HitX = new List<long>(), HitY = new List<long>();
  static int Overlap(int a0, int a1, int b0, int b1) { int lo = Math.Max(a0, b0), hi = Math.Min(a1, b1); return Math.Max(0, hi - lo); }
  public static void Scan(int x0, int y0, int x1, int y1, string[] skipProcs) {
    Hits.Clear(); HitX.Clear(); HitY.Clear();
    EnumWindows((h, lp) => {
      if (!IsWindowVisible(h)) return true;
      RECT r; GetWindowRect(h, out r);
      if (Overlap(r.L, r.R, x0, x1) < 12 || Overlap(r.T, r.B, y0, y1) < 12) return true;
      if ((r.R - r.L) < 24 || (r.B - r.T) < 24) return true;               // tray helpers / edge slivers
      if ((r.R - r.L) >= 1900 && (r.B - r.T) >= 1000) return true;         // fullscreen covers
      int cloaked; DwmGetWindowAttribute(h, 14, out cloaked, 4);
      if (cloaked != 0) return true;                                        // cloaked: never renders into DDA
      uint pid; GetWindowThreadProcessId(h, out pid);
      string pname = ""; try { pname = System.Diagnostics.Process.GetProcessById((int)pid).ProcessName; } catch {}
      if (skipProcs != null) foreach (string s in skipProcs) { if (pname.Equals(s, StringComparison.OrdinalIgnoreCase)) return true; }
      StringBuilder sb = new StringBuilder(256); GetWindowText(h, sb, 256);
      Hits.Add(h.ToInt64() + "|" + pid + "|" + pname + "|" + (r.R-r.L) + "x" + (r.B-r.T) + "@" + r.L + "," + r.T + "|" + sb.ToString().Replace("|","/"));
      HitX.Add(r.L); HitY.Add(r.T);
      return true;
    }, IntPtr.Zero);
  }
}
'@

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$json = Join-Path $here 'strays.json'
# own host process tree + recording toolchain + companion app never count as strays
$skip = @('powershell','pwsh','cmd','ffmpeg','obs64','node','companion-desktop','chrome','explorer','ApplicationFrameHost','SystemSettings') + $Exclude

switch ($Action) {
  'list' {
    [Stray]::Scan(0, 0, 1920, 1032, $skip)
    if ([Stray]::Hits.Count -eq 0) { Write-Output 'CLEAN' } else { [Stray]::Hits | ForEach-Object { Write-Output $_ } }
  }
  'hide' {
    [Stray]::Scan(0, 0, 1920, 1032, $skip)
    $saved = @()
    for ($i = 0; $i -lt [Stray]::Hits.Count; $i++) {
      $p = ([Stray]::Hits[$i]).Split('|')
      $h = [IntPtr]::new([Int64]$p[0])
      $x = [int][Stray]::HitX[$i]; $y = [int][Stray]::HitY[$i]
      # SWP_NOSIZE(0x1) | SWP_NOZORDER(0x4) | SWP_NOACTIVATE(0x10)
      [Stray]::SetWindowPos($h, [IntPtr]::Zero, 2500, $y, 0, 0, 0x1 -bor 0x4 -bor 0x10) | Out-Null
      $saved += [pscustomobject]@{ hwnd = [Int64]$p[0]; proc = $p[2]; x = $x; y = $y; desc = ($p[3] + ' ' + $p[4]) }
      Write-Output ("HID " + $p[2] + " " + $p[3] + " " + $p[4])
    }
    if ($saved.Count -gt 0) { ConvertTo-Json -InputObject $saved | Out-File -FilePath $json -Encoding ascii }
    else { Write-Output 'NOTHING_TO_HIDE' }
  }
  'restore' {
    if (-not (Test-Path $json)) { Write-Output 'NO_SAVE'; exit 0 }
    # PS 5.1: ConvertFrom-Json emits a JSON array as ONE pipeline object, so @() would
    # nest it and foreach would hand us the whole array as a single item -> cast crash.
    $parsed = Get-Content $json -Raw | ConvertFrom-Json
    foreach ($it in $parsed) {
      if ($null -eq $it) { continue }
      $h = [IntPtr]::new([Int64]$it.hwnd)
      if ([Stray]::IsWindowVisible($h)) {
        [Stray]::SetWindowPos($h, [IntPtr]::Zero, [int]$it.x, [int]$it.y, 0, 0, 0x1 -bor 0x4 -bor 0x10) | Out-Null
        Write-Output ("RESTORED " + $it.proc + " -> " + $it.x + "," + $it.y)
      } else { Write-Output ("GONE " + $it.proc) }
    }
    Remove-Item $json -ErrorAction SilentlyContinue
  }
  default { throw "unknown action: $Action (list|hide|restore)" }
}
