# dialog-click.ps1 - UIA-locate a named button inside a #32770 common dialog, click its center with real input.
# Usage: powershell -NoProfile -File dialog-click.ps1 -Name 上传 [-Pid <pid>]  (default: companion-desktop's)
# Prints "CLICK <name> x=<x> y=<y>" or "NO_BUTTON <name>".
param([string]$Name = '上传', [int]$ProcId = 0)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
if ($ProcId -eq 0) {
  $p = Get-Process companion-desktop -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $p) { Write-Output 'NO_PROCESS'; exit 1 }
  $ProcId = $p.Id
}
$root = [System.Windows.Automation.AutomationElement]::RootElement
$pidCond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $ProcId)
$wins = $root.FindAll([System.Windows.Automation.TreeScope]::Children, $pidCond)
foreach ($w in $wins) {
  # common dialogs (#32770) surface as child windows of the app process; also match by walking all its top-level windows
  $btnCond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $Name)
  $btns = $w.FindAll([System.Windows.Automation.TreeScope]::Descendants, $btnCond)
  foreach ($b in $btns) {
    if ($b.Current.ControlType -ne [System.Windows.Automation.ControlType]::Button) { continue }
    try {
      $pt = $b.GetClickablePoint()
      Write-Output ("CLICK {0} x={1} y={2}" -f $Name, [int]$pt.X, [int]$pt.Y)
      & (Join-Path $PSScriptRoot 'input.ps1') -Action click -X ([int]$pt.X) -Y ([int]$pt.Y)
      exit 0
    } catch {}
  }
}
Write-Output "NO_BUTTON $Name"
exit 1
