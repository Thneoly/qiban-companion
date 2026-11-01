# bgview.ps1 - borderless fullscreen WinForms window displaying a background PNG.
# Does not steal focus (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW). Kill the process to close.
#   powershell -File bgview.ps1 -Image bg.png [-Topmost] [-Seconds 0] [-W 1920] [-H 1080]
param(
  [Parameter(Mandatory=$true)][string]$Image,
  [switch]$Topmost,
  [int]$Seconds = 0,
  [int]$W = 1920,
  [int]$H = 1080,
  [int]$Left = 0,
  [int]$Top = 0
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

if (-not (Test-Path $Image)) { throw "image not found: $Image" }

$src = [System.Drawing.Image]::FromFile((Resolve-Path $Image).Path)

# WS_EX_NOACTIVATE 0x08000000 | WS_EX_TOOLWINDOW 0x00000080: never takes focus, no taskbar entry
Add-Type -TypeDefinition @"
using System;
using System.Windows.Forms;
public class NoActivateForm : Form {
  protected override CreateParams CreateParams {
    get {
      CreateParams cp = base.CreateParams;
      cp.ExStyle |= 0x08000000 | 0x00000080;
      return cp;
    }
  }
}
"@ -ReferencedAssemblies System.Windows.Forms, System.Drawing

$form = New-Object NoActivateForm
# enum assignments as strings: PS coerces reliably; type literals like
# [System.Drawing.ImageLayout] fail to resolve in some sessions (enum lives in a
# different assembly than its namespace suggests) - that killed the bg window silently
# in the 2026-11-01 rework smoke and is the likely cause of the session-2 white-flash
# marker never rendering.
$form.FormBorderStyle = 'None'
$form.StartPosition = 'Manual'
$form.Bounds = New-Object System.Drawing.Rectangle($Left, $Top, $W, $H)
$form.ShowInTaskbar = $false
$form.TopMost = [bool]$Topmost
$form.BackgroundImage = $src
$form.BackgroundImageLayout = 'Stretch'

if ($Seconds -gt 0) {
  $timer = New-Object System.Windows.Forms.Timer
  $timer.Interval = $Seconds * 1000
  $timer.Add_Tick({ $form.Close() })
  $timer.Start()
}

[System.Windows.Forms.Application]::EnableVisualStyles()
[System.Windows.Forms.Application]::Run($form)
$src.Dispose()
