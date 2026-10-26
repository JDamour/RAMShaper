# RS-020 — managed-launch spike: creation-time job attribute, 2026-10-26

Rig: jmpat — Windows 11 Pro build 26200.9711, i9-12900HK, 64 GiB, standard
user (no elevation), Git Bash.

## What was evaluated

Doc 04 §D (R-06): the managed launcher must not strand a suspended child
or lose ownership on a launcher crash. The earlier
suspended-create/assign/resume sketch is superseded by the documented
creation-time job-list attribute. This spike implements the mechanism in
`ramshaper-windows::job` and measures it live:

- `AdvisoryJob::create` — unnamed `CreateJobObjectW` job; the module never
  calls `SetInformationJobObject`, so the job carries no limits, no
  kill-on-close, no UI restrictions (queried back, not assumed);
- `ManagedChild::spawn` — `CreateProcessW` with `EXTENDED_STARTUPINFO_PRESENT`
  and one `UpdateProcThreadAttribute` entry
  (`PROC_THREAD_ATTRIBUTE_JOB_LIST = 131085`); **no `CREATE_SUSPENDED`
  anywhere**; membership is verified through the child's own process
  handle before the spawn reports success;
- `is_in_any_job` / `AdvisoryJob::contains_process` / `limit_flags` /
  `process_ids` — the probe seam (environment preflight) and the AT-16
  "queried job flags" evidence path;
- `AdvisoryJob::assign` — the legacy post-create assignment, kept only as
  a test fixture seam (production assignment via attribute list needs no
  post-create step).

## Live findings

1. **Creation-time assignment works on this build.** A `ping` child
   spawned through the attribute list reports job membership through its
   own handle immediately after `CreateProcessW`, before the spawn call
   returns, and completes unaided (exit 0) — nothing suspended, nothing
   to resume, no unmanaged window between birth and membership.
2. **Legacy post-create `AssignProcessToJobObject` is `ACCESS_DENIED`
   (0x80070005 / code 5) on build 26200** for a same-user, already-running
   child, even with the documented `PROCESS_SET_QUOTA` right on the
   target handle. The refusal is clean (the child stays out of the job).
   This independently supports doc 04 §D's supersession: the legacy path
   is no longer reliable on current Windows; the creation-time attribute
   is the only mechanism this spike could verify.
3. **The advisory job's queried limit flags are 0** — before and after a
   managed launch (AT-16: "no kill/hard/time/process-count limits are
   configured" is shown by query, not by construction claim).
4. **Closing/dropping the job handle never terminates members** (no
   kill-on-close): the child kept running after the last job handle was
   dropped and exited 0 on its own schedule (AT-15/AT-16 "closing
   UI/agent/job handles never unexpectedly kills user apps", in-process
   shape).
5. **A failed launch leaves nothing behind**: a missing image fails
   `CreateProcessW`; the job's process list stays empty; no orphan, no
   half-managed child (AT-15 "no automatic duplicate launch / no
   suspended orphan", the failure branch).
6. **Nesting is preserved, never escaped**: with the launcher itself
   inside an advisory job, a creation-time child becomes a member of BOTH
   jobs (Win8+ nested jobs) — the child cannot be used to slip the
   launcher's environment (doc 04 §D step 6: detect, do not escape).

## Boundary (recorded, not hidden)

- The full AT-15 "interrupt the launcher at each stage" lifecycle trace
  (killing the launcher mid-`CreateProcessW`) is G4 rig work; this spike
  pins the crash-window *shape* by construction: there is no suspend
  state and membership is the same syscall as birth.
- The spike module has **no production caller** — nothing in the agent,
  CLI or UI references it (verified: no non-test call sites). The managed
  launcher stays unavailable in the shipped surface until RS-023's
  consent flow and the G4 interruption evidence exist (doc 07: keep the
  launcher unavailable if the safety gate fails).
- `AssignProcessToJobObject`'s ACCESS_DENIED finding is one rig, one
  build (26200.9711); older builds may still allow it — the test accepts
  both outcomes and asserts membership probing matches whichever happened.

## Gates (2026-10-26)

- `cargo fmt -p ramshaper-windows` clean;
- `cargo clippy -p ramshaper-windows --all-targets -- -D warnings` clean;
- `cargo test -p ramshaper-windows`: **28 lib + 15 integration + 1 pipe
  probe = 44 passed, 0 failed** (+6 new job tests over 0.3.27's 22 lib);
- full workspace run: see `workspace-run.txt` in this directory.

Commands recorded verbatim in `test-run.txt`.
