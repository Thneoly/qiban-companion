# phone-top.ps1 - position/topmost the Playwright chromium window (never touches the user's real Chrome).
#   powershell -File phone-top.ps1 -Mode pos    -> place at 560,140, no z-change (sits under topmost bg)
#   powershell -File phone-top.ps1 -Mode top    -> raise to topmost (becomes visible over bg)
param([Parameter(Mandatory=$true)][string]$Mode)

$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class PT {
  public delegate bool EnumProc(IntPtr h, IntPtr lp);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lp);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder sb, int max);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  public struct RECT { public int L,T,R,B; }
}
'@

# only windows of chromium launched from the playwright browser cache
$pw = Get-Process chrome -ErrorAction SilentlyContinue | Where-Object { $_.Path -like '*ms-playwright*' }
if (-not $pw) { Write-Output 'NO_PLAYWRIGHT_CHROME'; exit 1 }
$pids = @($pw | ForEach-Object { $_.Id })

$target = [IntPtr]::Zero
$cb = { param($h,$lp)
  $p = 0; [PT]::GetWindowThreadProcessId($h, [ref]$p) | Out-Null
  if ($pids -contains $p -and [PT]::IsWindowVisible($h)) {
    $r = New-Object PT+RECT; [PT]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $ht = $r.B - $r.T
    if ($w -gt 250 -and $ht -gt 500) { $script:target = $h }
  }
  return $true
}
[PT]::EnumWindows($cb, [IntPtr]::Zero) | Out-Null
if ($target -eq [IntPtr]::Zero) { Write-Output 'NO_WINDOW'; exit 1 }

if ($Mode -eq 'pos') {
  # SWP_NOSIZE(0x1) | SWP_NOZORDER(0x4) | SWP_NOACTIVATE(0x10)
  [PT]::SetWindowPos($target, [IntPtr]::Zero, 560, 120, 0, 0, 0x1 -bor 0x4 -bor 0x10) | Out-Null
  Write-Output "PLACED $target"
} elseif ($Mode -eq 'top') {
  # HWND_TOPMOST(-1), SWP_NOMOVE(0x2)|SWP_NOSIZE(0x1)
  [PT]::SetWindowPos($target, [IntPtr]::new(-1), 0, 0, 0, 0, 0x1 -bor 0x2) | Out-Null
  Write-Output "TOPMOST $target"
} else { Write-Output 'BAD_MODE'; exit 1 }
