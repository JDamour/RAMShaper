# AT-19 automated portion: keyboard traversal + activation, live-region semantics,
# stale/disconnected legibility (text, not color). Complements the corrected UIA
# tree probe (uia-reprobe.ps1); the interactive Narrator/human pass remains open.
# Raw output: at19-results.txt + at19-stale-state.png next to this script.
$ErrorActionPreference = 'Continue'
$evid  = "F:\_RAMShaper\docs\evidence\2026-10-24-rs010-slint-spike"
$agent = "F:\_RAMShaper\target\release\ramshaper-agent.exe"
$ui    = "F:\_RAMShaper\target\release\ramshaper-ui.exe"
$pipe  = "rs010-at19"
$out   = "$evid\at19-results.txt"

Remove-Item $out -ErrorAction SilentlyContinue
function Rec($msg) { $msg | Out-File $out -Append -Encoding utf8 }

$agentProc = Start-Process -FilePath $agent `
    -ArgumentList "serve","--pipe-name",$pipe,"--interval-ms","500" `
    -PassThru -WindowStyle Hidden `
    -RedirectStandardError "$evid\at19-agent.err"
Start-Sleep -Seconds 2
$uiProc = Start-Process -FilePath $ui -ArgumentList "--pipe-name",$pipe -PassThru
Start-Sleep -Seconds 6

# --- UIA helper job: focused element, named descendant, LiveSetting ------------
function Uia-Job($scriptBlock, $timeoutSec = 30) {
    $job = Start-Job -ScriptBlock $scriptBlock -ArgumentList $uiProc.Id
    if (Wait-Job $job -Timeout $timeoutSec) {
        $r = Receive-Job $job
        Remove-Job $job -Force
        return $r
    }
    Stop-Job $job; Remove-Job $job -Force
    return "UIA JOB TIMED OUT (${timeoutSec}s)"
}

$common = @'
param($targetPid)
Add-Type -AssemblyName UIAutomationClient
$list = New-Object 'System.Collections.Generic.List[string]'
$root = [System.Windows.Automation.AutomationElement]::RootElement
$cond = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::ProcessIdProperty, $targetPid)
$win = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
function Focused {
    $f = [System.Windows.Automation.AutomationElement]::FocusedElement
    if ($f -and $f.Current.ProcessId -eq $targetPid) {
        return "focused: $($f.Current.ControlType.ProgrammaticName) name='$($f.Current.Name)'"
    } elseif ($f) {
        return "focused elsewhere: pid=$($f.Current.ProcessId) $($f.Current.ControlType.ProgrammaticName)"
    }
    return "focused: <none>"
}
'@

$tail = @'
if ($win) { $list.Add((Focused)) } else { $list.Add('WINDOW NOT FOUND') }
return $list
'@

# Phase A1: initial focus, after foregrounding
Rec "=== A1: foreground + initial focused element ==="
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class At19Win {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
}
"@
$null = [At19Win]::ShowWindow($uiProc.MainWindowHandle, 9)
$null = [At19Win]::SetForegroundWindow($uiProc.MainWindowHandle)
Start-Sleep -Seconds 2
Rec (Uia-Job ([scriptblock]::Create($common + $tail)))

# Phase A2: TAB once -> focus should land on the Quit button (only focusable control)
Rec "=== A2: after one TAB ==="
$sh = New-Object -ComObject WScript.Shell
$null = $sh.AppActivate($uiProc.Id)
Start-Sleep -Milliseconds 500
$sh.SendKeys('{TAB}')
Start-Sleep -Seconds 1
Rec (Uia-Job ([scriptblock]::Create($common + $tail)))

# Phase B: live-region + keyboard-activation semantics on the pressure group
Rec "=== B: LiveSetting of the Memory pressure group (expect Polite=1) ==="
Rec (Uia-Job ([scriptblock]::Create($common + @'
if (-not $win) { $list.Add('WINDOW NOT FOUND'); return $list }
$nameCond = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::NameProperty, 'Memory pressure')
$grp = $win.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $nameCond)
if (-not $grp) { $list.Add('Memory pressure group NOT FOUND'); return $list }
try {
        $lp = [System.Windows.Automation.AutomationElement]::LiveSettingProperty
        if (-not $lp) { $lp = [System.Windows.Automation.AutomationProperty]::LookupById(33155) }
        $live = $grp.GetCurrentPropertyValue($lp)
        $list.Add("Memory pressure group: LiveSetting=$live (Off=0 Polite=1 Assertive=2)")
    } catch {
        $list.Add("Memory pressure group: LiveSetting query failed: $($_.Exception.Message)")
    }
return $list
'@)))

# Phase A3: keyboard activation (Enter on focused Quit) -> app exits cleanly
Rec "=== A3: ENTER on focused Quit -> expect graceful exit 0 ==="
$null = $sh.AppActivate($uiProc.Id)
Start-Sleep -Milliseconds 500
$sh.SendKeys('{ENTER}')
$exited = $uiProc.WaitForExit(10000)
if ($exited) {
    Rec "ui process exited; exit code = $($uiProc.ExitCode)"
} else {
    Rec "ui process STILL RUNNING after 10 s of ENTER on Quit"
}
if ($exited) {
    # Phase C needs a live window: relaunch for the stale-state legibility check.
    $uiProc = Start-Process -FilePath $ui -ArgumentList "--pipe-name",$pipe -PassThru
    Start-Sleep -Seconds 6
}

# Phase C: kill the agent -> stale + disconnected states must be legible in text
Rec "=== C: agent killed -> stale/disconnected text states (no color-only info) ==="
Stop-Process -Id $agentProc.Id -Force
Start-Sleep -Seconds 8
Rec (Uia-Job ([scriptblock]::Create($common + @'
if (-not $win) { $list.Add('WINDOW NOT FOUND'); return $list }
function TextByName($name) {
    $c = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::NameProperty, $name)
    $el = $win.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $c)
    if ($el) { return $true } else { return $false }
}
$found = @()
foreach ($n in @('Data freshness:')) {
    $c = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Text)
    $texts = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, $c)
    foreach ($t in $texts) { $found += $t.Current.Name }
}
$list.Add("text elements ($($found.Count)):")
foreach ($t in $found) { $list.Add("  '$t'") }
return $list
'@)) 40)

# Screenshot the stale state (running-app capture for the AT-19 checklist)
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class At19Rect {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
$r = New-Object At19Rect+RECT
$null = [At19Rect]::GetWindowRect($uiProc.MainWindowHandle, [ref]$r)
$w = [Math]::Max(1, $r.Right - $r.Left); $h = [Math]::Max(1, $r.Bottom - $r.Top)
$bmp = New-Object System.Drawing.Bitmap $w, $h
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.Left, $r.Top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
$bmp.Save("$evid\at19-stale-state.png")
$g.Dispose(); $bmp.Dispose()
Rec "screenshot saved: at19-stale-state.png (${w}x${h})"

Stop-Process -Id $uiProc.Id -ErrorAction SilentlyContinue
Write-Output "at19 done -> $out"
