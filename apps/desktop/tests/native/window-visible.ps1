param([Parameter(Mandatory)][int]$TargetProcessId, [Parameter(Mandatory)][string]$WindowTitle)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class AcceptanceWindows {
    private delegate bool Visitor(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] private static extern bool EnumWindows(Visitor visitor, IntPtr data);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowText(IntPtr window, StringBuilder text, int count);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr window);
    public static bool Visible(int process, string title) {
        bool result = false;
        EnumWindows((window, data) => {
            uint owner; GetWindowThreadProcessId(window, out owner);
            var text = new StringBuilder(256); GetWindowText(window, text, 256);
            if (owner == process && text.ToString() == title && IsWindowVisible(window)) result = true;
            return true;
        }, IntPtr.Zero);
        return result;
    }
}
'@
[AcceptanceWindows]::Visible($TargetProcessId, $WindowTitle)
