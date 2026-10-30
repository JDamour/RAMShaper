# Evidence — journal read-failure safety (contract-14 audit follow-up), 2026-10-30

Environment: same dev machine jmpat as every recorded slice (Windows 11
26200.9711, i9-12900HK, 64 GiB, standard user, Rust 1.93.1 pinned). All
commands below were run from the workspace root `F:\_RAMShaper` on
2026-10-30 against the working tree of this slice; the raw logs are the
files in this directory. Nothing is projected.

## Commands and outcomes

| Command | Log | Outcome |
|---|---|---|
| `cargo fmt --all -- --check` | `fmt.log` (empty output) | exit 0 — clean |
| `cargo check --workspace --all-targets --locked` | `check.log` | exit 0 — `Finished dev profile … in 52.76s`, no warnings |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | `clippy.log` | exit 0 — clean |
| `cargo test --workspace --locked -- --test-threads=1` | `test-serial.log` | exit 0 — **454 passed / 0 failed** (34 test binaries; serial = the CI `ci.yml` form) |
| `cargo build --release --target x86_64-pc-windows-msvc --locked` | `release-build.log` | exit 0 — `Finished release profile [optimized] target(s) in 3m 17s` (incremental over the warm 0.3.62 target dir) |
| `target/x86_64-pc-windows-msvc/release/ramshaper-agent.exe --version` | this file | `ramshaper-agent 0.1.0` |
| `…ramshaper-agent.exe --help` | `agent-help-release.txt` | the shipped help documents `--policy-journal`, the `ENABLE` consent and the POLICY STAGE block |
| `powershell -NoProfile -ExecutionPolicy Bypass -File docs/evidence/2026-10-30-journal-read-fault-audit/ci-smoke.ps1` | `ci-smoke.ps1` (verbatim extract), `smoke.log` | exit 0 — every CI smoke check passed against this slice's release binaries |

Test tally method: `grep '^test result:' test-serial.log | summed passed/failed`
→ 454 passed, 0 failed, 0 error. The 447 pre-existing tests plus the 7
new regression tests (all 7 names verified present in the log):

- `ramshaper_journal::read_fault_tests::armed_read_fault_fails_reads_not_writes_and_disarms_cleanly`
- `ramshaper_agent::policy_loop::tests::startup_recovery_journal_read_failure_is_not_an_empty_success`
- `ramshaper_agent::policy_loop::tests::startup_recovery_record_read_failure_is_not_a_completed_pass`
- `ramshaper_agent::policy::journal_read_failure_tests::enable_with_an_unreadable_journal_refuses_and_stays_disabled`
- `ramshaper_agent::policy::journal_read_failure_tests::disable_with_an_unreadable_journal_is_an_explicit_failure_not_zero_success`
- `ramshaper_agent::policy::journal_read_failure_tests::list_actions_with_an_unreadable_journal_is_an_error_never_an_empty_list`
- `ramshaper_agent::dispatch::policy_dispatch_tests::list_actions_journal_read_failure_is_a_typed_error_not_an_empty_list`

Live e2e suites that exercise the changed enable path on real journals
and real child processes all passed inside the serial run (agent
`policy_ipc.rs` 8 tests incl. the clean-exit trigger and the live
ticker; CLI `restore.rs` 3 tests incl. the gateless wire refusal).

## Smoke-script provenance and shell caveat

`ci-smoke.ps1` is the `CLI smoke test as standard user` step of
`.github/workflows/ci.yml` extracted verbatim (2026-10-30, at 7931411).
CI declares `shell: pwsh` (PowerShell 7); this machine has only Windows
PowerShell 5.1, so the local run used 5.1 — the script's constructs
(`& $exe`, `$LASTEXITCODE`, `ConvertFrom-Json`, integer type checks,
`New-TemporaryFile`, stderr suppression) behave identically for this
script, and the same "local is not the GitHub runner" caveat as every
recorded certification applies.

## Not run (recorded, not hidden)

- The GitHub-hosted CI run (billing still broken as of 0.3.62; the same
  local certification caveat applies).
- A bit-level OS SQLite read failure (e.g. torn page on real storage):
  the fault seam injects at the journal API boundary, the accepted
  pattern from the existing `set_fault_page_cap` disk-full seam.
- Provenance regeneration (`scripts/provenance.ps1`): not part of this
  audit's scope; the next close-out slice owns it.
