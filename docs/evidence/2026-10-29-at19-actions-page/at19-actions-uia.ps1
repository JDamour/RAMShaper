# AT-19 automated portion for the ACTIONS page (RS-013, shipped 0.3.32).
# The 2026-10-24 inventory predates this page; the human interactive pass
# needs its element inventory. Methodology = uia-reprobe.ps1 (async tree
# push wait, bounded walks, provider diagnostics, two phases).
# The agent runs GATELESS (production default), so the page renders the
# explained-refusal state - which is itself AT-19 scope (stale/unavailable
# states must be carried as text, never color-only).
$ErrorActionPreference = 'Continue'
$evid  = "F:\_RAMShaper\docs\evidence\2026-10-29-at19-actions-page"
$agent = "F:\_RAMShaper\target\release\ramshaper-agent.exe"
$ui    = "F:\_RAMShaper\target\release\ramshaper-ui.exe"
$pipe  = "at19-actions-page"
$out   = "$evid\uia-actions-result.txt"

Remove-Item $out -ErrorAction SilentlyContinue

$agentProc = Start-Process -FilePath $agent `
    -ArgumentList "serve","--pipe-name",$pipe,"--interval-ms","500" `
    -PassThru -WindowStyle Hidden `
    -RedirectStandardError "$evid\agent.err"
Start-Sleep -Seconds 2
$uiProc = Start-Process -FilePath $ui -ArgumentList "--pipe-name",$pipe,"--page","actions" -PassThru
Start-Sleep -Seconds 6
"agent pid=$($agentProc.Id)  ui pid=$($uiProc.Id)  page=actions (gateless agent: refusal state expected)" | Out-File $out -Encoding utf8

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
    $list.Add("window: name='$($win.Current.Name)' type=$($win.Current.ControlType.ProgrammaticName)")
    Start-Sleep -Seconds 3
    $win2 = Find-Window $targetPid
    if ($win2) { $win = $win2 }
    $list.Add("window diagnostics: framework='$($win.Current.FrameworkId)' isControl=$(Prop $win 30016) serverSide=$(Prop $win 30066) providerDesc=$(Prop $win 30067)")
    Walk ([System.Windows.Automation.TreeWalker]::ControlViewWalker) $win "controlview" 5 260
    Walk ([System.Windows.Automation.TreeWalker]::RawViewWalker) $win "rawview" 3 140
    return $list
} -ArgumentList $uiProc.Id

if (Wait-Job $job -Timeout 60) {
    Receive-Job $job | Out-File $out -Append -Encoding utf8
} else {
    "UIA JOB TIMED OUT AFTER 60s" | Out-File $out -Append -Encoding utf8
    Stop-Job $job
}
Remove-Job $job -Force

# Cleanup by explicit PID (never image name - the multi-session gotcha).
Stop-Process -Id $uiProc.Id -Force -ErrorAction SilentlyContinue
Stop-Process -Id $agentProc.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1
"cleanup: ui + agent stopped by explicit pid" | Out-File $out -Append -Encoding utf8
Get-Content $out
