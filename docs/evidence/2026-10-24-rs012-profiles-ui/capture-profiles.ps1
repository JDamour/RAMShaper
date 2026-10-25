# Capture the RAMShaper window content via PrintWindow (works even when the
# window is behind other windows — no foreground lock involved).
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public struct RECTP { public int Left, Top, Right, Bottom; }
public static class PW {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECTP r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
}
'@
Add-Type -AssemblyName System.Drawing
$p = @(Get-Process ramshaper-ui -ErrorAction SilentlyContinue) | Select-Object -First 1
if (-not $p) { throw "no ramshaper-ui process" }
$hwnd = $p.MainWindowHandle
$r = New-Object RECTP
$null = [PW]::GetWindowRect($hwnd, [ref]$r)
$w = $r.Right - $r.Left; $h = $r.Bottom - $r.Top
if ($w -le 0 -or $h -le 0) { throw "window has no size" }
$bmp = New-Object System.Drawing.Bitmap $w, $h
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
# 2 = PW_RENDERFULLCONTENT: includes DWM-composited content
$ok = [PW]::PrintWindow($hwnd, $hdc, 2)
$g.ReleaseHdc($hdc)
$g.Dispose()
if (-not $ok) { throw "PrintWindow failed" }
$bmp.Save("F:\_RAMShaper\docs\evidence\2026-10-24-rs012-profiles-ui\profiles-page.png")
$bmp.Dispose()
Write-Output "captured ${w}x${h} at ($($r.Left),$($r.Top))"
