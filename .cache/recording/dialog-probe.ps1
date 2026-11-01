# dialog-probe.ps1 - enumerate common-dialog windows (#32770) and report class/title/rect/foreground.
param()
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class WinEnum {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
$found = New-Object System.Collections.ArrayList
$cb = [WinEnum+EnumProc]{ param($h, $l)
  $sb = New-Object System.Text.StringBuilder 256
  [void][WinEnum]::GetClassName($h, $sb, 256)
  if ($sb.ToString() -eq '#32770') {
    $t = New-Object System.Text.StringBuilder 256
    [void][WinEnum]::GetWindowText($h, $t, 256)
    $r = New-Object WinEnum+RECT
    [void][WinEnum]::GetWindowRect($h, [ref]$r)
    [void]$found.Add([pscustomobject]@{ Hwnd = $h; Title = $t.ToString(); Visible = [WinEnum]::IsWindowVisible($h); Rect = "$($r.R-$r.L)x$($r.B-$r.T)@$($r.L),$($r.T)" })
  }
  return $true
}
[void][WinEnum]::EnumWindows($cb, [IntPtr]::Zero)
if ($found.Count -eq 0) { Write-Output 'NO_DIALOG'; exit 0 }
$fg = [WinEnum]::GetForegroundWindow()
foreach ($d in $found) {
  Write-Output ("DIALOG hwnd={0} title='{1}' visible={2} rect={3} foreground={4}" -f $d.Hwnd, $d.Title, $d.Visible, $d.Rect, ($d.Hwnd -eq $fg))
}
# optional: raise the first visible dialog to HWND_TOPMOST (flag: pass -Topmost)
if ($args -contains '-Topmost') {
  $d = $found | Where-Object { $_.Visible } | Select-Object -First 1
  if ($d) { [void][WinEnum]::SetWindowPos($d.Hwnd, [IntPtr]-1, 0,0,0,0, 0x0003); Write-Output "RAISED $($d.Hwnd)" }
}
