# Documentation Changelog

## 0.3.13 — 2026-10-24 (later)

- Completed the **AT-23 G2-applicable portions** (offline/privacy boundary review, RS-026): static audit shows zero network API usage in workspace code and no network-capable crates in the Windows host dependency graph (`tokio` resolves only on the Slint UI's wasm32 path via `wasm-bindgen-futures → winit`); live socket observation shows the long-running observers hold no sockets (soak watch ~4.5 h: tcp=0 udp=0; session agent: tcp=0 udp=0); real `config export` captures (`docs/evidence/2026-10-24-at23-privacy-defaults/`) contain no identity material — only schema/mode/cadence markers plus exactly the user-supplied fields; no ramshaper scheduled task, service or Run-key entry exists (uninstall = delete folder until RS-027 packaging). Journal/uninstall-report portions recorded as not-applicable-yet (RS-031/G3). Evidence record appended to [16 — Implementation evidence](docs/docs/16_IMPLEMENTATION_EVIDENCE.md).

## 0.3.12 — 2026-10-24 (latest)

- Prepared the **AT-19 interactive-pass checklist** ([design/2026-10-24-at19-interactive-checklist.md](docs/design/2026-10-24-at19-interactive-checklist.md), linked from the AT-19 row in [15 — Acceptance matrix](docs/docs/15_ACCEPTANCE_TEST_MATRIX.md)): element-by-element Narrator expectations generated from the verified 42-element UIA inventory, keyboard walk steps, stale/disconnected legibility script, the 100–200 % DPI matrix with the flagged 200 %-at-1366×768 logical-space risk (window minimum 1024×640 vs ~683×384 logical desktop — record behavior, do not assume a pass), and explicit evidence-capture instructions. The pass itself remains a human task.

## 0.3.11 — 2026-10-24 (later)

- Implemented **RS-011** (G2 read-only application table): additive protocol-v1 command `GetProcesses` (contract 14 §7 updated) served **on demand — one fresh scan per request, never on the collector cadence** (a cached 1 Hz scan would cost ~2 % of a core and break the NFR-01 idle budget); rates span the gap since the previous scan via baselines shared across requests (first request honestly `warming_up`); responses serve the heaviest processes first and cap at 1,024 rows with an explicit `row_cap_note` inside the 1 MiB frame cap; scan failures are retryable `OsError`, agents without the command answer `Unsupported` (rendered as an explained empty table). `ramshaper-ui` gained the **Applications** page (nav item, `--page` start flag, sortable table, poll-while-visible, per-cell unavailable notes, whole-row accessible labels). +9 tests (workspace 206 green across both sessions' increments); live evidence in `target/rs011/` (459 processes served, disconnect notes) and the record in [16 — Implementation evidence](docs/16_IMPLEMENTATION_EVIDENCE.md). ADR-026 added.
- Recorded the retroactive changelog for the `--interval-ms` defect fixed earlier today (full record in [16](docs/16_IMPLEMENTATION_EVIDENCE.md)): the flag was parsed then dropped for `low-memory-watch` (M1 regression); precedence is now flag > config file > built-in default, pinned by unit + binary banner tests, verified live ("every 60000 ms").

## 0.3.10 — 2026-10-24

- Completed the same-session adversarial portion of **AT-17** (read-only IPC): new end-to-end tests for a zero-length framing prefix (silent close — no correlatable response exists) and for replay behavior (v1 has no replay cache — `request_id` is correlation only; pinned that a replayed read-only request is served identically, an accepted limitation to revisit in any state-mutating protocol v2), plus a response-side 1 MiB cap test (`response_frame` refuses oversized payloads with `Oversized`). Coverage map + raw run in `docs/evidence/2026-10-24-at17-ipc-adversarial/`; [16 — Implementation evidence](docs/docs/16_IMPLEMENTATION_EVIDENCE.md) AT-17 record updated (cross-session/remote portions remain Not run locally under the documented same-user limitation). Agent + core: 13 integration / 110 core tests green; fmt and clippy (`-D warnings`) clean.

## 0.3.9 — 2026-10-24 (latest)

- Readiness and evidence status refreshed for the G2 prep work: [00 — Implementation readiness](docs/docs/00_IMPLEMENTATION_READINESS.md) R-02 row and checklist now record the RS-010 spike GO, the withdrawn UIA blocker, the AT-19 automated portion, the license memo and the RS-012 slice; [15 — Acceptance matrix](docs/docs/15_ACCEPTANCE_TEST_MATRIX.md) header records the RS-012 portions of AT-08 and the AT-19 automated portion (interactive pass Not run).
- UI overhead budget **proposal** prepared in [design/2026-10-24-ui-budget-memo.md](docs/design/2026-10-24-ui-budget-memo.md) from the measured prototype numbers (WS < 40 MiB mean / < 50 MiB peak, private < 20 MiB, ≤ 1 % of one core idle, ≤ 16 MiB binary; hidden-window pausing required before any hidden claim; RS-014 tray only if it fits the same budget) — linked from [08 — Benchmarks](docs/docs/08_BENCHMARKS_AND_VALIDATION.md) §5, pending project selection.

## 0.3.8 — 2026-10-24 (later)

- Implemented the M2 configuration slice of **RS-012**: closed-schema `[profiles] active` (balanced/low_ram/focus/gaming_observe/custom — inert labels that share all defaults and never grant write permission) and `[exclusions] processes` (user exclusion names, shape- and count-validated) in `ramshaper-core::config`; new `ramshaper-cli config export [--config <PATH>] <DEST>` and `config import <SOURCE> --into <DEST>` — field-level validation against the closed schema and atomic replace via `fs::atomic_write`, startup stays Observe Only. 17 tests added (12 unit + 5 binary; workspace 202, all green); live demo + raw artifacts in `docs/evidence/2026-10-24-rs012-inert-profiles/`; evidence record in [16 — Implementation evidence](docs/docs/16_IMPLEMENTATION_EVIDENCE.md). Release-binary rebuild deferred while the AT-06 soak holds `ramshaper-cli.exe`.

## 0.3.7 — 2026-10-24 (later)

- Recorded the AT-19 **automated portion** in [16 — Implementation evidence](docs/docs/16_IMPLEMENTATION_EVIDENCE.md) (evidence `at19-keyboard-states.ps1` → `at19-results.txt` + `at19-stale-state.png` in `docs/evidence/2026-10-24-rs010-slint-spike/`): Tab reaches the Quit button from the window and Enter activates it (graceful exit 0); with the agent killed, every stale/disconnected state is carried in UIA-exposed text (`agent: -`, `Data freshness: last updated 11s ago`, `Status: … not reachable … Retrying…`) alongside unit-bearing metric values — no color-only information. `accessible-live-region: polite` is declared in .slint but not machine-verifiable via the .NET Framework UIA wrapper (no `LiveSettingProperty`); deferred to the interactive Narrator pass. AT-19 remains open for the human pass (Narrator quality, 1366×768, 100/125/150/200 % DPI matrix).

## 0.3.6 — 2026-10-24 (later)

- Superseded the AT-19 "UIA zero children" blocker: the original probe's `TreeWalker` calls used non-existent .NET method names (`GetFirstChildElement`/`GetNextSiblingElement` instead of `GetFirstChild`/`GetNextSibling`), and `ErrorActionPreference='Continue'` routed the `MethodNotFound` errors away from the captured output — no child enumeration ever ran. The corrected re-probe (`docs/evidence/2026-10-24-rs010-slint-spike/uia-reprobe.ps1` → `uia-reprobe-result.txt`) shows a live Slint 1.18.1/AccessKit server-side UIA provider (`FrameworkId='Slint'`, `HasServerSideProvider=True`) exposing **42 annotated elements** — pressure/physical/commit groups with all accessible labels, the focusable Quit button, mode/freshness/instance labels — stable across background, foreground and raw-view passes with live values updating. The Slint accessibility bridge is functional in this build; the blocker is withdrawn and AT-19 remains open only for the interactive keyboard/Narrator/DPI pass. Doc 16 gained a correction section.

## 0.3.5 — 2026-10-24 (later)

- Added `ramshaper-cli low-memory-watch --stop-after-ms <MS>` (range 1000..=2592000000): graceful timed stop through the existing console-cancel event (`ConsoleCancel::trigger`), producing the same history summary, "handles released" line and exit code as Ctrl+C; rejected for `info`/`processes`. +1 unit test. Source-only for now — the release-binary rebuild is deferred while the concurrent AT-06 soak watchers hold `ramshaper-cli.exe` open (os error 5); the running soak continues on its pre-flag copy in `target/soak/`.
- Fixed `low-memory-watch --interval-ms` being silently ignored (dispatch passed `config.watch.interval_ms`, dropping `args.interval_ms`): `watch_interval()` now resolves flag > config > default, with a unit test for the precedence and a binary test for the stderr banner; verified live (`--interval-ms 60000` → "every 60000 ms"). Found during soak-harness work; defect record in [16 — Implementation evidence](docs/docs/16_IMPLEMENTATION_EVIDENCE.md).

## 0.3.4 — 2026-10-24 (later)

- Completed the RS-010 post-GO evidence addendum in [16 — Implementation evidence](docs/docs/16_IMPLEMENTATION_EVIDENCE.md): DPI probe (process is `PROCESS_PER_MONITOR_DPI_AWARE`), UIA control-view probe (window exposed with correct name but **zero child elements** — recorded as the concrete AT-19 blocker), and hidden-window footprint (no trim after 8 s minimized; updates continue, so visibility-aware pausing is required G2 work). Raw captures and scripts in `docs/evidence/2026-10-24-rs010-slint-spike/`.
- The generated Slint module is now wrapped in a scoped `#[allow(clippy::unwrap_used)]`, so workspace `clippy -D warnings` includes the ui crate (the ADR-024 exclusion note is superseded). Fresh gates: fmt clean, clippy clean, tests **175 passed / 0 failed / 16 suites**. ADR-025 added; BUILDING.md gained the UI-spike build/run section.

## 0.3.3 — 2026-10-24

- Implemented the RS-010 feasibility spike for G2 (R-02): new `ramshaper-ui` crate — a read-only Slint Overview window (slint 1.18.1, software renderer) rendering live system resident-memory counters and `pressure_v1` from the session agent over protocol v1, with subscribe-stream default transport and a `--poll-ms` one-shot `GetSnapshot` probe-poll mode; stderr state-transition log and visible serving `agent_instance_id`.
- Measured and recorded the R-02 feasibility evidence in [16 — Implementation evidence](docs/docs/16_IMPLEMENTATION_EVIDENCE.md) (raw captures in `docs/evidence/2026-10-24-rs010-slint-spike/`): binary 10.6 MiB unstripped; idle CPU 0.60% of one core probe-polling at 1 Hz (0.52% subscribed); 28.7 MiB working set / 11.2 MiB private; kill+restart of the agent detected within one poll interval and reconnected automatically to the new agent instance. **Decision: GO** on feasibility; Slint license selection and the AT-19 accessibility pass remain owner-gated before G2.
- 1 test added (workspace 174, all green); observer, agent and core untouched.

## 0.3.2 — 2026-10-24 (later)

- Implemented the AT-21 cohort classification (RS-025 classification half, observe-only): new pure `ramshaper-core::cohort` module (declared `4gib`/`8gib`/`16gib`/`none` cohorts, ±20% membership bands, `verify_declared` refusal of any declared cohort the machine does not classify into), a closed `[cohort]` config section, and `ramshaper-bench observer-overhead --config` stamping every report with `cohort { declared, measured, total_physical_bytes, status }`; mismatches exit 2 with no report.
- The 64 GiB validation machine is classified and measured **only as a non-cohort baseline**; live evidence records the baseline report, the refusal of a declared `8gib` on this machine, and an explicit list of what a 64 GiB machine can and cannot claim about cohort behavior. The declared-cohort matrix remains Not run; 19 tests added (workspace 173, all green).
- ADR-024 added; acceptance-matrix execution status, readiness checklist and [16 — Implementation evidence](docs/16_IMPLEMENTATION_EVIDENCE.md) updated.

## 0.3.1 — 2026-10-24 (later)

- Implemented the RS-030 session-agent/IPC slice: new `ramshaper-agent` crate (`serve`/`probe`), protocol v1 in core (`ramshaper-core::ipc`), windows modules for named pipes (explicit user-only DACL, peer session+SID validation, 1 MiB frame cap, bounded subscribers with gap markers), manual-reset events, SID/SDDL helpers and atomic config persistence; config gained an `[agent]` section.
- Extended [16 — Implementation evidence](docs/16_IMPLEMENTATION_EVIDENCE.md) with the RS-030 record: AT-17 frame-level portions Passed on a real pipe; cross-session/foreign-user peers Not run locally; four defects found and fixed during validation (double framing, uncancellable accept, lost final frames, bare pipe names). ADR-023 added.
- Workspace now 151 tests, all green; G1 open items (24-hour soak, cohorts) and G2 (Slint UI) unchanged.

## 0.3.0 — 2026-10-24

- Implemented the observation increment: process telemetry (`processes` command with identity-keyed two-scan rates), `pressure_v1` classifier in `info`/watch output, bounded in-session history with exit summaries, and versioned observe-only TOML configuration (RS-007–RS-009 + RS-030 config portion).
- Added `ramshaper-bench observer-overhead` (RS-024 Stage A) with a machine-local overhead baseline measured on the validation machine.
- Extended [16 — Implementation evidence](docs/16_IMPLEMENTATION_EVIDENCE.md) with the G1 section: AT-06 deterministic portion Passed (24-hour soak Not run), AT-07 Passed, AT-08 core/config portion Passed, AT-21 Not run (declared cohorts unavailable). G1 remains open.
- Documented implementation decisions in ADR-022 (Toolhelp32 enumeration, two-scan rates, churn identity-set comparison after a real-machine regression, TOML closed schema).

## 0.2.1 — 2026-10-23

- Implemented the first read-only CLI increment (RS-001–RS-006) at the workspace root: Cargo workspace (ramshaper-core, ramshaper-platform, ramshaper-windows, ramshaper-cli), pinned Rust 1.93.1 toolchain, committed lockfile, Windows CI, BUILDING.md.
- Added [16 — Implementation evidence](docs/16_IMPLEMENTATION_EVIDENCE.md): G0 cases AT-01–AT-05 executed and passed on Windows 11 build 26200.9711 (standard user) with recorded limitations; AT-06+ remain Not run.
- Documented implementation decisions in ADR-021 (windows-rs 0.62.2 pin, no-clap parsing, watch cadence grid, identity liveness check).
- Updated readiness checklist and acceptance-matrix execution status; refreshed workspace READMEs.

## 0.2 — 2026-10-22

- Separated the first read-only CLI increment from UI, policies and managed launches; defined evidence gates and dependency order.
- Added runtime contracts for typed telemetry, Windows process identity, clocks, bounded sampling, consent, recovery, IPC, configuration and data deletion.
- Added a requirements-to-ticket acceptance matrix and an implementation risk/decision register.
- Narrowed the first intervention to one manually approved temporary CPU-priority change; deferred recurring policies, memory priority and EcoQoS.
- Replaced ambiguous rollback promises with durable intent, compare-before-restore, crash reconciliation and explicit pending/conflict states.
- Required a creation-time Job Object launch spike to avoid making suspended-create/assign/resume the default design.
- Clarified benchmark sampling, missing values, guardrails and observer overhead; retained unmeasured status.
- Updated Windows source references, document authority, developer handoff and historical-document labeling.

This is a documentation revision only. No application code, build, OS mutation test or performance benchmark is included.

## 0.1 — 2026-10-21

- Initial research, PRD, architecture, API notes, UI concepts, roadmap and safety/benchmark proposals.
