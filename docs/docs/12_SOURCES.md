# RAMShaper — Research Sources and Verification Register

**Initial register:** 2026-10-21. **Targeted documentation review:** 2026-10-22; only the pages listed in the verification table below were checked in this revision. Other entries remain the prior research baseline, not newly verified. External pages may change; verify applicable APIs/toolkits before implementation milestones. API documentation is not RAMShaper integration-test evidence. Design defaults, caps and performance targets are project decisions, not vendor recommendations.

## Official APIs and OS internals

- **[S01] Microsoft — PERFORMANCE_INFORMATION / GetPerformanceInfo:** https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-performance_information  and https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-getperformanceinfo  
  Supports physical/commit/cache pages and page-size conversions; `PhysicalAvailable` includes standby/free/zero lists.
- **[S02] Microsoft — CreateMemoryResourceNotification:** https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-creatememoryresourcenotification  
  Low/high available-memory event signal; not an absolute system-speed measurement.
- **[S03] Microsoft — SetProcessInformation:** https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation  
  `ProcessMemoryPriority`, `ProcessPowerThrottling`, access rights and EcoQoS behavior.
- **[S04] Microsoft — Job Objects overview:** https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects  
  Group management, association, limits, notifications, inheritance and lifetime.
- **[S05] Microsoft — Job Object committed-memory limits:** https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_limit_information  and https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_extended_limit_information  
  Hard commit limits can cause allocation failure; kill-on-close behavior documented.
- **[S06] Linux Kernel — Control Group v2:** https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html  
  `memory.low`, `memory.high`, `memory.max` semantics and OOM behavior.
- **[S07] Linux Kernel — Pressure Stall Information (PSI):** https://docs.kernel.org/accounting/psi.html  
  `/proc/pressure/{cpu,memory,io}`, stall semantics, thresholds and trigger events.
- **[S08] Apple — Dispatch memory pressure:** https://developer.apple.com/documentation/dispatch/dispatch_source_type_memorypressure  
  macOS system pressure events.
- **[S09] Apple — NSWorkspace:** https://developer.apple.com/documentation/appkit/nsworkspace/runningapplications  and https://developer.apple.com/documentation/appkit/nsworkspace  
  Running/frontmost application observation; does not by itself authorize arbitrary memory-control claims.
- **[S13] Microsoft — Process working set:** https://learn.microsoft.com/en-us/windows/win32/procthread/process-working-set  
  Working-set resident/shared/private semantics; page-in cost and caution with changes.
- **[S14] Microsoft — Working Set and page faults:** https://learn.microsoft.com/windows/win32/memory/working-set  and https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-setprocessworkingsetsize  
  Soft and hard fault differences, eviction and trimming; no implied benefit from making numbers smaller.
- **[S18] Linux Kernel — zram:** https://docs.kernel.org/admin-guide/blockdev/zram.html  
  Compressed RAM block devices; does not create physical RAM.
- **[S19] Microsoft — Windows 10 end of support:** https://learn.microsoft.com/en-us/lifecycle/announcements/windows-10-end-of-support  
  Covered Windows 10 editions end normal support 2025-10-14, with ESU or edition-specific differences.
- **[S20] Microsoft — Nested Jobs and assignment:** https://learn.microsoft.com/en-us/windows/win32/procthread/nested-jobs  and https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-assignprocesstojobobject  
  Nested job hierarchy and association constraints.
- **[S21] Microsoft — QueryInformationJobObject:** https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-queryinformationjobobject  
  Job notification information classes; version-specific support.
- **[S22] Microsoft — Job termination/lifetime flags:** https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_limit_information  
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` must not be enabled by default.

## Rust / UI dependencies

- **[S10] Microsoft — windows-rs Rust project:** https://github.com/microsoft/windows-rs  and https://docs.rs/windows/latest/windows/  
  Windows API Rust bindings; pin compatible crate version in build.
- **[S11] sysinfo crate:** https://docs.rs/sysinfo/latest/sysinfo/  and https://docs.rs/sysinfo/latest/sysinfo/struct.Process.html  
  Cross-platform info with platform differences; `memory()` resident set != private bytes.
- **[S12] Slint official docs/license:** https://slint.dev/ , https://slint.dev/get-started , https://slint.dev/community , https://docs.slint.dev/latest/docs/slint/  
  Native Rust UI and multiple licensing paths; attribution/copyright review required for chosen distribution mode.

## Competitors / context

- **[S15] Bitsum — Process Lasso docs:** https://bitsum.com/apps/process-lasso/docs/ and https://bitsum.com/apps/process-lasso/docs/algorithms/probalance/  
  Established automated process priority and high-CPU responsiveness management.
- **[S16] Bitsum — memory/I/O priorities and rules:** https://bitsum.com/apps/process-lasso/docs/rules/priorities/ and https://bitsum.com/automation/  
  Process Lasso already competes in this functionality.
- **[S17] Microsoft PC Manager:** https://pcmanager.microsoft.com/en-us and https://pcmanager.microsoft.com/en-us/termsofservice  
  Cleanup, startup, health and memory boost positioning.

## Sources added for revision 0.2

- **[S23] Microsoft — GetProcessTimes:** [API reference](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes). Creation FILETIME and process CPU times; creation identity is distinct from a duration clock.
- **[S24] Microsoft — process priority class:** [GetPriorityClass](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getpriorityclass) and [SetPriorityClass](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setpriorityclass). Query/set rights and class behavior; background-mode flags are restricted to the calling process. The one-action/TTL/restore design is RAMShaper policy, not a Windows guarantee.
- **[S25] Microsoft — extended process creation:** [UpdateProcThreadAttribute](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute) and [STARTUPINFOEX](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-startupinfoexa). Job-list creation attribute and attribute storage lifetime. Use the Unicode W variant in implementation. Neither citation proves arbitrary interrupted-call outcomes or exactly-once launch.
- **[S26] Microsoft — named pipe security:** [Named Pipe Security and Access Rights](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights). Explicit descriptors, default access and logon-session restriction. Does not establish an isolation boundary against same-user malware.
- **[S27] Microsoft — process memory counters:** [PROCESS_MEMORY_COUNTERS_EX](https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex). WorkingSetSize versus PrivateUsage; page faults are not identified as hard faults by this structure.
- **[S28] Microsoft — process I/O accounting:** [GetProcessIoCounters](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getprocessiocounters) and [IO_COUNTERS](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-io_counters). Process read/write/other operation and byte accounting; do not substitute this for a dedicated physical-disk instrument.
- **[S29] Microsoft — low-memory condition query:** [QueryMemoryResourceNotification](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-querymemoryresourcenotification). Nonblocking condition query and explicit success/failure; sampling/hysteresis remains product policy.

## Targeted verification record — 2026-10-22

| Source | Checked page / finding | Remaining verification |
|---|---|---|
| S01 | PERFORMANCE_INFORMATION: page units, physical available, cache overlap and changing commit limit | Real sample/conversion/error tests |
| S04 | Job Objects: persistent association, child/lifetime and notification limitations | App survival, child escape and restart observation |
| S20 | AssignProcessToJobObject: rights, nested jobs and dangerous limit failure cases | Compatibility fixtures; no assumption all jobs are safe |
| S23–S24 | Creation FILETIME, CPU timing and priority API restrictions | Identity reuse, external writer and restore races |
| S25 | Job-list entry verified in official API search content; initial rendered page omitted that row. STARTUPINFOEX layout also checked | Binding/SDK support, Unicode launch, interrupted creation; no runtime test |
| S26 | Default pipe security is insufficient for the intended session boundary | Explicit DACL/peer/session tests |
| S27–S29 | Memory-counter semantics, process I/O accounting and condition query | Sampling, missing-data and cancellation tests |

Slint versions/licenses, current Windows lifecycle coverage, competitors and Linux/macOS APIs were not revalidated in this targeted review. Resolve the applicable release decision before relying on them. Historical `[web:...]` markers in the original concept are not source-register IDs.

## Research gaps and warnings (current)

- No valid measured RAMShaper benchmark result exists at the time of these docs.
- No source establishes universal compatibility with all Windows/macOS/Linux applications.
- No credible source supports a claim that Rust alone increases available physical RAM.
- No source validates business pricing, market size, domain availability or trademark ownership.
- Vendor pages can change; re-check current stable Rust/Slint/sysinfo versions at implementation time.
- The generated PNG images have fictional utilization numbers and illustrative performance statistics, not measured data.
