# RAMShaper — Developer Implementation Handoff

**Revision:** 0.2 • **Date:** 2026-10-22  
This is a contract for a future implementation request. Reading it during a documentation review does not instruct an agent to begin coding.

## Objective and authority

Build RAMShaper incrementally as a safe Windows resource monitor and optional workload manager. Use documented APIs and truthful evidence. Follow the user's requested slice; do not treat the long-term blueprint as permission to implement every phase.

Read, in order:

1. [Readiness and authority](00_IMPLEMENTATION_READINESS.md).
2. [Safety](09_SECURITY_PRIVACY_AND_SAFETY.md).
3. [Requirements](02_PRODUCT_REQUIREMENTS.md).
4. [Runtime contracts](14_RUNTIME_CONTRACTS.md).
5. [Architecture](03_SYSTEM_ARCHITECTURE.md) and [Windows APIs](04_WINDOWS_API_IMPLEMENTATION.md).
6. [Acceptance matrix](15_ACCEPTANCE_TEST_MATRIX.md) and [backlog](07_MVP_ROADMAP_AND_BACKLOG.md).
7. [Benchmarks](08_BENCHMARKS_AND_VALIDATION.md), [UI behavior](06_UI_UX_DESIGN_SPEC.md), [ADRs](13_ARCHITECTURAL_DECISIONS.md) and [sources](12_SOURCES.md).

Written UI behavior is a specification; PNGs are visual references only. Historical research and the master guide do not override canonical documents. If a verified API limitation conflicts with the intended behavior, update the affected design/acceptance decision; do not silently weaken safety.

## First task when implementation is requested

Implement **RS-001–RS-006 only** at the workspace root, preserving the existing documentation paths. Deliver a Windows x64 standard-user CLI with `info [--json]`, `low-memory-watch [--json]`, `--version` and `--help`.

Create only the core/platform/Windows/CLI components needed for this increment. Pin a compatible stable Rust toolchain, MSVC/SDK prerequisites and dependencies. Use minimal official Windows binding features. No UI, resident service, Job launch, priority mutation or future-platform placeholder modules.

Demonstrate checked page-to-byte math, exact native process identity, missing/error statuses, partial exit codes, cancellable waits, handle cleanup and no busy loop while a low-memory condition remains signaled. Verify G0 cases on an identified Windows build. A mock pass does not prove real OS support.

## Later increments

- G1: process telemetry, bounded storage/queues, pressure_v1 and observer baseline.
- G2: a non-elevated session agent/IPC and native observation UI after toolkit review. Profiles remain inert.
- G3: one confirmed Normal → Below Normal CPU-priority experiment only after durable journal and restoration work together. Apply/restore code must not be released separately.
- G4: evaluate creation-time Job attributes and complete crash/nesting tests before exposing managed launch. No suspended-create fallback without an explicit design revision.
- G5: complete evidence wizard, independent workload reruns and release packaging.

No automatic continuation past the requested increment. Subsequent slices require their applicable evidence, not invented success.

## Implementation constraints

Use explicit units, typed statuses/errors and capability reasons. Keep OS handles under RAII; isolate unsafe FFI. Expected access denial, stale data, process exit and storage failures must not panic. Do not use unchecked unwraps in OS-facing runtime paths.

The core decision engine is pure and testable with fake clocks/adapters. Only one coordinator owns process-setting changes and the journal. Windows identity is PID plus creation FILETIME; it is not a monotonic timestamp. Validate and act through the same handle.

Observe Only is startup mode. Profile selection/import is not consent. Record durable intent before every setting write, verify results, retain unresolved recovery, and compare current state before restoration. Never call restoration “defaults,” overwrite an external setting knowingly, or promise immediate rollback while the agent is crashed.

No process kills, suspension, restarts, hard caps, cache/working-set trimming, High/Realtime boost, injection, kernel driver, arbitrary command RPC, elevated helper or hidden network behavior.

## Validation and deliverable

Run the commands in [07](07_MVP_ROADMAP_AND_BACKLOG.md) when their workspace exists. Add meaningful failure tests for actual boundaries. Record each case as Passed, Failed, Blocked or Not run, with actual outputs and environment.

For every increment report:

- Implemented scope and files changed.
- Exact build/test commands, exit codes and evidence paths.
- OS edition/build, account privileges, toolchain and source revision.
- Acceptance cases run and remaining gaps.
- Measured overhead/results only where actually collected.
- Known limitations and next gate.

Provide running-app screenshots only once a UI exists; do not substitute generated art. If Windows/hardware tests cannot run, mark them unverified and keep the relevant gate open. Never claim a build, API behavior or benchmark result from documentation alone.