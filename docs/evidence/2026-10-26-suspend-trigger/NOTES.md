# Suspend/resume restore trigger — interrupt-time divergence, 2026-10-26

Rig: jmpat — Windows 11 Pro build 26200.9711, standard user.

## What was built

The last contract 14 §6 trigger with no production path: `RestoreTrigger::Suspend`
existed in the coordinator (tested) but nothing produced it — AT-11 names
"suspend" among the restore triggers. The agent is windowless, so the
event-driven routes (`RegisterSuspendResumeNotification`,
`WM_POWERBROADCAST`) are unavailable; the design (ADR-042) detects the
sleep from the **interrupt-time domain**:

- `ramshaper_windows::power::interrupt_time_pair()` — `GetTickCount64`
  (advances through sleep) and `QueryUnbiasedInterruptTime` (pauses
  during sleep; 100 ns → ms), read back-to-back; a failed unbiased query
  is `None` (skip that tick, never fabricate). New workspace feature
  `Win32_System_WindowsProgramming` on the pinned windows crate.
- core `ResumeDetector` (pure, in `guardrails`): the difference
  (biased − unbiased) equals accumulated sleep since boot; growth of more
  than `RESUME_SLEEP_GAP_MS` (1 s, against ~15.6 ms counter granularity)
  between two observations means the system slept in between. Immune by
  construction to wall-clock changes (no wall clock read) and to the
  agent itself being suspended (both counters are system-wide).
- vocabulary: `SafetyObservation::SystemResumed { sleep_ms }` →
  `TripTrigger::SystemResumed` (code `system_resumed`) → the loop maps to
  `RestoreTrigger::Suspend` — the agent can only observe the RESUME; the
  contract's journal reason is the suspend that caused it.
- the 0.3.35 ticker holds one detector for the agent's lifetime and
  samples it **every pass, before the outstanding-action early-return**
  (a stale baseline would misread the first post-idle jump); a detected
  resume is fed even when nothing is outstanding (`observe_safety`
  refuses it honestly).

## Evidence

- **Live adapter probe** (`pair_is_available_and_stable_while_awake`):
  both counters advance while awake and the sleep accumulator is stable
  (< 1 s growth across a 60 ms gap) on this rig.
- **Pure detector** (`resume_detector_needs_a_baseline_and_a_real_jump`):
  baseline anchoring, sub-threshold jitter ignored, the jump reported
  with its size, post-resume stability, and counter-reset treated as a
  new baseline (never a trip).
- **Loop trip** (`system_resume_trips_the_suspend_restore`): the
  observation restores the outstanding Applied record; with nothing
  outstanding the same observation is refused (no fabricated restore).
- **End-to-end boundary (recorded, not hidden):** a real sleep is never
  forced in tests — it would sleep the operator's machine. The chain is
  parts-covered (probe + detector + trip) and the ticker glue is
  inspection-verified, the same standard as the session-lock trigger.

## Parallel-flake repair (found during this slice's gate run)

The AT-14 inheritance test failed once under `cargo test --workspace`
(parallel test threads): the full `scan_processes` probe missed the
child's whole ~10 s life because Toolhelp snapshots fail transiently
under process churn (the documented behavior, stressed by parallel
tests spawning/killing pings). The test now uses the light
`enumerate_pid_parents` map as its primary probe at 150 ms cadence with
a 25 s window and reports failed-probe counts on timeout; the full
workspace suite passes in parallel mode after the fix.

## Gates (2026-10-26)

- `cargo fmt` on core/windows/agent; clippy `--all-targets -- -D
  warnings` clean on all three;
- core guardrails 9/0 (+2), windows 46/0 (+1 power probe), agent **61/0**
  serial (+1 trip test) — `agent-run.txt`;
- full workspace **382 passed, 0 failed** in parallel mode —
  `workspace-run.txt` (includes bench, whose 0.3.38/0.3.39 landed during
  this slice).

Version 0.3.40; ADR-042.
