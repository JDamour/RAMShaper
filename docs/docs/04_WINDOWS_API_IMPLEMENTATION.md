# RAMShaper — Windows API Plan

**Revision:** 0.2 • **Reviewed:** 2026-10-22 • **Status:** documented API feasibility; integration behavior untested.
Behavioral requirements are in [14](14_RUNTIME_CONTRACTS.md); API access does not itself authorize an intervention.

## A. Capability map

| Capability | Documented API / source | Product boundary |
|---|---|---|
| Physical, commit, cache counters | `GetPerformanceInfo`, `PERFORMANCE_INFORMATION` [S01] | Pages converted to bytes; cache overlaps available classifications |
| Low-memory condition | `CreateMemoryResourceNotification`, `QueryMemoryResourceNotification` [S02][S29] | Resource condition, not application latency or numeric pressure |
| Native identity and CPU counters | `GetProcessTimes` [S23] | Exact creation FILETIME; kernel/user deltas separate from identity |
| Working set / private commit | `GetProcessMemoryInfo`, `PROCESS_MEMORY_COUNTERS_EX` [S27] | `WorkingSetSize` and `PrivateUsage` have different meanings |
| Process I/O | `GetProcessIoCounters` [S28] | Process I/O accounting; not physical-disk/paging instrumentation |
| CPU-priority class | `GetPriorityClass`, `SetPriorityClass` [S24] | First policy: Normal → Below Normal only |
| Memory-priority / EcoQoS | `SetProcessInformation` [S03] | Research-only; deferred beyond first policy alpha |
| New managed launch | `CreateProcessW`, `STARTUPINFOEXW`, `PROC_THREAD_ATTRIBUTE_JOB_LIST` [S25] | Preferred spike; no suspended-create fallback in current design |
| Job accounting | `QueryInformationJobObject` [S04][S21] | Membership/accounting with compatibility limits |
| Job notifications | Notification limits/completion port [S04][S21] | Advisory only; reconcile membership rather than trusting every event |
| Local IPC | Windows named pipe with explicit security descriptor [S26] | Session-scoped and non-elevated; no default ACL |

## B. First CLI collector

Use a minimal standard-user handle for each query; do not enable debugging privilege or require administrator. Initialize API structure sizes correctly, check results and capture the immediate Win32 error. Wrap all acquired handles in RAII ownership with cancellable waits.

For global counters, convert using checked u64 arithmetic and reject invalid page size/totals. Compare exact conversions against fixtures first. For real Windows comparison, record both raw API sample and displayed bytes at the same sample timestamp. Task Manager is an approximate cross-check with different sampling/labels, not an exact oracle.

A memory-resource condition can stay signaled. An unconditional wait loop on an already-signaled object can spin; use a cancellable, rate-limited state-query/watch design and test persistent signaling. Collect at the intervals in [14](14_RUNTIME_CONTRACTS.md); no catch-up bursts after sleep or a blocked sample.

Process rate calculation needs two successful samples of the same identity. Reset on topology/counter changes or invalid intervals. Never substitute `PageFaultCount` for hard faults, process I/O for physical disk traffic, or CPU-priority class for memory consumption.

The first six tickets expose only `info`, `low-memory-watch`, `--version` and `--help`. Process enumeration is subsequent work.

## C. Identity and priority actuator

`GetProcessTimes` supplies creation FILETIME, not a monotonic start clock [S23]. Pair it with PID, then validate owner/session and eligibility. Use the same validated process handle through reading, writing and verification. Target exit, identity-query failure and access denial are expected outcomes.

Read priority with query rights and change it with `PROCESS_SET_INFORMATION` [S24]. Do not request all-access handles. `PROCESS_MODE_BACKGROUND_BEGIN/END` applies to the calling process; it is not the mechanism for lowering an arbitrary other process's priority. Use only the planned class change after consent.

```text
revalidate target, fresh evidence, exclusions and scoped consent
read original Normal priority
commit Prepared record durably
recheck cancellation/eligibility immediately before write
set Below Normal through validated handle
read back -> Applied, Failed or Uncertain
restore according to recorded-state reconciliation
```

No read-check-write sequence is atomic against other priority managers. Preserve external conflicts, read back restoration, and disclose this limitation. Changed scheduling can affect background completion and potentially descendants; begin only with tested leaf workloads. Never enroll children automatically. A known incompatible priority manager or unknown safety classification makes the target ineligible.

Foreground detection and protected/audio/security classification need a documented, tested signal matrix under R-03. Do not replace that gate with an executable-name denylist or claim universal detection.

## D. Managed launch spike (later, RS-020)

The earlier suspended-create/assign/resume suggestion is superseded for the planned launcher: a launcher crash between steps can strand a suspended child. Evaluate the documented creation-time job list attribute instead [S25]. This is a design choice to test, not a claim of a universal crash-atomic guarantee.

1. Accept a selected local ordinary desktop executable and explicit argv. Show resolved path, arguments and working directory only in the local consent preview.
2. Reject scripts, shell aliases, shortcuts and brokered/packaged launchers in the initial beta. Set explicit `lpApplicationName`; use reviewed Windows argument encoding and test quotes/backslashes/Unicode. No shell expansion.
3. Create a fresh job, with **no** kill-on-close, hard memory/CPU limits, execution-time limits, active-process limits, UI restrictions or forced breakaway flags.
4. Prepare `STARTUPINFOEXW` and `PROC_THREAD_ATTRIBUTE_JOB_LIST`, honoring API attribute-buffer lifetimes. Launch with extended startup information; do not request CREATE_SUSPENDED.
5. Check launch result and membership via returned handles. Observe actual children; do not promise containment of broker-created or escaped workloads.
6. On failure, explain it. No automatic unmanaged fallback or repeat launch. If a crash/lost result makes launch outcome uncertain, reconcile known records and require user review before another launch; do not claim exactly-once launch across process death.
7. Interrupt the launcher before/during/after creation, test nested jobs and confirm that closing UI/agent/job handles never unexpectedly kills user apps. If any case fails the product contract, keep the feature disabled.

Job association cannot be detached from a running process [S04]. Removing monitoring does not undo association. First beta may lose group monitoring after an agent crash; report that limitation rather than reattaching unrelated processes. Resuming group observation after restart requires separately tested handle/discovery logic.

A job may inherit constraints from the launcher environment. Detect incompatible preexisting jobs; do not escape them. Post-create assignment APIs have rights and limits that can produce destructive failure in some configurations [S20]; a fresh advisory job must not acquire those limits. Query the effective environment in tests.

## E. Verification scope

G0 tests actual collection, handles and failures. G3 adds consent, protected/unknown targets, same-handle identity, disk-full/journal crashes, external conflicts, focus/session triggers and child inheritance. G4 adds launch interruption, nested jobs, escaped children and app survival. Use [15](15_ACCEPTANCE_TEST_MATRIX.md) for exact cases.

All Windows builds, privileges, API results and unsupported cases must be recorded. No API documentation check substitutes for these tests. Detailed ETW hard-fault/disk instrumentation, elevated helpers, memory-priority writes and EcoQoS remain deferred.