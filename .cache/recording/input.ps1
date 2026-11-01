# input.ps1 - SendInput-based real mouse/keyboard injection for recording.
# PS 5.1: keep source ASCII (Chinese built from code points elsewhere); run via
#   powershell -File input.ps1 -Action move -X 100 -Y 200
#   powershell -File input.ps1 -Action click -X 960 -Y 900
#   powershell -File input.ps1 -Action drag -X 1500 -Y 900 -X2 300 -Y2 500 -Ms 2200
#   powershell -File input.ps1 -Action text -Text "..."   (KEYEVENTF_UNICODE, no IME needed)
#   powershell -File input.ps1 -Action key -Key enter
param(
  [Parameter(Mandatory=$true)][string]$Action,
  [int]$X = 0,
  [int]$Y = 0,
  [int]$X2 = 0,
  [int]$Y2 = 0,
  [string]$Text = "",
  [string]$Key = "",
  [int]$Ms = 0,
  [int]$Steps = 40,
  [int]$HoldMs = 90
)

$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public static class NativeInput {
  [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] inputs, int size);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int nIndex);

  [StructLayout(LayoutKind.Sequential)]
  public struct INPUT { public uint type; public InputUnion u; }

  [StructLayout(LayoutKind.Explicit)]
  public struct InputUnion {
    [FieldOffset(0)] public MOUSEINPUT mi;
    [FieldOffset(0)] public KEYBDINPUT ki;
  }

  [StructLayout(LayoutKind.Sequential)]
  public struct MOUSEINPUT {
    public int dx; public int dy; public uint mouseData; public uint dwFlags;
    public uint time; public IntPtr dwExtraInfo;
  }

  [StructLayout(LayoutKind.Sequential)]
  public struct KEYBDINPUT {
    public ushort wVk; public ushort wScan; public uint dwFlags;
    public uint time; public IntPtr dwExtraInfo;
  }

  const uint MOUSEEVENTF_MOVE       = 0x0001;
  const uint MOUSEEVENTF_LEFTDOWN   = 0x0002;
  const uint MOUSEEVENTF_LEFTUP     = 0x0004;
  const uint MOUSEEVENTF_RIGHTDOWN  = 0x0008;
  const uint MOUSEEVENTF_RIGHTUP    = 0x0010;
  const uint MOUSEEVENTF_ABSOLUTE   = 0x8000;
  const uint KEYEVENTF_KEYUP        = 0x0002;
  const uint KEYEVENTF_UNICODE      = 0x0004;

  static INPUT[] One() { return new INPUT[1]; }

  public static void MoveTo(int x, int y) {
    INPUT[] inp = One();
    inp[0].type = 0;
    inp[0].u.mi.dx = (int)((long)x * 65535 / GetSystemMetrics(0));
    inp[0].u.mi.dy = (int)((long)y * 65535 / GetSystemMetrics(1));
    inp[0].u.mi.dwFlags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE;
    SendInput(1, inp, Marshal.SizeOf(typeof(INPUT)));
  }

  public static void LeftDown()  { Send(MouseFlag(MOUSEEVENTF_LEFTDOWN)); }
  public static void LeftUp()    { Send(MouseFlag(MOUSEEVENTF_LEFTUP)); }
  public static void RightDown() { Send(MouseFlag(MOUSEEVENTF_RIGHTDOWN)); }
  public static void RightUp()   { Send(MouseFlag(MOUSEEVENTF_RIGHTUP)); }

  static INPUT MouseFlag(uint f) {
    INPUT i = new INPUT(); i.type = 0; i.u.mi.dwFlags = f; return i;
  }
  static void Send(INPUT i) { INPUT[] a = new INPUT[1]; a[0] = i; SendInput(1, a, Marshal.SizeOf(typeof(INPUT))); }

  public static void Unicode(char c) {
    INPUT down = new INPUT(); down.type = 1;
    down.u.ki.wVk = 0; down.u.ki.wScan = c; down.u.ki.dwFlags = KEYEVENTF_UNICODE;
    INPUT up = new INPUT(); up.type = 1;
    up.u.ki.wVk = 0; up.u.ki.wScan = c; up.u.ki.dwFlags = KEYEVENTF_UNICODE | KEYEVENTF_KEYUP;
    INPUT[] a = new INPUT[2]; a[0] = down; a[1] = up;
    SendInput(2, a, Marshal.SizeOf(typeof(INPUT)));
  }

  public static void Vk(ushort vk) {
    INPUT down = new INPUT(); down.type = 1;
    down.u.ki.wVk = vk; down.u.ki.wScan = 0; down.u.ki.dwFlags = 0;
    INPUT up = new INPUT(); up.type = 1;
    up.u.ki.wVk = vk; up.u.ki.wScan = 0; up.u.ki.dwFlags = KEYEVENTF_KEYUP;
    INPUT[] a = new INPUT[2]; a[0] = down; a[1] = up;
    SendInput(2, a, Marshal.SizeOf(typeof(INPUT)));
  }
}
"@

function Get-Vk([string]$name) {
  switch ($name.ToLower()) {
    'enter'  { 0x0D }
    'esc'    { 0x1B }
    'tab'    { 0x09 }
    'space'  { 0x20 }
    'backspace' { 0x08 }
    default  { throw "unknown key name: $name" }
  }
}

switch ($Action) {
  'move' {
    [NativeInput]::MoveTo($X, $Y)
  }
  'glide' {
    # human-ish interpolated cursor travel without buttons (for on-camera motion)
    $dur = if ($Ms -gt 0) { $Ms } else { 1200 }
    $stepMs = [Math]::Max(1, [int]($dur / $Steps))
    $rnd = New-Object System.Random
    for ($i = 1; $i -le $Steps; $i++) {
      $t = $i / $Steps
      $e = if ($t -lt 0.5) { 2 * $t * $t } else { 1 - [Math]::Pow(-2 * $t + 2, 2) / 2 }
      $jx = $rnd.Next(-1, 2); $jy = $rnd.Next(-1, 2)
      [NativeInput]::MoveTo([int]($X + ($X2 - $X) * $e) + $jx, [int]($Y + ($Y2 - $Y) * $e) + $jy)
      Start-Sleep -Milliseconds $stepMs
    }
    [NativeInput]::MoveTo($X2, $Y2)
  }
  'click' {
    [NativeInput]::MoveTo($X, $Y)
    Start-Sleep -Milliseconds 120
    [NativeInput]::LeftDown(); Start-Sleep -Milliseconds $HoldMs; [NativeInput]::LeftUp()
  }
  'doubleclick' {
    [NativeInput]::MoveTo($X, $Y)
    Start-Sleep -Milliseconds 120
    [NativeInput]::LeftDown(); Start-Sleep -Milliseconds 60; [NativeInput]::LeftUp()
    Start-Sleep -Milliseconds 90
    [NativeInput]::LeftDown(); Start-Sleep -Milliseconds 60; [NativeInput]::LeftUp()
  }
  'rightclick' {
    [NativeInput]::MoveTo($X, $Y)
    Start-Sleep -Milliseconds 120
    [NativeInput]::RightDown(); Start-Sleep -Milliseconds $HoldMs; [NativeInput]::RightUp()
  }
  'drag' {
    [NativeInput]::MoveTo($X, $Y)
    Start-Sleep -Milliseconds 250
    [NativeInput]::LeftDown()
    Start-Sleep -Milliseconds 200
    $dur = if ($Ms -gt 0) { $Ms } else { 2000 }
    $stepMs = [Math]::Max(1, [int]($dur / $Steps))
    $rnd = New-Object System.Random
    for ($i = 1; $i -le $Steps; $i++) {
      # ease-in-out interpolation with small jitter, human-ish
      $t = $i / $Steps
      $e = if ($t -lt 0.5) { 2 * $t * $t } else { 1 - [Math]::Pow(-2 * $t + 2, 2) / 2 }
      $jx = $rnd.Next(-1, 2); $jy = $rnd.Next(-1, 2)
      [NativeInput]::MoveTo([int]($X + ($X2 - $X) * $e) + $jx, [int]($Y + ($Y2 - $Y) * $e) + $jy)
      Start-Sleep -Milliseconds $stepMs
    }
    [NativeInput]::MoveTo($X2, $Y2)
    Start-Sleep -Milliseconds 180
    [NativeInput]::LeftUp()
  }
  'text' {
    # human-ish typing pace; assumes target input already focused
    $rnd = New-Object System.Random
    foreach ($ch in $Text.ToCharArray()) {
      [NativeInput]::Unicode($ch)
      Start-Sleep -Milliseconds ($rnd.Next(55, 140))
    }
  }
  'key' {
    [NativeInput]::Vk((Get-Vk $Key))
  }
  default {
    throw "unknown action: $Action (move|click|doubleclick|rightclick|drag|text|key)"
  }
}
