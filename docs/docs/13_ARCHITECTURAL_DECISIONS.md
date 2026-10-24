# RAMShaper — Architecture Decision Records (ADR)

**Revision:** 0.2 • **Updated:** 2026-10-22. Required constraints apply immediately to the plan. Accepted-for-planning decisions are selected designs, not proof of a working implementation. Proposed/Open items require evidence at their named gate. Status vocabulary and open risks are defined in [00_IMPLEMENTATION_READINESS.md](00_IMPLEMENTATION_READINESS.md).

## ADR-001: Rust-native core — Accepted for planning

**Context:** persistent low-resource monitoring needs predictable performance and OS API control.  
**Decision:** Rust for core/user agent and platform adapters.  
**Consequences:** FFI `unsafe` must be isolated and audited; ergonomics vs safety tradeoffs acceptable. Rust does not itself optimize other programs' RAM.

## ADR-002: Native Slint UI — Proposed; G2 feasibility/license gate

**Context:** target old devices.  
**Decision:** test Slint before webview/React-style shell.  
**Consequences:** smaller potential runtime, but must benchmark actual rendering overhead, licensing, accessibility and high-DPI behavior. Do not assume marketed toolkit footprint equals whole app footprint.  
**Update 2026-10-24:** the feasibility half is GO (doc 16, RS-010 spike: 10.6 MiB binary, 0.60 % of one core idle, 28.7 MiB working set, automatic agent-restart reconnect; UIA accessibility tree verified present). The license half is prepared for the owner decision in [design/2026-10-24-slint-license-memo.md](../design/2026-10-24-slint-license-memo.md) — GPL-3.0-only vs Royalty-free 2.0 vs commercial, with the attribution obligation and the "open-source claim" trade-off spelled out. The interactive AT-19 pass remains open.

## ADR-003: Windows first — Accepted for planning

**Context:** OS controls vary and parity would delay shipping.  
**Decision:** first MVP on Windows x64, Linux subsequent, macOS diagnostic-first.  
**Consequences:** capability-gated platform adapters; no parity claims.

## ADR-004: Observe-only by default — Required

**Context:** inappropriate priority/memory mutations can worsen latency and crash apps.  
**Decision:** no changes before informed opt-in; pressure evidence and rollback required.  
**Consequences:** initial release may show neutral results; trust takes precedence over aggressive marketing.

## ADR-005: “App Container” terminology — Required rename in UI

**Context:** Job Objects manage process groups but do not security-sandbox files/network.  
**Decision:** internal module `managed_groups`; UI name **Managed App Groups**, with explanatory tooltip.  
**Consequences:** less confusion with Docker/VM containers.

## ADR-006: No blanket memory-trimming — Required

**Context:** emptying working sets can lower displayed numbers but increase faults and disk I/O.  
**Decision:** forbid in the MVP; any later experimental option needs a separate safety contract and benchmark.  
**Consequences:** honest use of OS virtual-memory behavior.

## ADR-007: No hard memory limits on ordinary apps in MVP — Required

**Context:** Job limits affect committed memory and can fail allocations [S05].  
**Decision:** advisory thresholds + monitoring only; hard limits in future explicit advanced mode after extensive testing.

## ADR-008: Process identity — Required

**Context:** OS PIDs recycle.  
**Decision:** pair PID with stable process creation marker and validate before every action.  
**Consequences:** extra API code/collection required.

## ADR-009: Offline-first, user-local telemetry — Required

**Context:** a performance monitor can reveal private app activity.  
**Decision:** local processing; no cloud, no hidden analytics, minimize logs.  
**Consequences:** fleet features need separate consent architecture.

## ADR-010: Safety outranks matching concept art — Required

**Context:** demo images include “restart offenders,” “cache trim every 5min,” hard budgets and fictional boost badges.  
**Decision:** visual style may inspire implementation, but features and claims must match PRD and benchmarks.

## ADR-011: No bundled Windows driver — Required for MVP

**Context:** elevated complexity, security risk, maintenance and signing obligations.  
**Decision:** use published Win32 APIs; no kernel module, scheduler patch or memory compression replacement.

## ADR-012: License is unresolved — Open

**Context:** potential open core + commercial editions.  
**Options:** Apache-2.0/MIT, GPLv3, dual licensing; Slint has separate license conditions.  
**Decision:** obtain legal review and publish SPDX license before external contributions and distribution.

## ADR-013: What success looks like — Required

**Decision:** benchmark p95 responsiveness under controlled workload alongside safety/overhead; free-memory amount alone cannot establish success. Negative results are publishable.

## ADR-014: Read-only CLI before desktop services — Accepted for planning

**Context:** phase-one text previously mixed diagnostics and managed launch. **Decision:** RS-001–RS-006 expose read-only CLI only. Later agent is current-user/session, not a system service. **Consequence:** no IPC, journal or Slint dependency is necessary for the first increment; later boundaries have their own gates.

## ADR-015: Exact identity and missing-data contracts — Required

**Decision:** Windows identity uses PID plus exact creation FILETIME [S23]; durations use a separate clock. Revalidate and act through the same handle. Metrics carry explicit status, age, source and units; no invented numeric confidence or substituting zero. **Consequence:** IPC/export preserves identifier precision and rate warmup/gaps.

## ADR-016: First policy is one CPU-priority experiment — Accepted for planning

**Decision:** Normal → Below Normal, one process, manual scoped confirmation, TTL and cooldown. Recurring permissions, memory-priority writes and EcoQoS are deferred. **Alternative:** implementing multiple policy kinds together would multiply recovery/benchmark variables. **Consequence:** smaller eligibility allowlist and no promise of broad benefit; see [14](14_RUNTIME_CONTRACTS.md).

## ADR-017: Durable intent and conflict-aware restoration — Required

**Decision:** SQLite journal is required before policy writes, with Prepared committed first; startup reconciles without reapplying. Preserve external changes and unresolved records. **Consequence:** disk/schema failure blocks new actions; deletion/uninstall can leave a disclosed minimal recovery journal. Read-check-write races and crash-time persistence remain explicit limits.

## ADR-018: Creation-time job attributes — Proposed; G4 spike gate

**Context:** suspended-create/assign/resume can strand a child on launcher crash. **Decision:** evaluate STARTUPINFOEXW job-list attributes [S25], no suspended-create fallback in the current design. **Consequence:** compatibility/interrupted-call behavior must be tested; no crash-atomic or exactly-once launch guarantee is asserted. Feature remains unavailable if the gate fails.

## ADR-019: Canonical split docs and evidence gates — Accepted for planning

**Decision:** numbered docs are authoritative; master reference is a linked guide; original research is historical. Requirement/ticket/case mapping lives in [15](15_ACCEPTANCE_TEST_MATRIX.md). **Consequence:** update scope, contract, case and ADR together; every unexecuted case stays Not run.

## ADR-020: Memory-only history by default — Accepted for planning

**Decision:** transient history, optional bounded aggregate persistence, separate pinned recovery data, local redacted exports. **Consequence:** lower default disk activity and exposure; restarting loses transient graphs. Caps and failure behavior are defined in [14](14_RUNTIME_CONTRACTS.md), not left to implementation guesswork.

## ADR-021: First-increment implementation choices — Accepted (implemented 2026-10-23; evidence in [16](16_IMPLEMENTATION_EVIDENCE.md))

**Context:** RS-001–RS-006 needed concrete pins and small auditable designs.

**Decisions:**
- `windows` crate pinned to exactly 0.62.2 with ten narrow features. `Win32_Security` is included solely because `CreateEventExW`'s binding signature references `SECURITY_ATTRIBUTES`; no security API is called directly.
- No argument-parser dependency: a unit-tested hand-rolled parser covers the four-command surface and the exit-2 usage contract.
- No datetime dependency: RFC 3339 UTC formatting is a validated pure function (known-epoch tests) in `ramshaper-core`.
- Watch loop waits only on the cancellation event, never on the low-memory object; ticks follow a fixed monotonic grid advanced by whole intervals, so late wakeups cause skips, never shortened intervals (a real 16 ms tick cluster found during validation was fixed and regression-tested).
- Identity verification checks liveness (`GetExitCodeProcess` ≠ `STILL_ACTIVE`) through the same handle as the creation-FILETIME comparison, because terminated processes remain queryable with their original marker until the OS retires the object.
- OS metadata comes from `RtlGetVersion` + registry (UBR, DisplayVersion, ProductName verbatim) + `GetNativeSystemInfo`; a stale registry product name is reported as-is rather than rewritten.
- `deny(unsafe_code)` in core/platform/cli; all unsafe FFI confined to seven audited modules in `ramshaper-windows`; `clippy::unwrap_used` denied workspace-wide.

**Consequences:** small dependency surface, deterministic core tests, and honest partial states; future slices must extend rather than bypass these contracts.

## ADR-022: Observation increment implementation choices — Accepted (implemented 2026-10-24; evidence in [16](16_IMPLEMENTATION_EVIDENCE.md))

**Context:** RS-007–RS-009 plus the config-schema portion of RS-030 (AT-08 core/config) and RS-024 Stage A.

**Decisions:**
- Process enumeration via Toolhelp32 snapshot (`Win32_System_Diagnostics_ToolHelp` feature); per-process queries through one `PROCESS_QUERY_LIMITED_INFORMATION` handle per process per scan; inventory capped at 4,096 identities with an explicit truncation flag (contract 14 §3).
- The `processes` CLI command takes **two scans** separated by a configurable window (default 500 ms) and computes identity-keyed rates over the exact monotonic gap; one-shot inventories report `warming_up` rates rather than inventing deltas.
- Churn compares the identity *sets* of consecutive scans (a first implementation compared successful baselines and mis-reported protected processes as churn on every scan; found on the real machine and regression-tested).
- `pressure_v1` is a pure state machine in core (retains previous valid level while pending; missing observation → Unknown; no Critical; byte-identical replays). A one-shot `info` therefore honestly reports `unknown`.
- Process CPU is normalized to percent of total machine capacity and clamped to [0, 100]; I/O rates are process accounting, explicitly not physical-disk/paging throughput.
- Configuration is TOML (`toml =1.1.8`): closed schema (`deny_unknown_fields`), integer `schema_version`, only `observe_only`; field-level errors; parse-then-validate so a bad file never partially applies. The CLI treats an invalid `--config` file as exit 2 (contract 14 §7/§8). Atomic persistence is deferred to the slice that writes configs.
- Bounded history (15 min / 8 MiB / capacity, oldest-first eviction, gap and rejection counters) lives in core and is wired into watch exit summaries.
- `ramshaper-bench observer-overhead` (new crate) reports machine-local collector latency percentiles and its own memory with an explicit not-a-cohort-claim note.

**Consequences:** observation capabilities are testable deterministically; per-process denial is expected output, not command failure; the G1 gate stays open pending the 24-hour soak and declared-cohort measurements.

## ADR-023: Session agent IPC v1 — Accepted (implemented 2026-10-24; evidence in [16](16_IMPLEMENTATION_EVIDENCE.md))

**Context:** RS-030 agent/IPC slice, prerequisite for the G2 UI (RS-010 depends on RS-030).

**Decisions:**
- Protocol v1 in `ramshaper-core::ipc` as a pure codec (4-byte LE length + JSON, 1 MiB cap enforced before allocation); the transport (windows pipes) owns framing so double-framing bugs like the one found in validation cannot recur silently.
- Pipe `\\.\pipe\ramshaper-agent-s<session>-v1`, explicit SDDL DACL granting only the serving user; per-connection re-validation of session + user SID; same-user malware explicitly outside the guarantee (R-07).
- Synchronous-mode instances with blocking `ConnectNamedPipe`; shutdown uses `CancelSynchronousIo` on the accept thread's duplicated handle (overlapped events are ignored on synchronous handles). Final frames are flushed (`FlushFileBuffers`) before `DisconnectNamedPipe`, which otherwise discards unread buffers.
- Subscribers hold at most two pending frames with explicit `gap_dropped` markers; connections capped at 8 with `Busy` frames; `GetSnapshot` attaches `age_ms` and returns `StaleSnapshot` beyond 2× cadence — never silently old data.
- `serve`/`probe` are separate binaries so the UI (and tests) can be developed against the protocol without the agent's process model.

**Consequences:** read-only commands are fully exercisable and adversarially tested at the frame level; the policy commands, idempotency ledger and journal integration of RS-030 remain G3 work.

## ADR-024: AT-21 cohort classification — Accepted (implemented 2026-10-24; evidence in [16](16_IMPLEMENTATION_EVIDENCE.md))

**Context:** RS-025 / NFR-01 declare 4/8/16 GiB measurement cohorts, but the
only validation machine available is a 64 GiB laptop. AT-21 forbids
"substituting a better machine"; observations therefore need an explicit,
auditable cohort classification before any overhead number is published.

**Decisions:**
- Cohorts are **declared, never inferred for publication**: the closed TOML
  config schema gains a `[cohort]` section with one field,
  `declared = "4gib" | "8gib" | "16gib" | "none"` (default `none` = baseline).
  Declaring is a measurement label only; it grants no authority and changes
  no collection behavior (the schema stays observe-only).
- A machine **belongs** to a cohort when its measured total physical memory
  (`GetPerformanceInfo`, the same source as `physical_total_bytes`) falls in
  `[4/5, 6/5) × nominal` bytes — ±20% with exact rational bounds. The band
  absorbs hardware reservations (a "64 GiB" machine reports 63.75 GiB) while
  adjacent cohorts (2× apart) can never overlap, and in-between sizes
  (6, 12, 32 GiB, …) classify as non-cohort.
- `verify_declared` refuses any declared cohort the machine does not
  classify into, in both directions (a 64 GiB machine cannot claim `8gib`;
  an 8 GiB machine cannot claim `16gib`). `none` always verifies: a baseline
  claims nothing, so nothing can clash — including a machine whose reading
  is unavailable. The bench tool maps a refusal to exit 2 with the reason on
  stderr and emits no report.
- Every report carries `cohort { declared, measured, total_physical_bytes,
  status }` so a reviewer can audit the classification without trusting the
  label; a cohort claim requires an explicit declaration (a missing config
  is always a baseline, never a cohort).
- The validation machine's recorded measurement (68,448,034,816 bytes) is a
  regression fixture: it must classify as non-cohort forever.

**Consequences:** AT-21's cohort matrix still requires real 4/8/16 GiB
machines (missing cohorts stay unverified), but no future measurement can be
accidentally labeled as cohort data, and the classification logic is fully
deterministic and tested. (The note that the ui spike's Slint-generated code
tripped the workspace `unwrap_used` deny is superseded — see ADR-025: the
generated module is now wrapped in a scoped allow and workspace clippy
includes the ui crate.)

## ADR-025: Slint UI spike choices (RS-010 feasibility half) — Accepted (implemented 2026-10-24; evidence in [16](16_IMPLEMENTATION_EVIDENCE.md))

**Context:** R-02 requires toolkit feasibility evidence — license,
overhead, robustness — before G2 UI work proceeds.

**Decisions:**

- Slint pinned exactly `=1.18.1` with a minimal, default-features-off set
  (`std`, `backend-default`, `renderer-software`, `accessibility`,
  `compat-1-2`). The UI is a separate `ramshaper-ui` crate that talks to
  the agent exclusively through protocol v1; the UI process contains no
  observer code.
- Subscription (`SubscribeSnapshots`) is the default transport;
  `--poll-ms` switches to one-shot `GetSnapshot` probes on the same
  connection for the polling-cost comparison. Both measured equal at idle
  (0.52 % vs 0.60 % of one core): the window is the cost, not the IPC mode,
  so either transport is viable for G2.
- Disconnect handling is automatic and visible: state transitions log to
  stderr, and the serving `agent_instance_id` is displayed, so an agent
  restart is observable; reconnect retries every 1.5 s and binds to the new
  instance without user action.
- Slint-generated code is wrapped in a scoped `#[allow(clippy::unwrap_used)]`
  module so the workspace deny keeps applying to hand-written code
  (supersedes the exclusion recorded in ADR-024).
- Spike findings are recorded, not fixed: the UIA control-view tree exposes
  the window but **zero children** (the AT-19 blocker); minimized windows
  keep updating at 1 Hz (visibility-aware pausing is G2 work); the process
  is `PROCESS_PER_MONITOR_DPI_AWARE` (V1/V2 not distinguished by the probe).

**Consequences:** R-02 feasibility is **GO** for G2. The Slint license
selection (ADR-002/ADR-012) and the AT-19 accessibility pass remain
owner-gated, and the UIA child-exposure gap must be resolved before AT-19
can pass.

## ADR-026: `GetProcesses` over IPC and the Applications page — Accepted (implemented 2026-10-24; evidence in [16](16_IMPLEMENTATION_EVIDENCE.md))

**Context:** RS-011 needs the UI's application table fed by the agent, but
NFR-01 bounds the idle agent's CPU (<1 % normalized) and the IPC frame cap
is 1 MiB.

**Decisions:**
- `GetProcesses` is **additive to protocol v1**; agents without it answer
  `Unsupported`, and the UI renders that as an explained empty table —
  never as "no processes".
- Inventories are collected **on demand, one fresh scan per request** —
  never cached on the snapshot collector cadence (a ~17-21 ms scan at 1 Hz
  would alone spend ~2 % of a core and break the NFR-01 idle budget).
- Rate baselines **survive across requests and connections** in shared
  agent state: the first request reports honest `warming_up` rates, later
  requests compute identity-keyed rates over the exact gap since the
  previous scan. A scan-level failure is a retryable `OsError`.
- Responses serve the heaviest processes first and **cap rows at 1,024
  with an explicit `row_cap_note`**, keeping the worst case inside the
  1 MiB frame cap (the 4,096-identity collector bound would not fit).
- The UI polls **only while the Applications page is visible** (2 s
  cadence, one fetch in flight), so a closed or Overview-facing window
  costs the agent nothing. Sorting is client-side; per-cell unavailable
  notes come from statuses, never zeros; every row exposes one
  whole-sentence accessible label for AT-19.

**Consequences:** the Applications page adds zero idle agent cost, degrades
honestly (disconnect notes, unsupported agents, capped rows), and leaves
eligibility explanations to RS-015 (G3). The page is observation-only.
