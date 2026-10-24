# RAMShaper — Project Documentation

**Revision:** 0.3.0 • **Updated:** October 24, 2026  
**Status:** read-only CLI (G0 passed) and observation increment (G1 partially verified: AT-06 deterministic, AT-07, AT-08 core/config Passed; 24-hour soak and declared cohorts open) — see [16](docs/16_IMPLEMENTATION_EVIDENCE.md). UI, policy and launch slices are unimplemented.  
**Direction:** Rust + proposed Slint UI; Windows x64 first  
**Name/license:** working name; project license and distribution review remain open

RAMShaper is a planned offline resource monitor and optional workload manager. Its intended outcome is improved responsiveness under measured contention. It cannot create physical RAM, guarantee performance gains, or security-sandbox arbitrary applications.

## Start here

Read the [implementation readiness review](docs/00_IMPLEMENTATION_READINESS.md). The first planned increment is **RS-001 through RS-006: a read-only Windows CLI**. The documentation is ready to guide that increment; no implementation gate has passed.

The [PRD](docs/02_PRODUCT_REQUIREMENTS.md) defines scope, [runtime contracts](docs/14_RUNTIME_CONTRACTS.md) define exact behavior, and [acceptance matrix](docs/15_ACCEPTANCE_TEST_MATRIX.md) defines evidence required to finish each slice. Observe Only is the startup mode. Policy and launch capabilities require separate gates.

## Canonical documentation

| Document | Purpose |
|---|---|
| [00 — Implementation readiness](docs/00_IMPLEMENTATION_READINESS.md) | Authority, release slices, unresolved decisions and risks |
| [01 — Research and feasibility](docs/01_RESEARCH_AND_FEASIBILITY.md) | Initial evidence, competitors and limits |
| [02 — Product requirements](docs/02_PRODUCT_REQUIREMENTS.md) | User stories, release scope, acceptance and non-goals |
| [03 — System architecture](docs/03_SYSTEM_ARCHITECTURE.md) | Boundaries, crate responsibilities and data flow |
| [04 — Windows API implementation](docs/04_WINDOWS_API_IMPLEMENTATION.md) | API mapping, rights, launch strategy and spikes |
| [05 — Linux/macOS roadmap](docs/05_LINUX_MACOS_ROADMAP.md) | Deferred platform capabilities; no parity promise |
| [06 — UI/UX design](docs/06_UI_UX_DESIGN_SPEC.md) | Screens, user control, accessibility and error states |
| [07 — Roadmap and backlog](docs/07_MVP_ROADMAP_AND_BACKLOG.md) | Ticket dependencies and release gates |
| [08 — Benchmarks and validation](docs/08_BENCHMARKS_AND_VALIDATION.md) | Reproducible evidence and regression criteria |
| [09 — Security, privacy and safety](docs/09_SECURITY_PRIVACY_AND_SAFETY.md) | Mandatory safeguards and recovery/privacy limits |
| [10 — Business plan](docs/10_OPEN_SOURCE_BUSINESS_PLAN.md) | Unvalidated market, pricing and licensing options |
| [11 — Developer handoff](docs/11_AI_DEVELOPER_BUILD_PROMPT.md) | Incremental implementation contract for future work |
| [12 — Source register](docs/12_SOURCES.md) | Primary URLs and per-source verification status |
| [13 — Architecture decisions](docs/13_ARCHITECTURAL_DECISIONS.md) | Required constraints, selected designs and open decisions |
| [14 — Runtime contracts](docs/14_RUNTIME_CONTRACTS.md) | Metrics, consent, journal, IPC, storage and CLI semantics |
| [15 — Acceptance matrix](docs/15_ACCEPTANCE_TEST_MATRIX.md) | Requirement/ticket/test traceability and evidence template |
| [16 — Implementation evidence](docs/16_IMPLEMENTATION_EVIDENCE.md) | Executed evidence for the first CLI increment (G0) |

The [master reference](RAMShaper_Master_Reference.md) is a reading guide. The [original product research](RAMShaper_Product_Research_Document.md) is retained as historical context and is not an implementation specification. See [CHANGELOG.md](CHANGELOG.md) for this revision.

## Delivery sequence

1. Read-only CLI and Windows error-handling evidence.
2. Process telemetry, bounded history and observer-overhead baseline.
3. Native observation UI and inert profile settings.
4. One manually approved temporary CPU-priority policy, after recovery tests.
5. Managed launch beta, after Job lifetime/compatibility tests.
6. Full benchmark wizard and evidence-backed release.

Benchmark instrumentation begins in the observation phase. No automatic optimization is authorized by selecting a profile or importing settings.

## Visual concepts

The four images in [design](design/01_dashboard.png) are generated visual references with synthetic numbers and some unapproved controls. The written UI spec governs behavior. Keep automatic restarts, periodic cache trimming, hard quotas and unmeasured benefit badges out of the implementation.

![Dashboard concept — synthetic data, not implemented](design/01_dashboard.png)

## Working in this package

The workspace has a documentation package at `docs/`, with the canonical numbered documents at `docs/docs/`. The first application increment (Cargo workspace, `crates/ramshaper-core|platform|windows|cli`, CI, `BUILDING.md`) now lives at the workspace root; the documentation was not moved. Build commands and prerequisites are in the root `BUILDING.md`; executed evidence is in [16](docs/16_IMPLEMENTATION_EVIDENCE.md).

For every implemented increment, record commands, exit codes, Windows build, privileges, source revision and evidence artifacts. Preserve failed, neutral and unavailable results. Read [AGENTS.md](AGENTS.md) before implementation. No software license or dependency approval is implied by these planning files.