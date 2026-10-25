# Measure the visibility-aware pausing with IsIconic ground truth: minimize,
# verify iconic, restore, verify not-iconic, then check the UI's own recovery
# (log line + post-restore CPU delta). Raw output: vispause-measure.txt.
$ErrorActionPreference = 'Stop'
$evid = "F:\_RAMShaper\docs\evidence\2026-10-24-rs010-slint-spike"
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class VisP {
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
}
'@
$all = @(Get-Process ramshaper-ui -ErrorAction SilentlyContinue)
if ($all.Count -ne 1) { throw "expected exactly one ramshaper-ui, found $($all.Count)" }
$ui = $all[0]
$hwnd = $ui.MainWindowHandle

function Sample($label) {
    $ui.Refresh()
    $line = "$label WS MiB " + [math]::Round($ui.WorkingSet64/1MB,2) + ", cpu_s " + [math]::Round($ui.TotalProcessorTime.TotalSeconds,3) + ", iconic=" + [VisP]::IsIconic($hwnd)
    $line | Tee-Object "$evid\vispause-measure.txt" -Append
}

"visible: ..." | Set-Content "$evid\vispause-measure.txt"
Sample "visible:"

[VisP]::ShowWindow($hwnd, 6) | Out-Null   # SW_MINIMIZE
Start-Sleep -Seconds 3
Sample "minimized +3s:"
Start-Sleep -Seconds 9
Sample "minimized 12s:"

[VisP]::ShowWindow($hwnd, 9) | Out-Null   # SW_RESTORE
Start-Sleep -Seconds 2
Sample "restored +2s:"
Start-Sleep -Seconds 3
Sample "restored +5s:"

$ui.Refresh()
$c0 = $ui.TotalProcessorTime.TotalSeconds
Start-Sleep -Seconds 8
Sample "post-restore +13s:"
$ui.Refresh()
$c1 = $ui.TotalProcessorTime.TotalSeconds
("post-restore 8s cpu delta: " + [math]::Round(($c1-$c0),3) + " cpu-s (resumed updates expect > 0.01)") |
    Tee-Object "$evid\vispause-measure.txt" -Append
Write-Output "measurement done"
