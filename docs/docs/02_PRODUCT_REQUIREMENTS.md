# RAMShaper — Product Requirements

**Revision:** 0.2 • **Date:** 2026-10-22 • **Owner:** product/engineering responsibility  
**Status:** planning baseline; no implemented capabilities or measured gains

## 1. Problem and intended outcome

People using constrained PCs need to understand why their work feels slow and control optional background workloads without losing work. RAMShaper first observes memory-related signals and explains their limits. Later, it offers small, explicitly approved experiments and measures the outcome.

The primary outcome is p95 task responsiveness under a reproducible workload, with foreground completion, background throughput, stability and observer overhead measured alongside it. Free memory alone is not success.

| User | Job | Primary slice |
|---|---|---|
| 4–8 GB laptop owner | See whether low-memory signals coincide with slow work | 0.1 observation |
| Developer/creator | Test a temporary adjustment to an optional background workload | 0.2 policy alpha |
| Power user | Reproduce a result and understand/revert every change | 0.2–0.4 |
| Lab/refurbisher operator | Evaluate defined hardware cohorts before deployment | Evidence pilot; fleet controls deferred |

## 2. Release scope and acceptance

P0 identifiers describe the staged MVP, not features all required in 0.1. Detailed cases are in [15](15_ACCEPTANCE_TEST_MATRIX.md); behavior/defaults are in [14](14_RUNTIME_CONTRACTS.md).

### P0-01 — Honest read-only observation (first CLI, then 0.1)

**Story:** As a PC user, I want clearly defined resource readings so that I can investigate a slowdown without modifying applications.

First increment: physical total/available, system commit used/limit and low-memory notification state. 0.1 adds accessible process working set/private commit, CPU and process I/O, bounded history and qualitative pressure evidence. Hard faults, physical disk contention and task latency remain **Not measured** until dedicated instrumentation exists.

**Acceptance:** Given an ordinary user session, when collecting successful API samples, then values use the units/source definitions in [14](14_RUNTIME_CONTRACTS.md). Given missing, denied or stale data, then it is explicitly unavailable/stale and cannot silently become zero or policy evidence. PID reuse cannot carry history or action authority to another process. AT-01–AT-07.

### P0-02 — Mode, profiles and user control (0.1 settings; 0.2 actions)

**Story:** As a cautious user, I want to choose whether any action is allowed and see its exact scope.

Mode and profile are separate. Startup mode is Observe Only; Balanced is the default preset. Low-RAM, Focus, Gaming and Custom initially change presentation/preferences only. Imports and profile selection do not enable actions. Login startup is opt-in. Persist exclusions and preferences, not blanket permission to mutate future processes.

**Acceptance:** Given a restart, import or profile change, when settings apply, then no process setting is changed without a fresh scoped confirmation. Given an active action, when the user disables actions, then new applies stop immediately, restoration starts, and unresolved outcomes stay visible. AT-08, AT-11, AT-18.

### P0-03 — One temporary background CPU-priority experiment (0.2)

**Story:** As a user with an optional background workload, I want to try a bounded adjustment and restore it without guessing what changed.

The first action is a manually confirmed Normal → Below Normal CPU priority class change to one tested, eligible process. Default TTL 120 s; range 10–300 s. Fresh Elevated evidence and all eligibility checks in [14](14_RUNTIME_CONTRACTS.md) are prerequisites, not proof of benefit. No automatic repeat, descendants, name-based wildcard grants or setting bundles. Memory priority, EcoQoS and recurring policy automation are deferred.

**Acceptance:** Given valid scoped consent, identity, freshness and journal, when confirming, then original state is durably recorded before exactly one verified write. Given focus return, expiry, disable, safety loss, exit or restart, then restoration/reconciliation follows the defined state machine. An external change produces Conflict; failed restoration produces Needs attention. AT-09–AT-14, AT-17–AT-18.

### P0-04 — Managed launch beta (0.3)

**Story:** As a power user, I want to launch a compatible ordinary desktop executable in an observable resource group.

Only new, explicitly chosen launches qualify. Start with a local executable path and explicit arguments; shortcuts, scripts, shell associations, packaged-app brokers and arbitrary existing PIDs are out of the initial launcher scope. Show advisory metrics/thresholds, observed membership and compatibility limitations. Job association lasts for the process lifetime; removing monitoring does not detach it or create security isolation.

**Acceptance:** Given supported or incompatible launch conditions, when launch is attempted, then the outcome is explicit, no silent unmanaged retry or duplicate launch occurs, and no RAMShaper-created suspended orphan remains. UI/agent exit must not kill apps. No kill-on-close, hard quotas, time limits or active-process limits. AT-15–AT-16. A failed safety spike keeps this capability unavailable.

### P0-05 — Explanations and reproducible evidence (all slices; full wizard 0.4)

**Story:** As a user or evaluator, I want to understand the uncertainty and compare outcomes so that I can decide whether a change helps.

Every action preview states evidence, target, exact change, duration, expected downside, permission needs and recovery limitations. Observer-overhead instrumentation starts in 0.1; policy comparison starts before 0.2 expansion. The complete benchmark wizard follows in 0.4.

**Acceptance:** Given a benchmark session, when exported/analyzed, then it includes run and event counts, machine/workload metadata, raw measurements, missing statuses, uncertainty and adverse outcomes. Neutral/negative results remain visible; only the preregistered evidence rule can label improvement. AT-20–AT-22.

## 3. Non-functional requirements

| ID | Requirement / planning target | Verification |
|---|---|---|
| NFR-01 | Hidden non-elevated agent: steady-state working set <25 MiB, average normalized CPU <1% on declared idle test; UI measured separately | Observer protocol in [08](08_BENCHMARKS_AND_VALIDATION.md), AT-21; targets unmeasured |
| NFR-02 | Offline operation, no automatic update check/upload, local minimal data and explicit redacted exports | AT-18, AT-23 |
| NFR-03 | Bounded queues/history, cancellable waits, explicit staleness, no catch-up polling burst | Runtime caps; AT-05–AT-07 |
| NFR-04 | Keyboard, Narrator, visible focus, non-color status, reduced motion and supported DPI layout | AT-19; toolkit feasibility gates UI release |
| NFR-05 | Durable recovery intent, identity-safe writes and truthful pending/conflict states | AT-03, AT-10–AT-18 |
| NFR-06 | Typed failures and capability discovery; no UAC for ordinary observation or MVP agent | AT-01, AT-04, AT-17 |

These are engineering goals and required behavior, not achieved measurements. Document misses instead of silently weakening targets.

## 4. Exclusions

No automatic kills, restarts, suspension, blanket working-set/cache trimming, hard memory limits, foreground High/Realtime boost, kernel drivers, code injection, private APIs, memory-content inspection, elevated helper, cloud telemetry, fleet server or AI inference in the MVP. No security-container or universal performance claim. Windows is responsible for scheduling and virtual memory.

Future work: startup auditing, leak-suspicion trends, recurring approved rules, memory-priority/EcoQoS experiments, Linux PSI/delegated cgroups, macOS diagnostics and fleet capabilities. Each requires a separate scoped contract and evidence.

## 5. Compatibility and release

Windows 11 x64 is the intended first platform. Record exact tested edition/build/toolchain before a release. Windows 10 is a separate legacy evaluation, not implied compatibility; Linux, macOS and ARM64 are deferred. The first delivery is a portable diagnostic CLI; installer, signing, login behavior and uninstall recovery arrive with their gates.

A release passes the applicable gate in [15](15_ACCEPTANCE_TEST_MATRIX.md), publishes actual compatibility/overhead evidence, and makes claims no broader than measured results. Three independently rerunnable workload classes are required before broad performance claims. Policy safety tests must pass even when benefit is neutral.

## 6. Scenarios and limits

A student sees low-memory signals while switching apps: 0.1 shows timing and metric definitions, with no inferred culprit. In 0.2 a compatible optional workload can receive one confirmed experiment. If the target becomes foreground, the agent restores it.

A gaming profile observes by default; game and anti-cheat workloads are excluded. If safety classification is uncertain, no action is offered.

A healthy device receives no optimization claim. A reported problem or measured benchmark regression disables the implicated policy pending review; the app does not pretend to automatically diagnose all third-party crashes.

Open decisions and their gates live in [00](00_IMPLEMENTATION_READINESS.md). License, toolkit feasibility, safe-target coverage and actual benefit remain unresolved.