# Measure the visibility-aware pausing: minimize 12 s, restore, then verify
# resumption (log line + post-restore CPU delta over 8 s). Single-instance
# guarded. Raw output: vispause-measure.txt.
$ErrorActionPreference = 'Stop'
$evid = "F:\_RAMShaper\docs\evidence\2026-10-24-rs010-slint-spike"
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class VisP {
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
}
'@
$all = @(Get-Process ramshaper-ui -ErrorAction SilentlyContinue)
if ($all.Count -ne 1) { throw "expected exactly one ramshaper-ui, found $($all.Count)" }
$ui = $all[0]

function Sample($label) {
    $ui.Refresh()
    $line = "$label WS MiB " + [math]::Round($ui.WorkingSet64/1MB,2) + ", cpu_s " + [math]::Round($ui.TotalProcessorTime.TotalSeconds,3)
    $line | Tee-Object "$evid\vispause-measure.txt" -Append
}

"visible: ..." | Set-Content "$evid\vispause-measure.txt"
Sample "visible:"

[VisP]::ShowWindow($ui.MainWindowHandle, 6) | Out-Null   # SW_MINIMIZE
Start-Sleep -Seconds 12
Sample "minimized 12s:"

[VisP]::ShowWindow($ui.MainWindowHandle, 9) | Out-Null   # SW_RESTORE
Start-Sleep -Seconds 3
Sample "restored +3s:"

$ui.Refresh()
$c0 = $ui.TotalProcessorTime.TotalSeconds
Start-Sleep -Seconds 8
$ui.Refresh()
$c1 = $ui.TotalProcessorTime.TotalSeconds
("post-restore 8s cpu delta: " + [math]::Round(($c1-$c0),3) + " cpu-s (resumed updates expect > 0.01)") |
    Tee-Object "$evid\vispause-measure.txt" -Append
Write-Output "measurement done"
