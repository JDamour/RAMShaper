# Building RAMShaper

Implemented increments: the read-only CLI (RS-001 through RS-006, gate G0
passed), the observation increment (RS-007 through RS-009 plus the Stage A
measurement harness; G1 partially verified), the RS-030 session-agent/IPC
slice (protocol v1 over a session-scoped named pipe), and the RS-010 Slint
UI feasibility spike (an experimental, observe-only Overview window; not
yet the G2 release UI). Open items and executed
evidence: `docs/docs/16_IMPLEMENTATION_EVIDENCE.md`. There is no installer,
policy actuation or managed launch in this build.

## Prerequisites (Windows x64)

| Requirement | Pinned / recorded value |
|---|---|
| Rust toolchain | 1.93.1 (pinned in `rust-toolchain.toml`, with `rustfmt` and `clippy`) |
| Rust host target | `x86_64-pc-windows-msvc` |
| C++ toolchain | Visual Studio 2022 (MSVC v143 toolset) "Desktop development with C++" workload, or Build Tools with the same workload |
| Windows SDK | 10.0.26100.0 was used for local validation; any currently supported Windows 10/11 SDK is expected to work |
| Privileges | Standard user. No UAC elevation, no service installation, no debugging privilege |

Install Rust with [rustup](https://rustup.rs); opening the workspace makes
rustup honor `rust-toolchain.toml` automatically. Dependencies are pinned by the
committed `Cargo.lock`; build with `--locked`. The `windows` crate is pinned to
exactly `0.62.2` with a narrow feature list; `toml` is the mandated
configuration format (contract 14 §7); `slint` is pinned exactly `=1.18.1`
for the UI crate. See the root `Cargo.toml`.

## Build and validate

From the workspace root:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --target x86_64-pc-windows-msvc --locked
```

`cargo test` includes Windows integration tests in `crates/ramshaper-windows`
(real `GetPerformanceInfo`/`GetProcessTimes`/memory-resource-notification and
process-enumeration calls, a child-process exit probe, a 1000x
create/query/close handle-cycle check, two-scan rate sanity and a cancellable
watch run) and binary-level CLI tests in `crates/ramshaper-cli`. These run as
a standard user.

## CLI usage

```text
ramshaper-cli info
ramshaper-cli info --json
ramshaper-cli low-memory-watch [--json] [--interval-ms <ms>]
ramshaper-cli processes [--json] [--interval-ms <ms>]
ramshaper-cli policies --dry-run [--json] [--trace <FILE>]
ramshaper-cli launch [options] -- <EXE> [ARGS...]
ramshaper-cli config export|import ...
ramshaper-cli --version
ramshaper-cli --help
```

- `--interval-ms` default 1000 for watch (poll cadence) and 500 for processes
  (rate window between two scans); allowed range 100..=3,600,000.
- `--config <path>` loads a TOML configuration file (`schema_version = 1`,
  observe-only; invalid files exit 2 with a field-level error).
- Stdout carries data only; diagnostics go to stderr.
- Exit codes: `0` success (including a healthy Ctrl+C-cancelled watch), `1`
  runtime failure / no usable global sample, `2` usage or configuration error,
  `3` partial snapshot (an optional metric or truncation is reported).
- JSON output is one versioned object per snapshot (`schema` field); watch
  mode emits newline-delimited JSON objects; `processes` emits a
  `ramshaper.processes.v1` inventory.
- All byte values are integers in JSON; MiB/GiB formatting exists only in text
  output. Metric envelopes carry `value`/`state`, `status`, `source`,
  `observed_at_utc`, `sample_sequence`, `age_ms` and an optional `error_code`.
- Per-process `access_denied`/`exited` statuses are expected on protected
  processes and do not fail the `processes` command.
- Ctrl+C cancels waits and releases handles; the watch prints a bounded-history
  summary (stored/gaps/evicted/expired/rejected) on exit.

## Measurement harness

```text
cargo run --release --locked -p ramshaper-bench -- observer-overhead --iterations 500 --json
```

Measures per-collection latency of the read-only collector paths on the current
machine plus the harness's own working set / private commit. A machine-local
observer baseline (RS-024 Stage A), not a hardware-cohort claim.

## Session agent

```text
ramshaper-agent serve [--pipe-name <N>] [--interval-ms <MS>]   # Ctrl+C to stop
ramshaper-agent probe [--pipe-name <N>] [--subscribe <SECS>] [--request <CMD>]
```

The agent serves protocol v1 (`ramshaper.snapshot.v1` payloads) on
`\\.\pipe\ramshaper-agent-s<session>-v1` (or a configured/overridden name)
with an explicit DACL granting only the current user; every client is
re-validated against the serving session and user SID. Frames are
length-prefixed JSON capped at 1 MiB; subscribers hold at most two pending
frames with gap markers. Commands: `GetCapabilities`, `GetSnapshot`,
`SubscribeSnapshots`, `ListActions` (empty — observe only). Same-user malware
is not isolated by these checks (documented limitation R-07).

## UI spike (RS-010, experimental)

```text
ramshaper-agent serve --pipe-name rs010 --interval-ms 500
ramshaper-ui --pipe-name rs010 [--poll-ms 1000]
```

A read-only Slint Overview window (`crates/ramshaper-ui`; slint pinned
exactly `=1.18.1`, software renderer). Default transport is
`SubscribeSnapshots`; `--poll-ms <MS>` switches to one-shot `GetSnapshot`
probes at that cadence. The window renders live system resident-memory
counters, the `pressure_v1` classification with pending evidence, the
serving `agent_instance_id` (visible across agent restarts) and a
last-updated/stale indicator; state transitions log to stderr. Mode is
Observe Only — no policy controls. Known spike gaps recorded in
`docs/docs/16_IMPLEMENTATION_EVIDENCE.md`: UIA exposes the window but no
child controls (AT-19 blocker), and hidden windows keep updating
(visibility-aware pausing is G2 work). Do not rebuild while a release
`ramshaper-ui.exe`/`ramshaper-agent.exe` is still running (MSVC link
fails with os error 5 until the process exits).

## Scope and safety

Implemented capabilities are strictly read-only: system memory counters, the
low-memory resource condition, process identity, per-process memory/CPU/I-O
counters, the `pressure_v1` observation classifier, bounded in-session history
and versioned observe-only configuration. The tool never changes process
settings, priorities, working sets or jobs, and it contains no network code, no
telemetry and no update checks. See
`docs/docs/09_SECURITY_PRIVACY_AND_SAFETY.md`.

Validation evidence is recorded in `docs/docs/16_IMPLEMENTATION_EVIDENCE.md`.
