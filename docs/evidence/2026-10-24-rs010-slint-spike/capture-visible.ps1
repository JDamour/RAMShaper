# RS-010 evidence capture: agent + UI live run, screenshot, UIA tree,
# DPI awareness, footprint visible/hidden.
$ErrorActionPreference = 'Stop'
$root = "F:\_RAMShaper"
$dir = "$root\evidence\rs-010"
$agent = "$root\target\x86_64-pc-windows-msvc\release\ramshaper-agent.exe"
$ui    = "$root\target\x86_64-pc-windows-msvc\release\ramshaper-ui.exe"

# 1. Start the agent.
$agentProc = Start-Process -FilePath $agent -ArgumentList "serve","--pipe-name","rs010-evidence","--interval-ms","500" -PassThru -WindowStyle Hidden -RedirectStandardError "$dir\agent.err"
Start-Sleep -Seconds 2

# 2. Start the UI (visible window on the interactive desktop).
$uiProc = Start-Process -FilePath $ui -ArgumentList "--pipe-name","rs010-evidence" -PassThru
Start-Sleep -Seconds 6   # connect + a few snapshots + pressure warm-up

# 3. Screenshot the foreground window.
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
Start-Sleep -Milliseconds 500
$b = [System.Drawing.Rectangle]::FromLTRB(0,0,[System.Windows.Forms.Screen]::PrimaryScreen.Bounds.Width,[System.Windows.Forms.Screen]::PrimaryScreen.Bounds.Height)
$bmp = New-Object System.Drawing.Bitmap($b.Width, $b.Height)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen(0,0,0,0,$bmp.Size)
$bmp.Save("$dir\ui-visible.png",[System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()

# 4. UIA accessibility tree for the UI process (what Narrator consumes).
Add-Type -AssemblyName UIAutomationClient
$rootEl = [System.Windows.Automation.AutomationElement]::RootElement
$cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $uiProc.Id)
$win = $rootEl.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
$lines = @()
if ($win) {
  $lines += "WINDOW: name='$($win.Current.Name)' type=$($win.Current.ControlType.ProgrammaticName) focusable=$($win.Current.IsKeyboardFocusable)"
  $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
  function Walk($el, $depth) {
    $child = $walker.GetFirstChildElement($el)
    while ($child) {
      $pad = '  ' * $depth
      $lines += "$pad$($child.Current.ControlType.ProgrammaticName) name='$($child.Current.Name)' focusable=$($child.Current.IsKeyboardFocusable)"
      Walk $child ($depth + 1)
      $child = $walker.GetNextSiblingElement($child)
    }
  }
  Walk $win 1
} else { $lines += "WINDOW NOT FOUND IN UIA TREE" }
$lines | Out-File "$dir\uia-tree.txt" -Encoding utf8

# 5. DPI awareness of the UI process.
$dpiAware = Get-Process -Id $uiProc.Id | Select-Object -ExpandProperty ProcessDpiAwareness 2>$null
$lines2 = @("ProcessDpiAwareness(raw): $dpiAware")
# Scale via UIA: bounding rectangle of the window in physical px.
if ($win) { $lines2 += "window rect: $($win.Current.BoundingRectangle)" }
$lines2 | Out-File "$dir\dpi.txt" -Encoding utf8

# 6. Footprint visible.
$vis = Get-Process -Id $uiProc.Id | Select-Object Name,WorkingSet64,PrivateMemorySize64
"VISIBLE  WS=$($vis.WorkingSet64) Private=$($vis.PrivateMemorySize64)" | Out-File "$dir\footprint.txt" -Encoding utf8

# 7. Minimize, wait, measure again (hidden footprint).
$SIG = '[DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);'
$u = Add-Type -MemberDefinition $SIG -Name W -Namespace P -PassThru
[void]$u::ShowWindow($uiProc.MainWindowHandle, 6)   # SW_MINIMIZE
Start-Sleep -Seconds 8
$hid = Get-Process -Id $uiProc.Id | Select-Object Name,WorkingSet64,PrivateMemorySize64
"HIDDEN   WS=$($hid.WorkingSet64) Private=$($hid.PrivateMemorySize64)" | Out-File "$dir\footprint.txt" -Append -Encoding utf8

# 8. Close UI, stop agent.
Stop-Process -Id $uiProc.Id -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1
$wsh = New-Object -ComObject WScript.Shell
[void]$wsh.SendKeys("^c")  # not reliable cross-process; stop agent directly:
Stop-Process -Id $agentProc.Id -ErrorAction SilentlyContinue
Write-Output "capture complete"
