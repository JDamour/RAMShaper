# G3 consent path — session consent + clean-exit trigger, 2026-10-26

Rig: jmpat — Windows 11 Pro build 26200.9711, standard user.

## What was built

The policy gate existed behind the API but the shipped agent could never
reach it (`serve` was always called with `None`). This slice completes the
shipped surface per contract 14 §5:

- **`--policy-journal <path>`** — the only way to start the agent with the
  policy gate. The explicit path is deliberate: the operator states where
  recovery records live; there is no default location, no bare `--policy`
  flag, and no config-file route. The gate starts **disabled** (startup is
  Observe Only), with the restore loop and startup reconciliation active.
- **Typed console consent** (`ramshaper-agent::consent`): after the pipe
  is up, the agent asks the operator to type the exact word `ENABLE` on
  the agent console — strict (case-sensitive, alone on the line, trimmed);
  anything else, including end of input (non-interactive stdin), declines
  and the session stays Observe Only. The decision function is pure and
  unit-tested; a flag, a profile, an import, a reconnect or an IPC command
  can never enable the session ("A profile/import is not consent",
  AGENTS.md rule 13). The consent is per-session and never persisted.
- **Exclusions flow into the gate**: the user's configured exclusions
  (`exclusions.processes`) are passed to the gate; the production
  compatibility allowlist is empty (nothing has passed RS-024 Stage B).
- **Clean-agent-exit trigger** (contract 14 §6): `AgentHandle::stop` now
  disables the gate and restores every outstanding record through the
  restore engine **before** the workers shut down and while the pipe still
  serves, then reports `restored`/`unresolved` counts. Without the gate
  the stop path is unchanged.

## Pinned by test (+4: consent 2, parse 1, integration 2 — net +5 with the
ipc split)

- `only_the_exact_word_consents` / `everything_else_stays_observe_only` —
  the strict consent decision (EOF declines, never guesses).
- `policy_journal_flag_selects_the_policy_stage` + the usage-error case —
  the flag requires a value; no path, no gate.
- `policy_commands_with_a_present_but_disabled_gate_answer_busy_not_unsupported`
  (real pipe): an agent started WITHOUT the journal answers
  `Unsupported` (older-agent behavior); an agent started WITH the journal
  but not consented answers `Busy` naming the disabled session — the
  distinction contract 14 §7 draws between "no stage" and "stage, observe
  only".
- `stopping_the_agent_runs_the_clean_exit_trigger` (live): an applied
  action on the test's spawned `ping` child (injected Elevated + allowlist
  seam, as in the RS-016 gate evidence) is **restored to Normal by
  `handle.stop()`**, the gate ends disabled and the journal record is
  terminal `Restored`.

## Boundary (recorded, not hidden)

- The consent prompt blocks the agent's main thread on stdin by design —
  the pipe, collector and restore loop run throughout; Ctrl+C works
  independently. A headless (non-interactive) start therefore cannot
  consent; that is the honest shape for an operator act.
- Enabling remains unavailable over IPC (a reconnect never enables policy
  mode); `DisableActions` over IPC remains the revocation path.
- The eligibility allowlist stays empty in production; previews under
  real Elevated pressure would still deny every third-party target until
  RS-024 Stage B evidence populates it. The UI control surface (RS-013)
  connects only after G3 evidence.

## Gates (2026-10-26)

- `cargo fmt -p ramshaper-agent` clean; `cargo clippy -p ramshaper-agent
  --all-targets -- -D warnings` clean (after removing one unused test
  import left by the concurrent session's mid-edit in `dispatch.rs` —
  collision note in the commit);
- agent crate: **25 lib + 15 ipc + 6 integration + 5 pipe_probe-suites as
  recorded in `test-run.txt`**, all green;
- full workspace: see `workspace-run.txt`.

Commands verbatim in `test-run.txt`.
