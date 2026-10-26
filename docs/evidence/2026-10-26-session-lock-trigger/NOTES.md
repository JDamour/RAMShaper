# Session-lock restore trigger — WTS adapter + ticker feed, 2026-10-26

Rig: jmpat — Windows 11 Pro build 26200.9711, standard user, interactive
session (attended run).

## What was built

The last contract 14 §6 restore trigger that could not fire live: the
`SessionLocked` observation and its guardrail trip existed since 0.3.27
and the loop mapped the trigger since 0.3.28, but **no OS adapter
produced the observation** — the 0.3.35 ticker feed recorded it as a
gap. This slice completes the matrix:

- core `SessionLockObservation { Locked, Unlocked, Unknown { reason } }`
  (alongside `FocusObservation`; every failure is `Unknown`, never a
  guess);
- `ramshaper_windows::eligibility::observe_session_lock()` — own session
  via `ProcessIdToSessionId`, then `WTSQuerySessionInformationW` with
  `WTSSessionInfoEx` (server handle `None` = current server; the
  `WTSCurrentServerHandle` constant does not exist in windows-rs 0.62),
  reading `WTSINFOEXW.Data.WTSInfoExLevel1.SessionFlags` against the
  documented `WTS_SESSIONSTATE_LOCK` (0) / `WTS_SESSIONSTATE_UNLOCK` (1)
  constants, buffer freed via `WTSFreeMemory` on every path; level/size
  and undocumented values are `Unknown`. Documented pre-Win8 flag
  reversal is noted; build 26200 uses the correct semantics;
- the 0.3.35 ticker feed polls the adapter each pass while an action is
  outstanding: only a positive `Locked` observation trips; `Unknown`
  never fabricates.

## Evidence

- **Live adapter probe** (`session_lock_probe_is_definite_on_an_interactive_rig`,
  in the windows crate): on this attended interactive rig the WTS query
  resolves to `Unlocked` ("session is unlocked (expected for an attended
  run)"). `LockWorkStation` is deliberately never called in tests — it
  would lock the operator's real screen.
- **Loop trip** (`session_lock_trips_the_restore`, agent lib): the
  `SessionLocked` observation restores the outstanding Applied record —
  the loop side of the path.
- **End-to-end boundary (recorded, not hidden):** the full
  lock-the-session → ticker-trips → restore chain is covered by parts
  (adapter probe + loop trip) and the 2-line feed glue is
  inspection-verified; it is NOT forced live in a test, because the only
  way to lock the session programmatically (`LockWorkStation`) would lock
  the operator's real desktop. The feed's other four observations use the
  same shape and were live-proven in 0.3.35.

## Gates (2026-10-26)

- `cargo fmt` on the three touched crates; clippy
  `--all-targets -- -D warnings` clean on core, windows, agent;
- windows crate **45 passed, 0 failed** (+1 probe); agent crate **60
  passed, 0 failed** (+1 loop trip); full workspace **373 passed, 0
  failed** (`workspace-run.txt`; includes the bench crate, whose
  in-flight prereg module compiled at run time).

Version 0.3.36; ADR-040.
