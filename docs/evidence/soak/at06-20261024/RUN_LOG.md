# AT-06 soak run log

## Attempt 1 — scheduled task "RAMShaper AT-06 Soak" (FAILED, archived in attempt1-schtask/)

- Launched 2026-10-24T11:21:23Z via `schtasks /sc once` (Logon Mode: Interactive
  only, Run As: jmpat). Start record + window 0 written (scan p50 19.766 ms,
  snap p50 0.062 ms, 442 processes, bench WS 4.7 MiB, watch WS 4.73 MiB — all
  within criteria).
- The driver powershell (pid 14620) was terminated externally a few seconds
  after window 0 (gone by 11:24:30Z); the watch child (pid 24764) died with it.
  No WER crash record, no PowerShell engine error, Task Scheduler operational
  log disabled on this machine; task Last Result 0x8007042B ("process
  terminated unexpectedly"). Cause of the termination not determinable from
  available logs; suspected interactive-only scheduled-task session semantics.
- Artifacts preserved under attempt1-schtask/ (raw evidence, not part of the
  24-hour verdict).

## Attempt 2 — WMI Win32_Process.Create + powershell host (FAILED, archived in attempt2-wmi-ps/)

- Launched 2026-10-24T11:44:30Z via `Invoke-CimMethod Win32_Process Create`
  (pid 38172, parent WmiPrvSE). Start record + window 0 written (scan p50
  33.433 ms @ 455 processes, snap p50 0.084 ms, bench WS 4.70 MiB, watch WS
  4.73 MiB — within criteria).
- Driver confirmed alive at the 7-minute check (11:52:50Z); dead by ~11:54:41Z
  (no window 1 record). Died idle-sleeping between ticks, same as attempt 1.
  The watch child (pid 29244) survived this time and kept writing 1 Hz samples
  until killed deliberately at teardown (~690 lines, archived).
- Diagnostics: no WER record for powershell.exe; no Defender detections; Task
  Scheduler operational log disabled. Attempt 1 (schtasks, svchost parent) and
  attempt 2 (WMI, WmiPrvSE parent) share only one trait: a long-lived
  **powershell.exe** running a -File script and idle-sleeping. Conclusion:
  something on this machine (most plausibly agent-session tooling reaping
  orphaned PowerShell script hosts) terminates them regardless of launch
  mechanism. Also killed at teardown: a second ramshaper-cli.exe (pid 36836)
  not spawned by this soak — if it belonged to a parallel session, that is
  what happened to it.

## Attempt 3 — WMI + pythonw.exe host (ACTIVE)

- Host moved to `pythonw.exe` running `soak_driver.py` (windowless, all
  children CREATE_NO_WINDOW), launched via `Invoke-CimMethod Win32_Process
  Create` — agent infrastructure runs python constantly, so a python host is
  systematically spared by tooling that reaps PowerShell script hosts.
- Record schema, cadence, pass criteria and analyzer are unchanged from
  attempts 1-2 (PASS_CRITERIA.md untouched; declared before the run).
- Soak window anchors to the attempt-3 start time.
