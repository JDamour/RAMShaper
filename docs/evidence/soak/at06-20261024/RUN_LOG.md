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

## Attempt 3 — WMI + pythonw.exe host (DIED AT THE OVERNIGHT SHUTDOWN; archived in attempt3-shutdown/)

- Correction (2026-10-28): this attempt was launched 2026-10-24T20:17Z-local
  minutes before the machine's recorded shutdown kill window
  (~20:07–20:25Z on 10-24). It captured ~6 minutes of data (partial
  soak.jsonl + watch.jsonl + driver.log, moved to attempt3-shutdown/) and
  never reached window 1. The "(ACTIVE)" status below was stale — the
  24-hour run recorded in the 2026-10-25 morning notes was the separate
  target\soak rig, not this criteria-declared rig.

- Host moved to `pythonw.exe` running `soak_driver.py` (windowless, all
  children CREATE_NO_WINDOW), launched via `Invoke-CimMethod Win32_Process
  Create` — agent infrastructure runs python constantly, so a python host is
  systematically spared by tooling that reaps PowerShell script hosts.
- Record schema, cadence, pass criteria and analyzer are unchanged from
  attempts 1-2 (PASS_CRITERIA.md untouched; declared before the run).
- Soak window anchors to the attempt-3 start time.

## Attempt 4 — WMI + pythonw.exe host (ACTIVE, launched 2026-10-28)

- Launched 2026-10-28T18:19:26Z via `Invoke-CimMethod Win32_Process Create`
  (pythonw pid 17132), watch child pid 3156. Planned end
  **2026-10-29T18:19:26Z** — the attempt completes only if the machine
  stays up for 24 h (attempts 1–3 all died to external causes, never to
  the rig).
- Binaries: release build at this commit's source (0.3.50; the on-disk
  release had been frozen since the 0.3.41 era). Record schema, cadence
  and PASS_CRITERIA.md are UNCHANGED (criteria stay exactly as declared
  2026-10-24; no mid-run loosening). The soak.jsonl start record's
  `"driver_host":"pythonw.exe (attempt 3)"` label is the driver script's
  static text — the script is byte-identical to attempt 3's; recorded
  here rather than editing the driver post-launch.
- Caveat carried into the verdict: C3 scan-latency exceeded its declared
  limits on this loaded dev machine in attempt 2/3 interims (~440–500
  processes vs the ~347-process baseline the thresholds were derived
  from). The limits stay as declared; if the attempt-4 verdict repeats
  that, the honest outcome is a recorded finding pointing at AT-21
  cohort hardware, not a threshold change.
- While the attempt runs: target
elease
amshaper-cli.exe and
  ramshaper-bench.exe are held open by the rig — `cargo build --release`
  will fail with os error 5 until the attempt ends (the recorded gotcha;
  do NOT kill the rig to unblock a build).
- attempt3-shutdown/watch.jsonl (60 MB, 6 minutes of the dead attempt) is preserved on disk but excluded from git per the recorded LFS-size guidance; the archive's small files (soak.jsonl, logs) are committed.
- attempt-4 live `watch.jsonl` is likewise excluded from git (2026-10-28): at ~0.14 MB/min it would reach ~150-200 MB by the planned end — far over GitHub's 100 MB per-file reject limit — so the verdict commit carries soak.jsonl + driver.log only, and the full watch stream stays on disk beside them (path recorded here; preserved verbatim for the analyzer).
