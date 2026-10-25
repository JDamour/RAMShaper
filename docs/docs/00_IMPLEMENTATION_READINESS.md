# RAMShaper — Implementation Readiness

**Revision:** 0.2 • **Reviewed:** 2026-10-22 • **State:** specifications only; no implementation evidence.

## 1. What is ready

The next implementation increment is a Windows x64, current-user, read-only CLI: workspace, typed adapter, process identity, build smoke test, global memory counters and low-memory notifications (RS-001–RS-006). Its contract is bounded enough to implement when implementation is requested. No GUI, resident service, process mutation or managed launch belongs in this increment.

All acceptance cases in [15_ACCEPTANCE_TEST_MATRIX.md](15_ACCEPTANCE_TEST_MATRIX.md) were **Not run** as of documentation revision 0.2. On 2026-10-23 the first increment (RS-001–RS-006) was implemented and G0 cases AT-01–AT-05 were executed on one identified Windows build; their evidence and limitations are recorded in [16_IMPLEMENTATION_EVIDENCE.md](16_IMPLEMENTATION_EVIDENCE.md). AT-06 and later remain **Not run**. API documentation checks establish feasibility, not working software. The overhead targets and benefit hypotheses remain unmeasured.

## 2. Document authority and change control

The individual numbered documents are canonical. Apply the safety constraints in [09](09_SECURITY_PRIVACY_AND_SAFETY.md), scope in [02](02_PRODUCT_REQUIREMENTS.md), and detailed behavior in [14](14_RUNTIME_CONTRACTS.md) together. [04](04_WINDOWS_API_IMPLEMENTATION.md) supplies API facts; [15](15_ACCEPTANCE_TEST_MATRIX.md) supplies test cases; [07](07_MVP_ROADMAP_AND_BACKLOG.md) supplies dependency order. [08](08_BENCHMARKS_AND_VALIDATION.md) governs performance claims.

The written UI specification is normative for interaction behavior. PNGs are visual concepts only. The original product research document is historical; the master reference is a reading guide, not a second copy of the specifications.

If canonical documents disagree, record and fix the disagreement before implementing that behavior. Verified API facts can invalidate a design assumption but cannot waive a product safety constraint. Use these statuses consistently:

- **Required:** a product or safety constraint.
- **Accepted for planning:** selected design, still requiring implementation tests.
- **Proposed:** candidate requiring evidence before adoption.
- **Open:** unresolved decision with a named gate.
- **Measured:** accompanied by reproducible commands, environment and artifacts.

Change the PRD, affected contract, acceptance cases, tickets and ADR together when behavior changes. Record newly checked source pages individually; do not imply the whole source register was revalidated.

## 3. Release boundaries

| Release slice | Included | Required evidence before release |
|---|---|---|
| First CLI increment | RS-001–RS-006, local text/JSON diagnostics | AT-01–AT-05; build and error-path evidence on an identified Windows build |
| 0.1 Observation | Process inventory, bounded history, qualitative signals, native UI, inert profile settings, observer baseline | G1/G2; unavailable/stale states, accessibility, overhead report |
| 0.2 Controlled policy alpha | One manually approved temporary CPU-priority action; journal and recovery | G3; all policy/recovery tests and synthetic-workload comparison |
| 0.3 Managed launch beta | New ordinary desktop processes in advisory Job groups | G4; compatibility and launcher crash tests |
| 0.4 Evidence workflows | User-facing benchmark wizard and reports | G5; independent reruns in three workload classes |

Benchmark instrumentation starts during observation. It is not postponed until 0.4; only the complete end-user wizard is later. Automatic recurring policies, memory-priority writes, EcoQoS and advanced tuning are deferred beyond the first controlled-policy alpha.

## 4. Decision and risk register

Owners below are responsibilities, not claims that people have been assigned. No decision is closed merely because it has a proposed answer.

| ID | Open issue / risk | Owner | Evidence needed / gate | Safe fallback |
|---|---|---|---|---|
| R-01 | Exact Windows builds and toolchain support | Platform maintainer | Pin Rust/MSVC/SDK and record Windows edition/build in RS-001/004; choose supported build matrix before G1 release | Report tested build only; no universal Windows claim |
| R-02 | Toolkit licensing, accessibility and UI overhead | UI maintainer / project owner | Feasibility GO recorded 2026-10-24 (doc 16 RS-010 spike); Slint license decision memo prepared (design/2026-10-24-slint-license-memo.md) — owner picks before G2; AT-19 automated portion recorded, interactive keyboard/Narrator/DPI pass open | Continue CLI; do not distribute an unreviewed GUI |
| R-03 | Reliable foreground, owner and protected-target eligibility | Platform / safety maintainer | Supported signal sources, unknown-state tests and narrow compatibility allowlist before G3 | Unknown targets cannot be changed |
| R-04 | A priority change may hurt useful background work or offer no gain | Benchmark maintainer | Preregistered latency and background completion comparisons before G3 expansion | Observe Only; keep negative results |
| R-05 | Agent crash leaves a setting active; another tool may change it | Core maintainer | Journal fault injection at every transition; compare-before-restore tests before G3 | Restore on next start; no immediate crash-recovery promise |
| R-06 | Managed launch may strand a suspended child or lose ownership on crash | Platform maintainer | Evaluate creation-time job attributes; interrupt creation and test nested jobs before G4 | Keep launcher unavailable if the safety gate fails |
| R-07 | Same-user malware can impersonate a local client | Security maintainer | Session ACL, peer validation and bounded command tests before G2/G3 | No elevated helper; do not claim isolation from same-user malware |
| R-08 | Telemetry growth and disk writes overwhelm a constrained PC | Core / benchmark maintainer | 24-hour retention/handle tests and observer overhead across cohorts before G1/G2 release | Drop old samples, report gaps, disable optional persistence |
| R-09 | Project/dependency license and release signing undecided | Project owner | Recorded license choice, attribution and package provenance before public distribution/contributions | Private local prototypes only |
| R-10 | Process respawn/inherited scheduling may outlive consent | Platform maintainer | Child-spawning fixture and external priority-owner tests before G3 | First policy allowlist limited to tested leaf workloads; never grant descendants implicit consent |

## 5. Readiness checklist

- [x] First increment separated from later policy and launch work.
- [x] Identity, missing data, clock, consent and recovery behavior specified.
- [x] Requirements mapped to tickets and acceptance tests.
- [x] Windows details affecting these decisions checked against primary documentation.
- [x] Toolchain and Windows test builds selected and recorded (2026-10-23: Rust 1.93.1, VS 2022 MSVC, SDK 10.0.26100, Windows 11 build 26200.9711 — see [16](16_IMPLEMENTATION_EVIDENCE.md)).
- [x] CLI built and acceptance cases run (AT-01–AT-05 Passed with recorded limitations — see [16](16_IMPLEMENTATION_EVIDENCE.md)).
- [x] Observation increment implemented 2026-10-24: AT-06 deterministic portion, AT-07, AT-08 core/config portion Passed; G1 still open pending the 24-hour soak, the declared 4/8/16 GiB cohort matrix (AT-21) and the G2 UI work — see [16](16_IMPLEMENTATION_EVIDENCE.md).
- [x] RS-030 session-agent/IPC slice implemented 2026-10-24 (protocol v1 named pipe, explicit DACL, peer validation, bounded subscribers); AT-17 frame-level portions Passed on a real pipe, cross-session/foreign-user peers Not run locally — see [16](16_IMPLEMENTATION_EVIDENCE.md).
- [x] AT-21 cohort classification implemented 2026-10-24 (declared 4/8/16 GiB cohorts in the closed config schema, report stamping, no-better-machine substitution guard); the validation 64 GiB machine classifies and is measured only as a non-cohort baseline — the declared-cohort matrix itself remains Not run — see [16](16_IMPLEMENTATION_EVIDENCE.md).
- [x] G2 UI feasibility evidence recorded 2026-10-24: RS-010 Slint spike **GO** (binary 10.6 MiB, idle CPU 0.60 % of one core probe-polling, working set 28.7 MiB, automatic agent-restart reconnect), UIA accessibility tree verified present (the earlier zero-children finding was withdrawn as a probe bug), AT-19 automated portion passed (keyboard activation, stale-state text legibility); Slint license decision prepared for the owner — see [16](16_IMPLEMENTATION_EVIDENCE.md) and [the license memo](../design/2026-10-24-slint-license-memo.md).
- [x] RS-012 M2 configuration slice implemented 2026-10-24 (inert profiles/exclusions in the closed schema; `config export|import` with field-level validation and atomic replace) — see [16](16_IMPLEMENTATION_EVIDENCE.md).
- [x] RS-010 Slint UI feasibility spike implemented 2026-10-24 (R-02 feasibility **GO**: overhead, reconnect robustness and binary size measured; DPI per-monitor aware); the UIA control-view tree exposes the window but zero children — recorded as the concrete AT-19 blocker — and hidden-window update pausing is deferred G2 work. Slint license selection and AT-19 remain owner-gated — see [16](16_IMPLEMENTATION_EVIDENCE.md).
- [x] RS-011 application table implemented 2026-10-24: additive `GetProcesses` IPC command (on-demand scan, cross-request rate baselines, 1,024-row cap inside the frame cap) and the `ramshaper-ui` Applications page (sortable, poll-while-visible, accessible rows) — see [16](16_IMPLEMENTATION_EVIDENCE.md) and ADR-026.
- [x] AT-19 contrast audit (programmatic portion) recorded 2026-10-24: WCAG 2.x ratios computed over every text/UI color pair in the live palette (test-parsed from the .slint source), one failing token darkened (`warning` #B87513 → #9C6200), and the invisible stock buttons restyled as palette-token `PaletteButton`s — see [16](16_IMPLEMENTATION_EVIDENCE.md). The interactive Narrator/DPI pass remains open.
- [ ] Policy, launcher, UI and release gates demonstrated.

Unchecked items are future implementation evidence, not work performed in this documentation revision.

## 6. Documentation validation performed

On 2026-10-22, local checks covered 22 Markdown files: 105 relative document/image links resolved, all 29 source IDs were defined, and no unbalanced fenced blocks or encoding replacement characters were found. The 31 backlog ticket definitions and 23 acceptance-case definitions are unique and contiguous; explicit ticket/case references resolved. Manual review aligned release order, UI modes, recovery/storage failures and the developer handoff.

This validates documentation structure and planning consistency only. No application code, Cargo build, Windows integration test, accessibility test or performance experiment was run.
