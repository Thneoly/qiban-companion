# petwin.ps1 - locate / move / re-topmost companion windows; optional tray query via UIA.
# Window titles are matched via Unicode code points so this file stays ASCII-only (PS 5.1 safe).
#   petwin "pet"   -> window containing U+6816 U+6816 (pet window title mark)
#   petwin "main"  -> window containing U+6816 U+4F34 (main window title mark)
# Usage:
#   powershell -File petwin.ps1 -Action find [-Which pet|main|any]
#   powershell -File petwin.ps1 -Action move  -Which pet -X 1450 -Y 620
#   powershell -File petwin.ps1 -Action rect  -Which pet
#   powershell -File petwin.ps1 -Action top   -Which pet     (re-assert HWND_TOPMOST)
#   powershell -File petwin.ps1 -Action tray                 (UIA list tray icons, informational)
param(
  [Parameter(Mandatory=$true)][string]$Action,
  [string]$Which = "pet",
  [int]$X = -1,
  [int]$Y = -1
)

$ErrorActionPreference = 'Stop'

# pet title mark: U+6816 U+6816 ; main title mark: U+6816 U+4F34
$MarkPet  = [string][char]0x6816 + [char]0x6816
$MarkMain = [string][char]0x6816 + [char]0x4F34
$Mark = if ($Which -eq 'main') { $MarkMain } elseif ($Which -eq 'pet') { $MarkPet } else { '' }

Add-Type -TypeDefinition @"
using System;
using System.Text;
using System.Runtime.InteropServices;
using System.Collections.Generic;

public static class NativeWin {
  public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);

  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lParam);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr hWnd, StringBuilder sb, int max);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr hWnd, StringBuilder sb, int max);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int cx, int cy, uint flags);

  [StructLayout(LayoutKind.Sequential)]
  public struct RECT { public int L, T, R, B; }

  public static List<string> Found = new List<string>();

  public static void Enumerate(string mark, string procName) {
    Found.Clear();
    EnumWindows((h, lp) => {
      if (!IsWindowVisible(h)) return true;
      StringBuilder sb = new StringBuilder(256);
      GetWindowText(h, sb, 256);
      string title = sb.ToString();
      if (title.Length == 0) return true;
      if (mark != null && mark.Length > 0 && title.IndexOf(mark) < 0) return true;
      uint pid; GetWindowThreadProcessId(h, out pid);
      string pname = "";
      try {
        var p = System.Diagnostics.Process.GetProcessById((int)pid);
        pname = p.ProcessName.ToLowerInvariant();
      } catch {}
      if (procName != null && procName.Length > 0 && !pname.StartsWith(procName)) return true;
      RECT r; GetWindowRect(h, out r);
      Found.Add(h.ToInt64() + "|" + pid + "|" + pname + "|" + title.Replace("|","/") + "|" + (r.R-r.L) + "x" + (r.B-r.T) + "@" + r.L + "," + r.T);
      return true;
    }, IntPtr.Zero);
  }
}
"@

function Select-Window([string]$mark) {
  # companion process name is "companion-desktop" (v0.3.0 build)
  [NativeWin]::Enumerate($mark, 'companion-desktop')
  if ([NativeWin]::Found.Count -eq 0) { return $null }
  return [NativeWin]::Found
}

switch ($Action) {
  'find' {
    $rows = Select-Window $Mark
    if (-not $rows) { Write-Output 'NOT_FOUND'; exit 1 }
    $rows | ForEach-Object { Write-Output $_ }
  }
  'rect' {
    $rows = Select-Window $Mark
    if (-not $rows) { Write-Output 'NOT_FOUND'; exit 1 }
    $rows | ForEach-Object { Write-Output $_ }
  }
  'move' {
    if ($X -lt 0 -or $Y -lt 0) { throw 'move requires -X -Y' }
    $rows = Select-Window $Mark
    if (-not $rows) { Write-Output 'NOT_FOUND'; exit 1 }
    foreach ($row in $rows) {
      $parts = $row.Split('|')
      $h = [IntPtr]::new([Int64]$parts[0])
      $dim = $parts[4]
      $w = [int]($dim -split 'x')[0]
      $hh = [int](($dim -split 'x')[1] -split '@')[0]
      # SWP_NOSIZE(0x1) | SWP_NOZORDER(0x4) | SWP_NOACTIVATE(0x10)
      [NativeWin]::SetWindowPos($h, [IntPtr]::Zero, $X, $Y, 0, 0, 0x1 -bor 0x4 -bor 0x10) | Out-Null
      Write-Output ("MOVED " + $parts[3] + " -> " + $X + "," + $Y)
    }
  }
  'top' {
    $rows = Select-Window $Mark
    if (-not $rows) { Write-Output 'NOT_FOUND'; exit 1 }
    foreach ($row in $rows) {
      $parts = $row.Split('|')
      $h = [IntPtr]::new([Int64]$parts[0])
      # HWND_TOPMOST(-1), SWP_NOMOVE(0x2)|SWP_NOSIZE(0x1)|SWP_NOACTIVATE(0x10)
      [NativeWin]::SetWindowPos($h, [IntPtr]::new(-1), 0, 0, 0, 0, 0x1 -bor 0x2 -bor 0x10) | Out-Null
      Write-Output ("TOPMOST " + $parts[3])
    }
  }
  'tray' {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $cond = [System.Windows.Automation.Condition]::TrueCondition
    $kids = $root.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)
    foreach ($k in $kids) {
      Write-Output ("TOPLEVEL " + $k.Current.Name)
    }
  }
  default { throw "unknown action: $Action (find|rect|move|top|tray)" }
}
