$ErrorActionPreference = 'Continue'
$dir = "F:\_RAMShaper\evidence\rs-010"
$agent = "F:\_RAMShaper\target\x86_64-pc-windows-msvc\release\ramshaper-agent.exe"
$ui    = "F:\_RAMShaper\target\x86_64-pc-windows-msvc\release\ramshaper-ui.exe"

$agentProc = Start-Process -FilePath $agent -ArgumentList "serve","--pipe-name","rs010-ev2","--interval-ms","500" -PassThru -WindowStyle Hidden -RedirectStandardError "$dir\agent2.err"
Start-Sleep -Seconds 2
$uiProc = Start-Process -FilePath $ui -ArgumentList "--pipe-name","rs010-ev2" -PassThru
Start-Sleep -Seconds 5

# Footprint while visible
$vis = Get-Process -Id $uiProc.Id
"VISIBLE   WS=$($vis.WorkingSet64) Private=$($vis.PrivateMemorySize64)" | Out-File "$dir\footprint.txt" -Encoding utf8

# DPI awareness (managed-ness agnostic raw value via registry-free API)
$dpi = Add-Type -MemberDefinition '[DllImport("shcore.dll")] public static extern int GetProcessDpiAwareness(IntPtr h, out int value);' -Name D -Namespace P -PassThru
$v = 0
[void]$dpi::GetProcessDpiAwareness($uiProc.Handle, [ref]$v)
$dpiText = switch ($v) { 0 {"UNAWARE"} 1 {"SYSTEM"} 2 {"PER_MONITOR_AWARE (v2 capable)"} default {"$v"} }
"DPI awareness: $dpiText" | Out-File "$dir\dpi.txt" -Encoding utf8

# UIA tree: depth-capped, whole-run bounded by an external timeout job
$job = Start-Job -ScriptBlock {
  param($pid2)
  Add-Type -AssemblyName UIAutomationClient
  $rootEl = [System.Windows.Automation.AutomationElement]::RootElement
  $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $pid2)
  $win = $rootEl.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
  if (-not $win) { return @("WINDOW NOT FOUND IN UIA TREE") }
  $out = @("WINDOW: name='$($win.Current.Name)' type=$($win.Current.ControlType.ProgrammaticName) focusable=$($win.Current.IsKeyboardFocusable)")
  $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
  function Walk($el, $depth) {
    if ($depth -gt 4) { return }
    $child = $walker.GetFirstChildElement($el)
    while ($child) {
      $out += ("  " * $depth) + "$($child.Current.ControlType.ProgrammaticName) name='$($child.Current.Name)' focusable=$($child.Current.IsKeyboardFocusable)"
      Walk $child ($depth + 1)
      $child = $walker.GetNextSiblingElement($child)
    }
  }
  Walk $win 1
  return $out
} -ArgumentList $uiProc.Id
if (Wait-Job $job -Timeout 25) { Receive-Job $job | Out-File "$dir\uia-tree.txt" -Encoding utf8 }
else { "UIA WALK TIMED OUT AFTER 25s" | Out-File "$dir\uia-tree.txt" -Encoding utf8; Stop-Job $job }
Remove-Job $job -Force

# Minimize -> hidden footprint
$SIG = '[DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);'
$u = Add-Type -MemberDefinition $SIG -Name W -Namespace P2 -PassThru
[void]$u::ShowWindow($uiProc.MainWindowHandle, 6)
Start-Sleep -Seconds 8
$hid = Get-Process -Id $uiProc.Id
"HIDDEN    WS=$($hid.WorkingSet64) Private=$($hid.PrivateMemorySize64)" | Out-File "$dir\footprint.txt" -Append -Encoding utf8

Stop-Process -Id $uiProc.Id -ErrorAction SilentlyContinue
Stop-Process -Id $agentProc.Id -ErrorAction SilentlyContinue
Write-Output "capture2 complete"
