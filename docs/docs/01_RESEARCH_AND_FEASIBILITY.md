# RAMShaper — Research, Competitive Landscape, and Feasibility

**Document status:** Research baseline, not an implementation claim  
**Research as of:** 2026-10-21  
**Product:** RAMShaper — a lightweight, offline-first, Rust-based desktop resource-management utility

**Scope note (2026-10-22):** This remains the initial research baseline. Current delivery scope is in [02](02_PRODUCT_REQUIREMENTS.md) and [00](00_IMPLEMENTATION_READINESS.md): observation first, then one manually confirmed CPU-priority experiment. Memory-priority/EcoQoS policies and recurring automation discussed as possibilities below are deferred. API feasibility is not tested RAMShaper behavior.

## Executive conclusion

**GO, with a deliberately narrower MVP.** A Rust application can use public APIs to observe memory contention, inspect ordinary processes, run approved applications in Windows Job Objects, and apply limited, reversible scheduling/memory-priority policies where the OS permits. It cannot create physical RAM, guarantee better performance, transparently sandbox every third-party app, or make a paging-heavy workload fast on insufficient hardware. The innovation opportunity is reliable, explainable *policy selection*, not a secret low-level RAM-cleaning trick.

### Confidence matrix

| Claim | Status | Evidence or caveat |
|---|---|---|
| Native Windows RAM/commit metrics | Proven OS capability | GetPerformanceInfo provides pages for commit, physical memory, cache; multiply by page size [S01] |
| Windows low-memory event signal | Proven OS capability | CreateMemoryResourceNotification: low/high signals, not a universal latency metric [S02] |
| Per-process memory priority hint | Proven OS capability | SetProcessInformation(ProcessMemoryPriority), permissions apply [S03] |
| Grouping selected processes with Job Objects | Proven OS capability | CreateJobObject / AssignProcessToJobObject; association cannot normally be undone [S04] |
| Safe generic hard memory budgets for arbitrary desktop apps | Not proven; high risk | Job hard limits can fail commit requests and cause app malfunction [S05] |
| Linux group memory controls and stall measures | Proven OS capability | cgroups v2 memory.low/high/max; PSI memory/cpu/io stall signals [S06][S07] |
| macOS universal third-party process controls | Restricted / not committed | Apple exposes running-app and pressure signals, but permission/sandbox constraints require investigation [S08][S09] |
| Rust and lightweight native UI | Feasible | windows-rs, sysinfo, Slint; actual total footprint requires measurement [S10][S11][S12] |
| 20%+ user-speed gains on old PCs | Hypothesis only | Must be demonstrated across repeatable workload trials, not inferred from free-RAM increase |

## Definitions that matter

- **Physical RAM:** hardware. No software creates more of it.
- **Available physical memory:** Windows includes memory immediately reusable without disk write, including standby pages. A large cache is often healthy, not waste [S01].
- **Working set / resident set:** physical pages currently mapped into a process; shared pages can be counted in multiple processes. A lower number does not prove lower overall memory need [S13].
- **Commit / private bytes:** different metrics from working set; process/job committed-memory limits are not gentle target RAM budgets [S05].
- **Page fault:** can be soft or hard; not every page fault is a disk operation. Profile hard faults and I/O, not only total faults [S14].
- **Memory pressure:** evidence that memory scarcity is impairing workloads. On Linux PSI directly measures stall time; Windows requires several signals [S02][S07].
- **Container:** in RAMShaper documentation means **supervised resource group**, not VM, Docker container, or security sandbox. Consider user-facing wording “Managed App Groups.”

## Real opportunities and their limits

1. **Resource contention mitigation.** A focused editor competes with optional background sync. Safely deprioritizing eligible background work *may* improve app switching during contention, but can prolong the background task.
2. **Workload-aware rules.** Local, opt-in profiles can change behavior for a game, creative work, a browser, or low-RAM usage. Avoid assuming every idle-looking process is safe to suspend.
3. **Repeatable evidence.** Users can see measured before/after distributions, negative results, and an automatic rollback trail.
4. **Old hardware compatibility.** Lean core and native UI are helpful. However, paging to HDD, thermal throttling, slow CPUs, failing disks, outdated drivers, and inadequate RAM cannot be solved solely by memory management.
5. **Fleet use.** Shared school/lab PC administrators need a low-overhead, accountable policy engine. This is a *potential market*, not validated demand.

## Existing competition — no invented white space

| Product / feature | Publicly evidenced positioning | RAMShaper differentiation to test |
|---|---|---|
| Process Lasso (Bitsum) | ProBalance responsiveness under CPU load; persistent process priorities, affinities, memory priority and SmartTrim [S15][S16] | Transparent memory-pressure-first decisions, simple explainable managed groups and reproducible benchmarks |
| Microsoft PC Manager | Storage cleanup, health, startup management, memory/temporary-file boost, process tools [S17] | Avoid cleaner branding; center OS-level workload orchestration and opt-in rules |
| Native Windows Task Manager/Resource Monitor | Process and system visibility; OS itself handles memory automatically | “Why did this slowdown happen?” correlation and safe, reversible intervention |
| Linux cgroups/systemd/zram | Already ship controls and compression [S06][S18] | Accessible, understandable cross-platform policy UX, where permissions permit |

**Commercial lesson:** the process-management category is real and competitive. There is no basis to claim “nobody has built this.” Differentiation must be proven with usability, low overhead, and results.

## Platform viability

### Windows — MVP

Feasible read-only system metrics and selected process rules. Job Objects group a launched process tree; nested jobs and preexisting jobs complicate attaching to arbitrary apps. Jobs are *not* security isolation. System/protected processes are off-limits. Use low-memory signals alongside commit headroom and hard-fault/I/O observations. Avoid `EmptyWorkingSet` as a default: evicted pages may fault back in [S13][S14]. Windows 10 standard support ended 2025-10-14 for covered editions; if supported as legacy target, communicate security limitations and ESU paths [S19].

### Linux — follow-on

Use `/proc/pressure/{memory,cpu,io}`, PSI trigger events, and delegated cgroup v2 controls. `memory.high` throttles/reclaims, while `memory.max` can force OOM kills. Never replace host policy blindly. zram can reduce disk swap I/O by compressing pages in RAM but it consumes CPU/RAM and is already configured on many systems [S06][S07][S18].

### macOS — diagnostic-first

Use memory-pressure dispatch sources and NSWorkspace's documented running-app information where allowed [S08][S09]. Generic control over other vendors' programs depends on sandbox, entitlements, privilege and runtime permissions. Defer “containers” until feasibility tests establish a supported and safe mechanism.

## Failure cases to design for

- Lowering an editor's memory priority just before switching back to it can worsen perceived performance.
- A browser child process may exit/relaunch, requiring PID+creation-time identity checks.
- A game anti-cheat system may reject tampering with process properties; exclude it.
- “Suspend on idle” can break downloads, audio, networking, autosave, database writes or sync.
- Forced working-set trimming can increase hard faults and I/O.
- Hard job memory limits can cause allocation failures.
- Closing a job handle with the kill-on-close flag can terminate associated applications; never enable by default [S04].
- On Windows, assigning a running process to a job is not a reversible toggle; MVP launches selected apps managed from the start [S04].

## Decision

Proceed with **read-only profiler + safe optional background-priority policy + managed launcher**. Defer universal automatic RAM reclaim, blind suspension/restarts, kernel drivers, cross-process injection, and “AI optimization” until controlled tests demonstrate need and safety. See `02_PRODUCT_REQUIREMENTS.md`, `03_SYSTEM_ARCHITECTURE.md` and `08_BENCHMARKS_AND_VALIDATION.md`.

## Source IDs

All source IDs resolve to URLs in [`12_SOURCES.md`](12_SOURCES.md). References are official vendor/kernel documentation wherever available, plus the official competitor websites. Documentation must be revalidated before each OS/toolkit upgrade.
