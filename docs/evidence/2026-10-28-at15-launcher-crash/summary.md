# AT-15 — launcher-crash lifecycle trace (in-process portion), 2026-10-28

Captured live with the debug build at this commit. Files:

- `live-tests.txt` — `cargo test -p ramshaper-windows --test at15_lifecycle`
  (2 tests, green, ~8 s serial).

## What the trace pins

The crash stages of AT-15 discretize into three, and the two flanks are
now traced live (`crates/ramshaper-windows/tests/at15_lifecycle.rs`):

1. **Before creation** — `launcher_force_terminated_before_creation_
   leaves_nothing`: a launcher force-terminated before creating anything
   reports zero children (nothing was ever created; there is nothing to
   orphan).
2. **Mid-creation** — not externally interruptible by construction: with
   the creation-time `PROC_THREAD_ATTRIBUTE_JOB_LIST` (RS-020) membership
   and birth are one syscall, so no launcher crash can observe a
   half-created child. This absence IS the no-suspended-orphan guarantee;
   the superseded suspended-create/assign design had the window, this
   mechanism does not.
3. **After creation** — `launcher_force_terminated_after_managed_
   creation_leaves_the_child_unaided`: the helper launcher creates the
   fresh advisory job + managed child (ping), reports the child's PID on
   stdout, and is then **force-terminated** (`taskkill /F` — the same
   OS-level fact as a crash: every handle closed by the OS; chosen over
   helper-side `abort()` because that can pop an interactive WER dialog
   on this machine). The parent then verifies, across the crash:
   - **exactly one child was ever reported** (no automatic duplicate
     launch — now traced across a real process boundary, not only pinned
     by the consumed-consent token);
   - the child is still alive immediately after its launcher's death;
   - the child is **still in a job** (`observe_membership.in_any_job`) —
     the job object survives while a member exists, so the association
     survives the monitoring process's death (the non-detachable fact
     across a crash, complementing RS-023's graceful removal);
   - the child **completes unaided** (bounded wait — a suspended orphan
     would never signal, and kill-on-close would have ended it at the
     launcher's death).

## Method note

The "launcher" is a re-exec of the test binary itself (env-var helper
branch, `--exact` filter, `--nocapture` so the pid line reaches the pipe
live). Two implementation gotchas pinned during development, both recorded
in the test source: the stdout reader must stream lines (waiting for EOF
deadlocks — EOF requires the kill, the kill requires the pid line), and
the pid line must be matched ANYWHERE in the line because libtest's
partial `test <name> ... ` prefix (no trailing newline) concatenates onto
it.

## Remaining for full AT-15

The instrumented-rig staged-failure matrix (a debugger-injected crash at
arbitrary instruction boundaries inside CreateProcessW) remains rig work;
the creation-time attribute design makes the only interruptible stages
the two flanks traced here.
