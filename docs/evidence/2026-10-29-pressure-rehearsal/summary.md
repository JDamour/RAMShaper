# Pressure-evidence rehearsal (runbook P3) — 2026-10-29

The AT-06 soak attempt 4 died the night before (0.3.58), removing the
reason this rehearsal had been deferred: it was off-limits while the soak
measured the machine. This was a bounded, stepped, fully-reversible
rehearsal of the Stage B trial runbook's P3 precondition — can the
low-memory condition (`Elevated`) actually be reached on this machine,
and how?

## Method

`pressure_consumer.py` allocated and TOUCHED (faulted in) memory in
3 GiB steps with a hard commit-margin guard (never below ~2.9 GiB free
commit; absolute cap 18 GiB), a settle window per step, its own
PID-file for explicit-PID stop (never image-name kills), and full
release on exit. `ramshaper-cli low-memory-watch --json` (1 Hz) recorded
the whole window.

## Result: P3 is NOT reachable on this machine as configured

- Baseline: 63.7 GiB physical (30.7 available), commit limit 67.7 GiB
  with **54.1 GiB already charged — only ~13.7 GiB of commit headroom**.
- The consumer reached **9 GiB touched** before the guard stopped it
  (free commit 3.97 GB — the next 3 GiB step would not fit safely).
- Available physical at peak: ~21.7 GB. The low-memory notification
  **never fired**: the watch recorded 248 samples, zero `elevated` lines
  (the classifier's 3-consecutive-samples/≥2 s rule never had input).
- Full recovery after release (30.4 GiB available again; the watch
  stayed operational throughout — 248 samples, no false elevated, clean
  cadence).

`machine-state-after.txt` holds the post-run state and the top ten
commit holders: the ~54 GiB charge is **distributed** across ordinary
desktop workloads (dwm 1.0 GiB, vlc 1.0 GiB, Unity 1.0 GiB, claude 0.8,
chrome 0.8, Defender 0.7 …) — there is no single whale; closing "one
app" frees ~1 GiB.

## What this means for the trial (runbook P3 updated)

On this machine as configured, `Elevated` **cannot be reached by new
allocations alone** — the kernel's low-memory condition needs available
physical far below what ~14 GiB of commit headroom can consume. Trial
day needs one of:

1. **Free the existing charge**: ~54 GiB is distributed across dozens of
   desktop processes; reaching the kernel's threshold by closing
   applications means closing most of them (operator's call, and the
   freed amount must exceed the 14 GiB headroom *plus* drive available
   physical low — tens of GiB).
2. **Enlarge the pagefile** (admin): the commit limit rises, more
   allocation can be touched, availability drops further. This machine
   is standard-user — admin help is required, and the threshold's
   actual value remains empirical (not documented by Microsoft).
3. **Run the trial on a machine where the condition is reachable** —
   the same conclusion the AT-06 suspend deaths point at: the declared
   cohort hardware solves both problems at once.

## Files

- `pressure_consumer.py` — the bounded consumer (guard + PID file +
  release-on-exit).
- `consumer.log` — the step log ("commit guard at step 3: free virtual
  3974028 KB").
- `watch-capture.ndjson` / `watch-stderr.txt` — the 1 Hz recording
  (248+ samples, zero elevated, clean recovery).
- `machine-state-after.txt` / `top-committers.ps1` — the post-run state
  and the commit-holder table.
