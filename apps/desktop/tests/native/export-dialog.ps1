param([Parameter(Mandatory)][int]$TargetProcessId,
      [ValidateSet('inspect','cancel','save','overwrite')][string]$Action='inspect',
      [string]$Destination)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new()
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class ExportDialogControls {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wParam, string text);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern IntPtr GetParent(IntPtr window);
}
'@
$root=[System.Windows.Automation.AutomationElement]::RootElement
$ownerCondition=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty,$TargetProcessId)
$dialogCondition=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty,'#32770')
$dialog=$null
for($attempt=0;$attempt -lt 50;$attempt++) {
    foreach($ownedWindow in $root.FindAll([System.Windows.Automation.TreeScope]::Children,$ownerCondition)) {
        $dialog=$ownedWindow.FindFirst([System.Windows.Automation.TreeScope]::Subtree,$dialogCondition)
        if($dialog){break}
    }
    if($dialog){break}
    Start-Sleep -Milliseconds 100
}
if(-not $dialog){throw 'No owned native dialog found'}
$controls=$dialog.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.Condition]::TrueCondition)
if($Action -eq 'inspect') {
    $controls | ForEach-Object { [pscustomobject]@{Name=$_.Current.Name;Id=$_.Current.AutomationId;Type=$_.Current.ControlType.ProgrammaticName} } | ConvertTo-Json -Depth 3
    exit
}
function Find-Control([string]$AutomationId) {
    $match=$controls | Where-Object {$_.Current.AutomationId -eq $AutomationId -and $_.Current.ControlType -ne [System.Windows.Automation.ControlType]::ListItem} | Select-Object -First 1
    if(-not $match){throw "Missing dialog control $AutomationId"}
    return $match
}
if($Action -eq 'cancel') {
    [ExportDialogControls]::PostMessage([IntPtr]$dialog.Current.NativeWindowHandle,0x111,[IntPtr]2,[IntPtr](Find-Control '2').Current.NativeWindowHandle) | Out-Null
} elseif($Action -eq 'save') {
    if(-not [System.IO.Path]::IsPathRooted($Destination)){throw 'An absolute test destination is required'}
    [ExportDialogControls]::SendMessage([IntPtr](Find-Control '1001').Current.NativeWindowHandle,0xC,[IntPtr]::Zero,$Destination) | Out-Null
    [ExportDialogControls]::PostMessage([IntPtr]$dialog.Current.NativeWindowHandle,0x111,[IntPtr]1,[IntPtr](Find-Control '1').Current.NativeWindowHandle) | Out-Null
} else {
    $button=[IntPtr](Find-Control 'CommandButton_6').Current.NativeWindowHandle
    if($button -eq [IntPtr]::Zero){throw 'Missing native overwrite button handle'}
    [ExportDialogControls]::PostMessage([ExportDialogControls]::GetParent($button),0x111,[IntPtr]6,$button) | Out-Null
}
