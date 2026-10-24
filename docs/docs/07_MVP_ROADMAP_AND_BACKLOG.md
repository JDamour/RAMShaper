# RAMShaper — Roadmap, Backlog and Release Gates

**Revision:** 0.2 • **Date:** 2026-10-22 • **Status:** planned work; every ticket is unimplemented.
No calendar commitments. Dependencies and evidence determine progression.

## Milestones

| Milestone | Scope | Gate |
|---|---|---|
| M0 / first CLI | RS-001–RS-006 only; Windows read-only diagnostics | G0: AT-01–AT-05 |
| M1 / observation | Process metrics, bounded history, pressure_v1, initial observer benchmark | G1 |
| M2 / native UI | Slint feasibility, dashboard, inert profiles and local IPC | G2 |
| M3 / policy alpha | One confirmed CPU-priority action, recovery, synthetic comparison | G3 |
| M4 / managed launch | Creation-time Job launch and advisory monitoring | G4 |
| M5 / evidence and distribution | Benchmark wizard, independent reruns, packaging | G5 |

Gates and case definitions are authoritative in [15](15_ACCEPTANCE_TEST_MATRIX.md). Resolve the relevant open decisions in [00](00_IMPLEMENTATION_READINESS.md) before releasing a slice.

## Ticket dependency and completion table

Each row must produce implementation plus the stated evidence. Ticket numbers are identifiers, not a total execution order. Existing RS-001–RS-029 IDs are retained; RS-030/031 cover previously missing contracts.

| Ticket | Deliverable / completion evidence | Depends on |
|---|---|---|
| RS-001 | Root Cargo workspace, pinned Rust/MSVC/SDK expectations, lockfile and Windows CI. Format/check/clippy/tests run with recorded outputs | Read specs; choose build/toolchain baseline |
| RS-002 | Typed metric/status/capability and read-only adapter contracts; deterministic fake adapter/clock; exact units and failure fixtures | RS-001 |
| RS-003 | Windows PID + creation identity, owner/session context; exit/reuse cannot transfer authority | RS-002 |
| RS-004 | Non-admin Windows x64 build/launch smoke test, CLI version/OS metadata; no service installation | RS-001 |
| RS-005 | Global memory/commit counters and `info --json`; checked math, missing/error output and raw API comparison | RS-002,004 |
| RS-006 | Cancellable low-memory watch; no level-signaled busy loop; 1,000 start/stop handle test | RS-002,004 |
| RS-007 | Accessible process working set/private commit, CPU and process I/O; proper deltas and unavailable reasons | G0, RS-003 |
| RS-008 | Bounded sampling/retention/queues, explicit gaps/truncation, optional persistence; 24-hour soak | RS-005–007 |
| RS-009 | pressure_v1 transitions/evidence and Unknown; no percent/benefit invention | RS-008 |
| RS-010 | Slint feasibility and native shell; keyboard/Narrator/DPI and visible/hidden footprint evidence | G1, R-02, RS-030 |
| RS-011 | Accessible application table, units, sorting and eligibility explanations | RS-007,010 |
| RS-012 | Inert profiles/exclusions and schema-validated atomic import/export; startup Observe Only | RS-010,030 |
| RS-013 | Action history/pending/conflict UI; disable and restore controls connect only after G3 evidence | RS-010,030,031; RS-017 for live restore |
| RS-014 | Optional read-only tray, only if overhead is justified | RS-010,025 |
| RS-015 | Eligibility matrix: owner/session, identity, foreground, exclusions and tested leaf-workload allowlist; unknown denies | RS-003,007,030; R-03/R-10 |
| RS-016 | Scoped preview/confirmation and Normal → Below Normal change; durable prepare + read-back + idempotency | G2, RS-015,019,031,024 fixture |
| RS-017 | Restore/reconcile on all triggers, bounded retry, external conflict, retained unresolved state | RS-016,031 |
| RS-018 | One-action cap, cooldown, stale-input safety trip, child/focus/session handling | RS-009,015–017 |
| RS-019 | Deterministic policy dry-run over synthetic traces, including deny/no-action cases | RS-009,015,030; no OS writes |
| RS-020 | Creation-time job-attribute spike; interrupted launch/nesting tests and no suspended orphan | G3; R-06 |
| RS-021 | Job membership/child observation and compatibility reporting; no forced breakaway or unmanaged retry | RS-020 |
| RS-022 | Group accounting/advisory thresholds; no hard/time/process-count/kill limits; app survives agent exit | RS-021 |
| RS-023 | Launch consent and explicit non-sandbox/non-detachable semantics; remove monitoring only | RS-020–022 |
| RS-024 | Stage A: measurement fixtures/observer harness during M1. Stage B: policy comparison before G3. Stage C: full wizard, quantiles/CI/export before G5 | Stage A: RS-005–008; B: RS-016–018; C: G4 |
| RS-025 | Observer overhead and leak tests on declared 4/8/16 GB cohorts; UI separate; missing cohorts unverified | RS-008,024 A; repeat UI portion after RS-010 |
| RS-026 | IPC, identity, storage, export/privacy and dependency review at each changed boundary | RS-030; RS-031/020 when those capabilities exist |
| RS-027 | Packaging/provenance, optional startup, signing decision, uninstall recovery | Release-appropriate gates; R-09 |
| RS-028 | Running-build README, contributor guide, FAQ/privacy, issue/report templates | Completed slice evidence; R-09 before contributions |
| RS-029 | Closed alpha, compatibility matrix and three workload classes independently rerun; negative outcomes retained | RS-024–028 |
| RS-030 | Session agent lifecycle, bounded/versioned IPC, idempotency, capability reporting and config schemas | G0, RS-002,003 |
| RS-031 | SQLite journal, durable intent, startup reconciliation, pinned recovery, crash/disk-full/schema tests | RS-003,030; fake actuator first |

RS-016 and RS-031 are developed behind disabled actuation until recovery tests pass; no policy build is released between apply and restore completion. G2's read-only UI need not wait for live action controls. No dependency on a complete benchmark wizard is required to build early measurement fixtures.

## First implementation increment

When implementation is requested, complete RS-001–RS-006 and stop at a reviewable G0 deliverable. Implement `info`, `low-memory-watch`, `--version`, `--help` and their text/JSON/error contracts. Do not add Job launch, priority setters, UI, resident agent or placeholder future crates.

Expected checks from the future workspace root:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --target x86_64-pc-windows-msvc --locked
```

These are planned commands; there is no Cargo workspace to run them against today. Pin a compatible toolchain and record exact invocations/results rather than copying a claimed pass.

## Definition of done

A ticket is done when its contract is implemented, relevant success/failure cases pass, evidence identifies platform/permissions/revision, user-facing errors and limitations are accurate, and affected docs/ADRs agree. Test actual boundaries and failure behavior; do not substitute coverage percentage for safety evidence.

Overhead and device tests apply at the slice gate; small pure-logic changes do not each require the full hardware matrix. Re-run broader tests when a changed boundary or failure justifies them.

Release sequence: **0.1 observation → 0.2 controlled policy → 0.3 managed launch → 0.4 complete evidence workflow**. No automated optimization, hard quotas, cloud backend or AI model is hidden inside these tickets.