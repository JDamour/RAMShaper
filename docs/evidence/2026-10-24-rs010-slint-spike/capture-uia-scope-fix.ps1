$ErrorActionPreference = 'Continue'
$dir = "F:\_RAMShaper\evidence\rs-010"
$agent = "F:\_RAMShaper\target\x86_64-pc-windows-msvc\release\ramshaper-agent.exe"
$ui    = "F:\_RAMShaper\target\x86_64-pc-windows-msvc\release\ramshaper-ui.exe"

$agentProc = Start-Process -FilePath $agent -ArgumentList "serve","--pipe-name","rs010-ev3","--interval-ms","500" -PassThru -WindowStyle Hidden -RedirectStandardError "$dir\agent3.err"
Start-Sleep -Seconds 2
$uiProc = Start-Process -FilePath $ui -ArgumentList "--pipe-name","rs010-ev3" -PassThru
Start-Sleep -Seconds 5

$job = Start-Job -ScriptBlock {
  param($targetPid)
  Add-Type -AssemblyName UIAutomationClient
  $list = New-Object 'System.Collections.Generic.List[string]'
  $rootEl = [System.Windows.Automation.AutomationElement]::RootElement
  $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $targetPid)
  $win = $rootEl.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
  if (-not $win) { $list.Add("WINDOW NOT FOUND IN UIA TREE"); return $list }
  $list.Add("WINDOW: name='$($win.Current.Name)' type=$($win.Current.ControlType.ProgrammaticName) focusable=$($win.Current.IsKeyboardFocusable)")
  $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
  function Walk($el, $depth) {
    if ($depth -gt 4) { return }
    $child = $walker.GetFirstChildElement($el)
    while ($child) {
      $list.Add(("  " * $depth) + "$($child.Current.ControlType.ProgrammaticName) name='$($child.Current.Name)' focusable=$($child.Current.IsKeyboardFocusable)")
      Walk $child ($depth + 1)
      $child = $walker.GetNextSiblingElement($child)
    }
  }
  Walk $win 1
  return $list
} -ArgumentList $uiProc.Id
if (Wait-Job $job -Timeout 30) { Receive-Job $job | Out-File "$dir\uia-tree.txt" -Encoding utf8 }
else { "UIA WALK TIMED OUT AFTER 30s" | Out-File "$dir\uia-tree.txt" -Encoding utf8; Stop-Job $job }
Remove-Job $job -Force
Stop-Process -Id $uiProc.Id -ErrorAction SilentlyContinue
Stop-Process -Id $agentProc.Id -ErrorAction SilentlyContinue
Write-Output "uia2 done"
