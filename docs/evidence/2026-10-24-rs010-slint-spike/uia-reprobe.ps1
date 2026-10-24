# AT-19 UIA re-probe (corrected methodology after the 14:16 zero-children capture).
# Differences from capture-uia-scope-fix.ps1:
#   1. Wait 3 s after the first UIA query before walking - the AccessKit initial
#      tree is pushed ASYNCHRONOUSLY (WM_GETOBJECT -> event proxy -> Slint event
#      loop -> tree update), so an immediate walk can race the push.
#   2. Report provider diagnostics on the window element (FrameworkId,
#      IsControlElement, IsContentElement, HasServerSideProvider id 30066,
#      ProviderDescription id 30067) to distinguish "no server-side provider
#      (HWND fallback only)" from "provider present but empty tree".
#   3. Walk ControlView (depth 4) AND RawView (depth 3) with a hard element
#      budget; unbounded walks hang (see RUN_LOG gotcha).
#   4. Phase 2: bring the window to the foreground, wait, and re-walk - some
#      providers only activate on focus.
# Raw output: uia-reprobe-result.txt next to this script.
$ErrorActionPreference = 'Continue'
$evid  = "F:\_RAMShaper\docs\evidence\2026-10-24-rs010-slint-spike"
$agent = "F:\_RAMShaper\target\release\ramshaper-agent.exe"
$ui    = "F:\_RAMShaper\target\release\ramshaper-ui.exe"
$pipe  = "rs010-uiareprobe"
$out   = "$evid\uia-reprobe-result.txt"

Remove-Item $out -ErrorAction SilentlyContinue

$agentProc = Start-Process -FilePath $agent `
    -ArgumentList "serve","--pipe-name",$pipe,"--interval-ms","500" `
    -PassThru -WindowStyle Hidden `
    -RedirectStandardError "$evid\uia-reprobe-agent.err"
Start-Sleep -Seconds 2
$uiProc = Start-Process -FilePath $ui -ArgumentList "--pipe-name",$pipe -PassThru
Start-Sleep -Seconds 6

$job = Start-Job -ScriptBlock {
    param($targetPid)
    Add-Type -AssemblyName UIAutomationClient
    $list = New-Object 'System.Collections.Generic.List[string]'
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    function Find-Window($pidToFind) {
        $cond = New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::ProcessIdProperty, $pidToFind)
        return $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
    }
    function Prop($el, [int]$id) {
        try {
            $v = $el.GetCurrentPropertyValue([System.Windows.Automation.AutomationProperty]::LookupById($id))
            if ($null -eq $v) { return "<null>" }
            $v = "$v"
            if ($v.Length -gt 180) { $v = $v.Substring(0,180) + "..." }
            return $v
        } catch { return "<unavailable>" }
    }
    function Walk($walker, $rootEl, [string]$label, [int]$maxDepth, [int]$budget) {
        $count = 0
        $stack = New-Object 'System.Collections.Generic.Stack[object]'
        $stack.Push(@{ el = $rootEl; depth = 0 })
        while ($stack.Count -gt 0 -and $count -lt $budget) {
            $cur = $stack.Pop()
            $parentDepth = $cur.depth
            $child = $walker.GetFirstChild($cur.el)
            while ($child -and $count -lt $budget) {
                $count++
                $list.Add(("  " * ($parentDepth + 1)) + "$($child.Current.ControlType.ProgrammaticName) name='$($child.Current.Name)' focusable=$($child.Current.IsKeyboardFocusable)")
                if ($parentDepth + 1 -lt $maxDepth) {
                    $stack.Push(@{ el = $child; depth = $parentDepth + 1 })
                }
                $child = $walker.GetNextSibling($child)
            }
        }
        $list.Add("[$label] $count elements (depth<=$maxDepth budget=$budget)")
    }

    $win = Find-Window $targetPid
    if (-not $win) { $list.Add("WINDOW NOT FOUND IN UIA TREE"); return $list }
    $list.Add("PASS1 window: name='$($win.Current.Name)' type=$($win.Current.ControlType.ProgrammaticName)")
    Start-Sleep -Seconds 3
    $win2 = Find-Window $targetPid
    if ($win2) { $win = $win2 }
    $list.Add("window diagnostics: framework='$($win.Current.FrameworkId)' isControl=$(Prop $win 30016) isContent=$(Prop $win 30017) serverSide=$(Prop $win 30066) providerDesc=$(Prop $win 30067)")
    Walk ([System.Windows.Automation.TreeWalker]::ControlViewWalker) $win "controlview" 4 200
    Walk ([System.Windows.Automation.TreeWalker]::RawViewWalker) $win "rawview" 3 120
    return $list
} -ArgumentList $uiProc.Id

if (Wait-Job $job -Timeout 45) {
    "=== PHASE 1 (background query, 3 s settle) ===" | Out-File $out -Encoding utf8
    Receive-Job $job | Out-File $out -Append -Encoding utf8
} else {
    "PHASE 1 UIA JOB TIMED OUT AFTER 45s" | Out-File $out -Encoding utf8
    Stop-Job $job
}
Remove-Job $job -Force

# Phase 2: foreground the window, wait, re-walk (provider may activate on focus).
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class Win32Focus {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
}
"@
$null = [Win32Focus]::ShowWindow($uiProc.MainWindowHandle, 9)
$null = [Win32Focus]::SetForegroundWindow($uiProc.MainWindowHandle)
Start-Sleep -Seconds 3

$job2 = Start-Job -ScriptBlock {
    param($targetPid)
    Add-Type -AssemblyName UIAutomationClient
    $list = New-Object 'System.Collections.Generic.List[string]'
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $cond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ProcessIdProperty, $targetPid)
    $win = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
    if (-not $win) { $list.Add("WINDOW NOT FOUND (phase 2)"); return $list }
    $list.Add("PASS2 window: name='$($win.Current.Name)' framework='$($win.Current.FrameworkId)'")
    $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
    $count = 0
    $stack = New-Object 'System.Collections.Generic.Stack[object]'
    $stack.Push(@{ el = $win; depth = 0 })
    while ($stack.Count -gt 0 -and $count -lt 200) {
        $cur = $stack.Pop()
        $parentDepth = $cur.depth
        $child = $walker.GetFirstChild($cur.el)
        while ($child -and $count -lt 200) {
            $count++
            $list.Add(("  " * ($parentDepth + 1)) + "$($child.Current.ControlType.ProgrammaticName) name='$($child.Current.Name)' focusable=$($child.Current.IsKeyboardFocusable)")
            if ($parentDepth + 1 -lt 4) { $stack.Push(@{ el = $child; depth = $parentDepth + 1 }) }
            $child = $walker.GetNextSibling($child)
        }
    }
    $list.Add("[controlview-pass2] $count elements (after foreground + 3 s)")
    return $list
} -ArgumentList $uiProc.Id

if (Wait-Job $job2 -Timeout 45) {
    "=== PHASE 2 (foreground + 3 s settle) ===" | Out-File $out -Append -Encoding utf8
    Receive-Job $job2 | Out-File $out -Append -Encoding utf8
} else {
    "PHASE 2 UIA JOB TIMED OUT AFTER 45s" | Out-File $out -Append -Encoding utf8
    Stop-Job $job2
}
Remove-Job $job2 -Force

Stop-Process -Id $uiProc.Id -ErrorAction SilentlyContinue
Stop-Process -Id $agentProc.Id -ErrorAction SilentlyContinue
Write-Output "reprobe done -> $out"
