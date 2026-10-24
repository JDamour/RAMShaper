# RAMShaper — System Architecture

**Revision:** 0.2 • **Date:** 2026-10-22 • **Status:** accepted for planning; runtime behavior is defined in [14](14_RUNTIME_CONTRACTS.md).

## 1. Delivery boundaries

The first CLI calls a Windows telemetry adapter directly. No daemon, IPC, database or UI is required to prove the first six tickets. Add components only when their release slice needs them.

The later desktop design uses one non-elevated agent per interactive user/logon session. The UI presents state and requests actions; only the agent's serialized policy coordinator may mutate process settings. There is no Windows service or privileged helper in the MVP.

```text
First increment:
CLI -> core types / read-only adapter -> Windows APIs

Later desktop slices:
Slint UI or CLI -> session-restricted named pipe -> user agent
                                                   |
                      +----------------------------+------------------+
                      |                            |                  |
                 collector                  policy coordinator    job controller
                      |                            |                  |
               bounded snapshots          durable journal     creation-time jobs
                      |                            |                  |
                      +------------------ Windows adapter ------------+
```

The agent owns handles and recovery. UI closure leaves an explicitly enabled resident agent running; otherwise orderly shutdown restores active settings. A UI connection is never a job lifetime requirement.

## 2. Planned repository layout

Keep the existing documentation package in place. Future sources are rooted at the workspace root:

```text
_RAMShaper/
  README.md
  Cargo.toml
  Cargo.lock
  rust-toolchain.toml
  crates/
    ramshaper-core/         # domain types, pure decisions, fake clock/adapter tests
    ramshaper-platform/     # capability, telemetry, identity and actuator interfaces
    ramshaper-windows/      # Win32 wrappers, RAII and Windows integration tests
    ramshaper-cli/          # first diagnostic surface
    ramshaper-agent/        # added for desktop; coordinator, IPC and recovery
    ramshaper-ui/           # added after UI feasibility gate
    ramshaper-benchmark/    # measurement fixtures before policy release
  ui/                      # later Slint sources
  config/                  # later versioned defaults
  tests/fixtures/
  docs/                    # existing documentation package
    docs/                  # canonical numbered documents
    design/                # illustrative images
```

Do not generate empty Linux/macOS crates, updater, installer or plugin framework in the first increment. Pin toolchain/dependencies during RS-001; no versions have been validated by this documentation change.

## 3. Responsibilities and interfaces

| Component | Inputs / outputs | Boundary |
|---|---|---|
| Telemetry adapter | `collect_system`, `collect_processes` -> typed metric envelopes | Read-only API calls; partial results permitted |
| Process registry | PID + native creation marker + owner/session | Lifecycle labels do not grant mutation permission |
| Pressure classifier | Valid snapshots + rule version -> level/evidence | Pure deterministic logic; v1 observation only |
| Policy engine | Snapshot, eligibility, mode, consent, cooldown -> proposal or denial | No OS calls; Unknown means ineligible |
| Policy coordinator | Proposal/confirmation -> action state | Serializes apply/restore/disable; prioritizes restoration over telemetry |
| Actuator | Validated open process handle + one property -> read/write/read-back | Minimal rights; expected OS failures are typed |
| Journal | Action intent/results -> durable recovery records | Must commit before mutation; pinned unresolved records |
| Job controller | Validated launch request -> identity/group/outcome | New processes only; no policy coupling |
| Benchmark engine | Preregistered runs + event samples -> report/export | Distinguishes synthetic fixtures from real evidence |
| UI adapter | Capabilities/state -> view and explicit requests | No direct setting changes or fabricated values |

Use interfaces separating telemetry and actuation so the first CLI cannot accidentally acquire mutation functionality. Isolate `unsafe` FFI and handle lifetimes in audited Windows wrapper modules.

## 4. Domain contract

Use an OS-specific creation marker, not a field named `started_at_monotonic`. On Windows, `ProcessIdentity { pid, creation_filetime_100ns }` uses `GetProcessTimes`; duration clocks are separate. Native timestamps used as identifiers retain exact precision.

Every metric has provenance, age, unit and explicit status. Distinguish physical available, system commit, process working set and private commit. A plain `Option<u64>` cannot explain access denied versus stale/unsupported; use a typed status envelope. Capabilities report supported/unavailable functions and reasons.

Separate `OperatingMode`, `ProfilePreset`, `PressureObservation`, `ConsentGrant`, `ActionRecord` and `RecoveryOutcome`. Use `RestoreRecordedState`, not “restore defaults”: rollback depends on the saved original setting. Evidence quality is categorical, not an unexplained confidence float. Full field/serialization semantics are in [14](14_RUNTIME_CONTRACTS.md).

## 5. Apply and recovery ordering

```text
Preview -> confirm -> revalidate identity / eligibility / consent
        -> durable Prepared -> OS write -> read-back -> Applied
Any restore trigger -> block new writes -> read current value
        -> original: Restored
        -> still our value: restore original + verify
        -> another value: Conflict
        -> original process absent: TargetGone
        -> unreadable or write uncertain: retain recovery, Needs attention
```

On startup, reconcile Prepared/Applied/Uncertain/RestorePending records before accepting new changes; never resume an old policy. A configuration error does not prevent recovery. Concurrent external setting writers remain a documented race; no atomic OS compare-and-swap is assumed.

The global disable command is ordered ahead of queued proposals. Requests that have not reached the OS write are cancelled after revalidation; any write already in flight is immediately followed by reconciliation. No multi-property transaction is claimed.

## 6. Storage and communication

For the initial CLI, metrics are transient and output is explicit. Later TOML stores versioned preferences, while SQLite stores action/recovery records. Optional historical aggregates are separate from the journal. Retention, quotas, deletion and export rules are in [14](14_RUNTIME_CONTRACTS.md).

IPC uses a bounded, versioned local named pipe with explicit session security and fixed commands. Authenticate the peer's user/session, then enforce target permissions and consent on every action. Same-user malware is outside the isolation guarantee; no admin capability is exposed.

Duplicate confirmations, lost replies, version mismatch, process exit, denied access, disk-full and corrupt schema are normal failure paths. Do not block sampling/UI on long exports or waits. Backpressure drops stale telemetry with gap markers, never recovery intent.

## 7. Performance and dependency strategy

Measure the collector, agent and visible UI independently. Keep one in-flight collection, bounded refresh work and cancellable waits. Do not use an unrestricted thread pool for polling. Choose async orchestration only when it simplifies tested cancellation.

Use official Windows Rust bindings with minimal features. `sysinfo` is optional convenience, not an alternate definition of memory metrics. Slint remains proposed until licensing, accessibility and footprint tests pass. SQLite durability is required before policy writes. No cloud backend, HTTP server or UI browser engine is part of the current design.

Risks and decisions are tracked in [00](00_IMPLEMENTATION_READINESS.md) and [13](13_ARCHITECTURAL_DECISIONS.md); this design does not claim that any of these contracts have been implemented.