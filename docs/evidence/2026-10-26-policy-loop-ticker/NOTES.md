# Policy-loop ticker — live safety feed + AT-14 inheritance probe, 2026-10-26

Rig: jmpat — Windows 11 Pro build 26200.9711, standard user.

## Gap closed

0.3.28 wired the pure policy loop but `serve()` built it transiently for
startup reconciliation and dropped it: **no `tick()` or `observe_safety()`
ever ran in the shipped agent**. TTL expiry, the child-spawned /
foreground-return / stale-input trips, due retries and missed-deadline
accounting existed only in tests. This slice wires the loop for real:

- `serve()` keeps the loop alive on a **1 s ticker thread** (stop-event
  driven, joined on `AgentHandle::stop`): each pass feeds safety
  observations then ticks TTL expiry + due retries + miss accounting.
- The ticker's safety feed (`feed_safety_observations`) observes the
  outstanding Applied record's target through the R-03 adapters and is
  **stateless and event-shaped**:
  - children: an applied target had zero children at apply (the
    leaf-workload check denies otherwise), so any child row in a fresh
    scan is a new descendant → `ChildSpawned` with the child's exact
    identity and image for the AT-14 finding; a failed scan is skipped
    honestly;
  - focus: only a positive "target is foreground" observation trips;
    `Unknown` never fabricates a trip;
  - staleness: a snapshot older than the contract's 2 s apply-freshness
    window is the `FocusStale` trip with the real age. (Session lock has
    no adapter in this build and is not fed — recorded gap.)
- **Journal as the single source of truth for TTL expiry**: the loop's
  in-memory deadline mirror + `note_applied` bridge is removed — applies
  made through the gate's own coordinator (the production path) were
  invisible to the old map, found live by the new
  `ttl_expiry_restores_through_the_live_ticker` test (an action stayed
  Applied forever; the ticker now expires it). The loop's dead
  `confirm`/`coordinator` wrappers are removed with it.
- AT-14's "incompatibility is recorded": the loop now captures the
  `CompatibilityFinding` (child identity + image + timestamp, parent
  label filled from the record's intent) into a findings sink
  (`PolicyLoop::findings`) — evidence for the RS-024 Stage B allowlist.
  The no-descendant-write rule holds by construction.

## Live findings (the AT-14 probe)

`child_spawned_after_apply_is_inherited_then_trips_the_restore` (live,
twice-stable): a consented `powershell.exe` parent (no children at apply)
is applied Below Normal through the gate; 3 s later it spawns a `ping`
child; the test proves the **child inherited Below Normal at creation**
(the compatibility risk the Stage B allowlist exists to test), then the
ticker's child observation trips the guardrail and the **parent is
restored to Normal** while the descendant receives no write.

Fixture note recorded for the next session that spawns children from a
script: PowerShell's `Start-Process` defaults to ShellExecute semantics —
the child can be launched through the shell broker and Toolhelp then does
NOT report the PowerShell process as the parent (observed live: zero
same-parent rows). `$psi.UseShellExecute = $false` forces a direct
CreateProcess and deterministic parenting.

## Gates (2026-10-26)

- `cargo fmt -p ramshaper-agent` clean; `cargo clippy -p ramshaper-agent
  --all-targets -- -D warnings` clean;
- agent crate **59 passed, 0 failed** across lib (28) + bin + ipc (15) +
  policy_boundary (3, the RS-026 session's committed probes) +
  policy_ipc (8, both new live tests green) — `test-run.txt`;
- full workspace: `workspace-run.txt`. Version 0.3.35; ADR-039.

## Boundary (recorded, not hidden)

- The 1 s tick granularity means a trigger's restore can land up to ~1 s
  after the observation — well inside the 2 s verification target, which
  remains accounted (missed deadlines counted, never fabricated).
- `SessionLocked` stays unfed (no adapter); the trip is reachable via the
  guardrail tests and the observation vocabulary, not yet from the OS.
- The loop and the gate each own a `RestoreEngine` over the same journal;
  retry attempt-counts are per-engine, but compare-before-restore keeps
  any interleaving safe (a second engine's attempt finds the value
  already restored → zero writes).
