# soak-driver.ps1 - AT-06 24-hour soak driver (RS-024A/RS-025 observer-overhead).
#
# Launched by scheduled task "RAMShaper AT-06 Soak". Writes:
#   soak.jsonl    one JSON record per window ({type: start|report|error|done})
#   driver.log    human-readable progress lines
#   watch.jsonl   persistent 1 Hz low-memory-watch sample stream (channel 2)
#   watch.err.log watch stderr (start banner + graceful-stop history summary)
#   watch.pid     watch process id
#
# Pass criteria: PASS_CRITERIA.md in this directory (declared before the run).

$ErrorActionPreference = 'Continue'

$SoakDir       = 'F:\_RAMShaper\docs\evidence\soak\at06-20261024'
$BenchExe      = 'F:\_RAMShaper\target\release\ramshaper-bench.exe'
$CliExe        = 'F:\_RAMShaper\target\release\ramshaper-cli.exe'
$Iterations    = 500
$TickSeconds   = 600
$DurationHours = 24

$SoakJsonl    = Join-Path $SoakDir 'soak.jsonl'
$DriverLog    = Join-Path $SoakDir 'driver.log'
$WatchStdout  = Join-Path $SoakDir 'watch.jsonl'
$WatchStderr  = Join-Path $SoakDir 'watch.err.log'
$WatchPidFile = Join-Path $SoakDir 'watch.pid'
$TmpOut       = Join-Path $SoakDir '.tmp-bench-out.json'
$TmpErr       = Join-Path $SoakDir '.tmp-bench-err.txt'

$Utf8 = New-Object System.Text.UTF8Encoding($false)

function Log([string]$msg) {
    $line = '{0} {1}' -f ([DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')), $msg
    [System.IO.File]::AppendAllText($DriverLog, $line + "`r`n", $Utf8)
}

function AppendJsonl([object]$obj) {
    [System.IO.File]::AppendAllText(
        $SoakJsonl,
        (ConvertTo-Json -InputObject $obj -Compress -Depth 16) + "`n",
        $Utf8)
}

# --- keep the machine awake for the soak (no persistent power-setting change) ---
Add-Type -Namespace Soak -Name Power -MemberDefinition @'
[DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint esFlags);
'@
# ES_CONTINUOUS (0x80000000) | ES_SYSTEM_REQUIRED (0x00000001)
[void][Soak.Power]::SetThreadExecutionState([uint32]'2147483649')

# --- console event plumbing: ignore CTRL_C for this driver, deliver it to the watch later ---
Add-Type -Namespace Soak -Name Con -MemberDefinition @'
[DllImport("kernel32.dll", SetLastError=true)] public static extern bool FreeConsole();
[DllImport("kernel32.dll", SetLastError=true)] public static extern bool AttachConsole(uint pid);
[DllImport("kernel32.dll")] public static extern bool SetConsoleCtrlHandler(IntPtr handler, bool add);
[DllImport("kernel32.dll", SetLastError=true)] public static extern bool GenerateConsoleCtrlEvent(uint evt, uint group);
'@
# Ignore CTRL_C in this driver (protects us when we broadcast CTRL_C to the shared console).
[void][Soak.Con]::SetConsoleCtrlHandler([IntPtr]::Zero, $true)

# --- refuse to run over a prior/parallel soak ---
if (Test-Path $SoakJsonl) {
    Log "ERROR: $SoakJsonl already exists - refusing to start a second soak over it"
    exit 1
}

$startedAt = [DateTime]::UtcNow
$endTime   = $startedAt.AddHours($DurationHours)
$tsFmt     = 'yyyy-MM-ddTHH:mm:ssZ'

AppendJsonl @{
    type            = 'start'
    started_at_utc  = $startedAt.ToString($tsFmt)
    planned_end_utc = $endTime.ToString($tsFmt)
    tick_seconds    = $TickSeconds
    iterations      = $Iterations
    bench_path      = $BenchExe
    cli_path        = $CliExe
    machine         = $env:COMPUTERNAME
    user            = $env:USERNAME
    driver_pid      = $PID
}
Log "soak start; planned end $($endTime.ToString($tsFmt)); tick=${TickSeconds}s; iterations=$Iterations"

# --- start the persistent watch (channel 2: RS-007/008/009 at 1 Hz) ---
$watch = $null
try {
    $watch = Start-Process -FilePath $CliExe `
        -ArgumentList @('low-memory-watch', '--json', '--interval-ms', '1000') `
        -NoNewWindow -RedirectStandardOutput $WatchStdout -RedirectStandardError $WatchStderr `
        -PassThru
    Set-Content -Path $WatchPidFile -Value $watch.Id -Encoding ASCII
    Start-Sleep -Seconds 2
    if ($watch.HasExited) {
        Log "ERROR: watch exited immediately (exit code $($watch.ExitCode)) - see watch.err.log; bench channel continues"
        $watch = $null
    } else {
        Log "watch started pid=$($watch.Id)"
    }
} catch {
    Log "ERROR: watch failed to start: $_"
    $watch = $null
}

# --- main loop: one bench report per window, anchored to the start wall clock ---
$window = 0
while ($true) {
    $tickStart = $startedAt.AddSeconds($window * $TickSeconds)
    $waitMs = ($tickStart - [DateTime]::UtcNow).TotalMilliseconds
    if ($waitMs -gt 0) { Start-Sleep -Milliseconds ([int]$waitMs) }
    if ([DateTime]::UtcNow -ge $endTime) { break }

    # sample the watch channel's aliveness + memory (NFR-01 WS evidence)
    $watchAlive = $false; $watchWs = $null; $watchPriv = $null
    if ($watch -and -not $watch.HasExited) {
        try {
            $wp = Get-Process -Id $watch.Id -ErrorAction Stop
            $watchAlive = $true
            $watchWs   = $wp.WorkingSet64
            $watchPriv = $wp.PrivateMemorySize64
        } catch { $watchAlive = $false }
    }

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $exitCode = $null; $errText = $null; $report = $null
    try {
        $bp = Start-Process -FilePath $BenchExe `
            -ArgumentList @('observer-overhead', '--iterations', "$Iterations", '--json') `
            -NoNewWindow -RedirectStandardOutput $TmpOut -RedirectStandardError $TmpErr `
            -PassThru -Wait
        $exitCode = $bp.ExitCode
        $sw.Stop()
        if ($exitCode -eq 0) {
            $raw = [System.IO.File]::ReadAllText($TmpOut)
            try { $report = $raw | ConvertFrom-Json } catch { $report = $null; $errText = "unparseable bench output: $_" }
        } else {
            $errText = [System.IO.File]::ReadAllText($TmpErr)
        }
    } catch {
        $sw.Stop()
        $errText = "driver exception while running bench: $_"
    }

    if ($null -ne $report) {
        AppendJsonl @{
            type               = 'report'
            window             = $window
            scheduled_at_utc   = $tickStart.ToString($tsFmt)
            captured_at_utc    = ([DateTime]::UtcNow.ToString($tsFmt))
            duration_ms        = [int]$sw.Elapsed.TotalMilliseconds
            exit_code          = $exitCode
            watch_alive        = $watchAlive
            watch_ws_bytes     = $watchWs
            watch_private_bytes = $watchPriv
            report             = $report
        }
        Log ("window={0} ok scan_p50={1:F3} scan_p95={2:F3} snap_p50={3:F3} procs={4} bench_ws={5} watch_ws={6}" -f `
            $window, $report.process_scan.p50_ms, $report.process_scan.p95_ms, `
            $report.system_snapshot.p50_ms, $report.process_count_last, `
            $report.bench_self_working_set_bytes, $watchWs)
    } else {
        AppendJsonl @{
            type               = 'error'
            window             = $window
            scheduled_at_utc   = $tickStart.ToString($tsFmt)
            captured_at_utc    = ([DateTime]::UtcNow.ToString($tsFmt))
            duration_ms        = [int]$sw.Elapsed.TotalMilliseconds
            exit_code          = $exitCode
            stderr             = [string]$errText
            watch_alive        = $watchAlive
            watch_ws_bytes     = $watchWs
            watch_private_bytes = $watchPriv
        }
        Log ("window={0} FAILED exit={1}" -f $window, $exitCode)
    }

    $window++
    if ([DateTime]::UtcNow -ge $endTime) { break }
}

# --- stop the watch: graceful CTRL_C via console attach, hard kill as fallback ---
$stopMode = 'not-running'
if ($watch) {
    if (-not $watch.HasExited) {
        $graceful = $false
        try {
            [void][Soak.Con]::FreeConsole()
            if ([Soak.Con]::AttachConsole([uint32]$watch.Id)) {
                $graceful = [Soak.Con]::GenerateConsoleCtrlEvent(0, 0)  # CTRL_C_EVENT
                [void][Soak.Con]::FreeConsole()
            }
        } catch { $graceful = $false }
        if ($graceful) { $watch.WaitForExit(15000) | Out-Null }
        if (-not $watch.HasExited) {
            try { Stop-Process -Id $watch.Id -Force -ErrorAction Stop } catch {}
            $stopMode = if ($graceful) { 'forced-after-graceful-timeout' } else { 'forced' }
        } else {
            $stopMode = if ($graceful) { 'graceful' } else { 'unexpected-exit' }
        }
    } else {
        $stopMode = 'already-exited'
    }
}
$watchExit = $null
if ($watch -and $watch.HasExited) { $watchExit = $watch.ExitCode }

AppendJsonl @{
    type             = 'done'
    completed_at_utc = ([DateTime]::UtcNow.ToString($tsFmt))
    windows_recorded = $window
    watch_stop_mode  = $stopMode
    watch_exit_code  = $watchExit
}
Log "SOAK COMPLETE windows=$window watch_stop=$stopMode watch_exit=$watchExit"

# clear ES_SYSTEM_REQUIRED (leave ES_CONTINUOUS with no system-required flag)
[void][Soak.Power]::SetThreadExecutionState([uint32]'2147483648')
exit 0
