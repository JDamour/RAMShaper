# RAMShaper — Product & Technical Research Document

> **Historical concept, retained for context (2026-10-21).** This document contains superseded feature proposals and opaque legacy citation markers such as `[web:31]`; those markers are not usable evidence references. Do not implement from it. Use the [current documentation index](README.md), [requirements](docs/02_PRODUCT_REQUIREMENTS.md), [runtime contracts](docs/14_RUNTIME_CONTRACTS.md) and [source register](docs/12_SOURCES.md). In particular, automatic policies, generic app containers and broad compatibility/benefit claims below are not approved current behavior.

**Tagline:** Less memory pressure. More responsiveness. Longer PC life.  
**Proposed product:** An open-source, OS-level memory and application resource manager built primarily in Rust.  
**Initial target platform:** Windows 10/11, followed by Linux.  
**Author concept:** RAMShaper Adaptive App Containers and Workload-Aware Resource Governance.

---

## 1. Executive Summary

RAMShaper is not a traditional “RAM cleaner.” It is a lightweight, local-first resource-governance tool that helps older or low-memory computers remain responsive when memory pressure rises.

Instead of claiming to create more RAM or blindly freeing memory, RAMShaper observes real system pressure, identifies eligible background workloads that are contributing to contention, and applies reversible, user-approved policies. It can lower the priority of noncritical background work, supervise selected applications through managed resource groups, and show the user whether an intervention actually improved responsiveness.

The product is technically feasible with Rust because modern operating systems expose supported mechanisms for process monitoring, memory-priority adjustment, process grouping, and resource limits. On Windows, RAMShaper can use process memory priority, Job Objects, performance counters, and memory-pressure notifications. On Linux, it can use Pressure Stall Information (PSI), cgroups v2, and systemd integration.

The central product promise should be:

> **RAMShaper helps the application you are using stay responsive by reducing avoidable interference from background work — without killing applications or pretending to add RAM.**

---

## 2. Problem Statement

### 2.1 The real-world problem

Many users with 4 GB or 8 GB RAM laptops experience:

- Slow application switching.
- Delayed typing or mouse response.
- Browser tabs reloading.
- Development tools freezing during builds or indexing.
- Games stuttering when background applications run.
- System-wide slowdowns when memory usage becomes high.
- Long startup times caused by unnecessary background applications.

These problems are especially common on older laptops, refurbished PCs, school computers, and office machines that must run modern browsers, communication apps, office software, and development tools simultaneously.

### 2.2 Why “free RAM” is the wrong framing

A common misconception is that low free RAM always causes poor performance. In modern operating systems, unused RAM is often intentionally used for caching. Windows, for example, keeps recently used file pages in a standby list so they can be reused quickly. Removing data from RAM simply to display a larger “free RAM” number can actually harm performance. [web:31]

The real issue is **memory pressure**: when active applications need more physical memory than is comfortably available, the operating system must remove pages from RAM and later reload them from disk. This creates page faults and disk activity, which can make the system feel slow or unresponsive. [web:32][web:34]

### 2.3 Technical cause: memory contention and paging

A process’s **working set** is the portion of its virtual memory currently resident in physical RAM. When RAM is scarce, the OS may remove pages from a process’s working set. If that process later needs those pages, it experiences a page fault and the data must be brought back from disk. [web:31][web:39]

When this happens repeatedly across many processes, the system can enter a state where it spends more time moving pages between RAM and disk than doing useful work. This is commonly called **thrashing**. It produces the familiar symptoms of a frozen or extremely slow computer. [web:32][web:44]

### 2.4 Why existing solutions are insufficient

| Existing approach | Limitation |
|---|---|
| RAM cleaners | Often focus on showing “free RAM,” may clear useful cached data, and do not address which workloads are causing contention. |
| Task Manager | Shows resource usage but provides limited, manual, per-process control and no adaptive policy engine. |
| Process priority tools | Can help, but often require technical knowledge and may not connect priority changes to actual memory-pressure events. |
| General optimizers | Frequently make broad claims, install background services, or apply risky system tweaks. |
| Upgrading hardware | Often the most effective solution, but not always affordable or possible for schools, offices, and users with older devices. |

Process Lasso, for example, already provides process automation, persistent priorities, CPU affinity controls, and responsiveness-oriented algorithms. Its ProBalance approach lowers the priority of selected background processes during high load rather than raising the priority of foreground work. [web:35][web:36][web:41]

RAMShaper should therefore not compete by simply adding another RAM graph or “Optimize” button. Its differentiation must be a **workload-aware, transparent, measurable, and conservative** resource manager.

---

## 3. Proposed Solution

### 3.1 Core idea

RAMShaper is a local Rust-based service and desktop application that:

1. Monitors real memory pressure and system responsiveness.
2. Identifies applications and background workloads contributing to pressure.
3. Allows users to classify applications as **Focused**, **Normal**, **Background**, or **Protected**.
4. Applies reversible priority and resource policies when pressure rises.
5. Launches selected applications inside supervised resource groups.
6. Measures whether interventions improved foreground responsiveness.
7. Does nothing when the system is healthy.

### 3.2 Product principle

> **Intervene only when there is measurable pressure, use supported OS mechanisms, make changes reversible, and prove the result.**

RAMShaper does not replace the operating system memory manager. It cooperates with it.

---

## 4. Improved Product Idea: Adaptive App Containers

The most distinctive improvement over a basic memory monitor is the **Adaptive App Container**.

Instead of merely inspecting running processes, RAMShaper can launch selected applications inside managed resource groups. Each group receives a policy describing how it should behave under memory pressure.

### Example policies

| Policy | Behavior |
|---|---|
| **Focused** | The active application receives preference; RAMShaper avoids interfering with it. |
| **Normal** | Default behavior; monitored but not aggressively managed. |
| **Background** | Eligible for reduced memory priority, lower CPU priority, delayed startup, or throttled background activity during pressure. |
| **Protected** | Never automatically modified; includes system services, security software, drivers, and user-designated critical apps. |
| **Budgeted** | Optional advanced mode with a memory or CPU ceiling; used with clear warnings because strict limits can cause application failures. |

### Practical example

A user opens VS Code on a 4 GB laptop while Chrome, OneDrive, a messaging app, and an update service are running.

RAMShaper detects rising memory pressure. It identifies Chrome extensions, sync clients, and update helpers as eligible background workloads. It temporarily lowers their memory priority and optionally reduces CPU priority. VS Code remains in the Focused category.

When pressure falls, RAMShaper restores the original priorities.

The user sees:

- What was slowing the system.
- What RAMShaper changed.
- Whether app-switching or input responsiveness improved.
- How to undo the change.

---

## 5. Core Features

### 5.1 Smart RAM Governor

Detects actual memory pressure using available memory, commit charge, paging activity, hard page faults, and OS memory-pressure signals. When pressure becomes sustained, RAMShaper selectively reduces the priority of eligible background workloads.

This is technically grounded: Windows provides `SetProcessInformation` with `ProcessMemoryPriority`, allowing an application to assign a lower memory priority to a process or thread. Microsoft describes this as appropriate for background operations whose data is not expected to be reused soon. Lower-priority pages are trimmed before higher-priority pages under memory pressure. [web:17][web:18]

### 5.2 Adaptive App Containers

Users can launch applications through RAMShaper with a selected policy. On Windows, Job Objects can group related processes and apply supported resource controls. This enables supervised launching without turning RAMShaper into a full virtual-machine or container system.

### 5.3 Focus Boost

A one-click mode that protects the current foreground workload and temporarily deprioritizes approved background work. This should be the flagship user-facing feature.

### 5.4 Memory Leak Watchdog

Detects suspicious memory growth patterns, such as continuous working-set or commit growth over time. It should notify the user and suggest safe actions, such as restarting the application, rather than automatically killing it.

### 5.5 Smart Startup

Identifies startup applications that contribute to early memory pressure. Users can disable, delay, or classify them. This feature is valuable because many low-RAM machines become overloaded before the user even opens their main application.

### 5.6 Performance Profiles

Profiles make the product usable for non-technical users:

- **Low-RAM Laptop:** Conservative, high-sensitivity pressure detection.
- **Developer Workstation:** Protects editors, compilers, containers, and databases while deprioritizing nonessential background tools.
- **Gaming:** Prioritizes the game and reduces interference from launchers, updaters, browsers, and sync clients.
- **Multitasking:** Balances several active applications.
- **Battery Saver:** Combines memory policy with lower background activity.

### 5.7 Proof-of-Boost Dashboard

The dashboard should not merely show “RAM freed.” It should show whether the user’s experience improved:

- Foreground app-switch delay.
- Input stalls or responsiveness events.
- Hard page faults per second.
- Disk read/write activity caused by paging.
- Memory-pressure duration.
- Background process impact.
- Before/after comparison for a managed session.

This is essential for credibility.

---

## 6. What RAMShaper Should Not Do

To avoid becoming another unreliable optimizer, RAMShaper should not:

- Claim to create RAM.
- Automatically terminate applications by default.
- Force strict memory caps on ordinary applications.
- Repeatedly empty process working sets.
- Disable system services without explicit user consent.
- Modify undocumented Windows internals.
- Install a kernel driver in the first version.
- Upload process activity or personal data to the cloud.
- Promise that it will double PC speed.

Working-set manipulation and hard allocation limits can degrade performance or cause applications to fail. The safer approach is reversible priority management, user-approved policies, and clear warnings for advanced controls. [web:17][web:18]

---

## 7. Technical Feasibility

### 7.1 Why Rust is suitable

Rust is appropriate for RAMShaper because the tool itself must be lightweight, reliable, and efficient. A memory-management utility that consumes hundreds of megabytes would undermine its own purpose.

Rust provides:

- Low runtime overhead.
- Strong memory-safety guarantees.
- Excellent systems-programming capabilities.
- Native Windows API access through crates such as `windows` and `windows-sys`.
- Strong concurrency support through Tokio.
- Suitable libraries for GUI development, such as Slint.
- Straightforward compilation into a small native executable.

### 7.2 Windows implementation

Windows should be the first platform.

| Capability | Windows mechanism |
|---|---|
| Process discovery | Win32 Toolhelp / process enumeration APIs |
| Memory metrics | Performance counters, `GetProcessMemoryInfo`, system memory information |
| Memory priority | `SetProcessInformation` with `ProcessMemoryPriority` [web:17][web:18] |
| Process grouping | Job Objects |
| CPU priority | Process and thread priority classes |
| Memory-pressure signals | Windows memory-notification APIs |
| Startup management | Registry startup entries, Startup folder, Task Scheduler |
| Background service | Windows Service |
| UI | Slint or Tauri, depending on footprint and development preference |

Windows maintains standby pages with priorities from 0 to 7. When free memory is insufficient, the memory manager can repurpose lower-priority standby pages first. This makes memory priority a meaningful, supported control point for RAMShaper. [web:31]

### 7.3 Linux implementation

Linux is a strong second platform because it exposes excellent resource-control primitives.

| Capability | Linux mechanism |
|---|---|
| Memory-pressure detection | Pressure Stall Information (PSI) [web:16][web:21] |
| Resource groups | cgroups v2 |
| Memory limits | `memory.max`, `memory.high` |
| Process grouping | systemd slices/services or direct cgroup management |
| Swap behavior | `memory.swap.max`, zram guidance |
| Service integration | systemd |
| Desktop integration | PolicyKit, D-Bus, desktop entries |

PSI measures the time tasks are delayed because of contention for CPU, memory, or I/O. It provides both system-wide and per-cgroup pressure metrics, making it a better signal than raw RAM usage alone. [web:16][web:19][web:21]

### 7.4 macOS implementation

macOS should be treated as a diagnostics-first platform.

RAMShaper can provide:

- Memory-pressure monitoring.
- Process resource visibility.
- User-guided recommendations.
- Startup-item review where permitted.

It should not promise the same level of control over third-party applications as Windows or Linux, because macOS imposes stricter restrictions on manipulating other processes.

---

## 8. Recommended System Architecture

```text
+------------------------------------------------------+
|                    RAMShaper UI                       |
|   Dashboard | Focus Boost | App Policies | Reports    |
+---------------------------+--------------------------+
                            |
                            v
+------------------------------------------------------+
|                 RAMShaper Rust Engine                 |
|                                                       |
|  +----------------+  +---------------------------+   |
|  | Memory Pressure|  | App Resource Supervisor   |   |
|  | Monitor        |  |                           |   |
|  +----------------+  +---------------------------+   |
|                                                       |
|  +----------------+  +---------------------------+   |
|  | Policy Decision|  | Performance Analyzer      |   |
|  | Engine         |  |                           |   |
|  +----------------+  +---------------------------+   |
|                                                       |
|  +----------------+  +---------------------------+   |
|  | Safety Layer   |  | Local Settings / History  |   |
|  +----------------+  +---------------------------+   |
+---------------------------+--------------------------+
                            |
        +-------------------+-------------------+
        v                                       v
+------------------+                   +------------------+
| Windows APIs     |                   | Linux OS APIs    |
| Memory priority  |                   | PSI              |
| Job Objects      |                   | cgroups v2       |
| Perf counters    |                   | systemd          |
+------------------+                   +------------------+
```

### 8.1 Memory Pressure Monitor

Collects:

- Total, available, and committed memory.
- Per-process working set and private working set.
- Commit size.
- Hard and soft page faults.
- Disk activity associated with paging.
- CPU usage.
- Foreground process identity.
- Memory-pressure notifications.
- PSI metrics on Linux.

### 8.2 App Resource Supervisor

Maintains application groups and applies policies:

- Focused.
- Normal.
- Background.
- Protected.
- Budgeted.

It should support both newly launched applications and, where permissions allow, already-running user applications.

### 8.3 Policy Decision Engine

The engine should use conservative rules:

```text
IF memory pressure is low:
    DO NOTHING

IF memory pressure is moderate:
    NOTIFY USER
    SUGGEST OPTIONAL BACKGROUND REDUCTION

IF memory pressure is high AND user enabled Focus Boost:
    LOWER PRIORITY OF APPROVED BACKGROUND APPS
    PRESERVE PROTECTED APPS
    RECORD BASELINE AND RESULT

IF pressure remains high:
    ESCALATE ONLY WITH USER APPROVAL
```

### 8.4 Safety Layer

The safety layer is critical.

Protected categories should include:

- Windows system processes.
- Security software.
- Display, audio, input, and storage drivers.
- Active downloads or updates explicitly requested by the user.
- Applications the user marks as protected.
- Foreground applications.
- Processes requiring elevated privileges that RAMShaper cannot safely manage.

Every automatic action should be:

- Reversible.
- Logged.
- Time-limited.
- Visible in the UI.
- Disabled by default for high-risk actions.

---

## 9. Minimum Viable Product

### MVP goal

Prove that RAMShaper can identify memory pressure and safely improve foreground responsiveness without harming stability.

### MVP scope: Windows

Build a small Rust tray application and optional background service with three features:

1. **Memory Pressure Monitor**
   - Show RAM, commit, paging, and pressure state.
   - Identify top memory consumers.
   - Show whether pressure is low, moderate, or high.

2. **Focus Boost**
   - Let the user select a foreground app to protect.
   - Let the user select eligible background apps.
   - Apply reversible memory-priority reduction when pressure rises.
   - Restore priorities when pressure falls.

3. **Startup Manager**
   - Show startup applications.
   - Let users disable or delay nonessential entries.
   - Explain the estimated impact in plain language.

### Explicitly excluded from MVP

- Automatic process termination.
- Hard RAM limits.
- RAM compression hacks.
- Working-set emptying.
- Cloud accounts.
- Machine-learning claims.
- Complex container runtime features.

---

## 10. Development Roadmap

### Phase 1 — Observe

**Duration:** 2–4 weeks  
**Goal:** Build trust through accurate monitoring.

Deliverables:

- Rust service and tray UI.
- Process and memory metrics.
- Memory-pressure classification.
- Local event history.
- “What is using memory?” report.
- No automatic system changes.

### Phase 2 — Control

**Duration:** 4–8 weeks  
**Goal:** Deliver the first useful optimization.

Deliverables:

- App classification.
- Focus Boost.
- Reversible memory-priority policies.
- Protected-process rules.
- Manual override and audit log.
- Basic before/after responsiveness indicators.

### Phase 3 — Supervised App Containers

**Duration:** 8–12 weeks  
**Goal:** Introduce advanced managed launching.

Deliverables:

- Launch apps through RAMShaper.
- Apply app profiles.
- Use Windows Job Objects for grouping.
- Optional soft limits and warnings.
- Advanced strict limits behind explicit consent.

### Phase 4 — Measure and Adapt

**Duration:** Ongoing  
**Goal:** Prove value with reproducible measurements.

Deliverables:

- Benchmark suite.
- Session reports.
- Policy-effectiveness scoring.
- Automatic disabling of harmful policies.
- Local-only telemetry and user-controlled diagnostics export.

### Phase 5 — Linux Expansion

**Duration:** After Windows MVP  
**Goal:** Use Linux PSI and cgroups v2 for stronger technical capabilities.

Deliverables:

- PSI-based pressure detection.
- cgroup-based app groups.
- systemd integration.
- Linux packages.
- Optional zram recommendations.

---

## 11. Benchmarking Strategy

RAMShaper must prove its value with repeatable tests.

### Test devices

Test across:

- 4 GB RAM laptop.
- 8 GB RAM laptop.
- 16 GB RAM desktop.
- HDD-based system, if available.
- SSD-based system.
- Windows 10 and Windows 11.
- Linux distributions with systemd and cgroups v2.

### Workloads

Use repeatable scenarios:

1. Browser with many tabs plus code editor.
2. Code editor, compiler, browser, and communication apps.
3. Game launcher, game, browser, and background sync.
4. Office multitasking: browser, document editor, PDF viewer, video call.
5. Startup-load test after reboot.
6. Artificial memory-pressure test using controlled demo workloads.

### Metrics

Measure:

- Application-switch latency.
- Time from launch to usable state.
- Foreground input stalls.
- Hard page faults per second.
- Paging disk throughput.
- Memory-pressure duration.
- CPU contention.
- Battery impact.
- RAMShaper’s own memory and CPU usage.

### Engineering target

The RAMShaper core service should aim for a very small footprint, for example under 25 MB RAM while idle, subject to real profiling. The UI may use more memory only while open.

### Honest claims

Do not advertise “2x faster PC” unless controlled testing proves it for a specific workload. A more defensible claim is:

> “RAMShaper reduced app-switch stalls by X% during memory-pressure tests on a 4 GB Windows laptop.”

---

## 12. Competitive Landscape

| Product / approach | Strength | RAMShaper opportunity |
|---|---|---|
| Windows Task Manager | Built-in, familiar, reliable | Adds adaptive policy, profiles, and measurable outcomes |
| RAM cleaners | Simple perceived value | Avoids harmful “free RAM” behavior; focuses on pressure and responsiveness |
| Process Lasso | Strong process automation, priorities, CPU affinity, and responsiveness algorithms | Adds memory-pressure awareness, user-friendly profiles, supervised app containers, and proof-of-boost reporting. [web:35][web:36][web:41] |
| Microsoft PC Manager | Trusted Microsoft branding and basic cleanup | Focuses on workload-aware resource governance rather than generic cleanup |
| Hardware upgrade | Most reliable performance improvement | Provides a lower-cost software option for usable older hardware |

### Differentiation statement

> RAMShaper is not a RAM cleaner. It is a workload-aware resource governor that identifies memory contention, temporarily reduces interference from approved background work, and shows whether the user’s active applications became more responsive.

---

## 13. Open-Source and Business Model

### 13.1 Open-source core

The core should be open source to build trust:

- Transparent policies.
- No hidden telemetry.
- Community auditing.
- Contributions from power users.
- Credibility against “snake oil” optimizer software.

### 13.2 Product tiers

| Offering | Audience | Scope |
|---|---|---|
| **RAMShaper Core** | Individual users | Free, open-source monitoring, Focus Boost, manual policies, startup manager |
| **RAMShaper Pro** | Power users and professionals | Profiles, automation rules, advanced diagnostics, supervised launching, detailed reports |
| **RAMShaper Fleet** | Schools, offices, labs, refurbishers | Central policy management, fleet reporting, deployment tools, support |
| **OEM / Refurbisher Edition** | PC refurbishers and manufacturers | Preconfigured profiles, branding options, bulk licensing |

### 13.3 Why Fleet is promising

Schools, computer labs, small offices, and refurbishers often manage large numbers of aging computers. For them, the value is not “more free RAM.” It is:

- Fewer support tickets.
- Longer usable hardware life.
- More predictable classroom or office performance.
- Lower replacement cost.
- Central visibility into problem devices.

---

## 14. Risks and Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Users expect miraculous performance | Poor reviews and mistrust | Clear onboarding, honest claims, benchmark evidence |
| Wrong process deprioritized | App slowdown or instability | Protected-process list, allowlists, reversible actions, audit log |
| Security software blocks changes | Feature failure | Graceful fallback, clear permission messaging, no forced bypass |
| Hard limits crash apps | User frustration | Make strict limits opt-in and clearly warn users |
| RAMShaper consumes too many resources | Product contradicts its purpose | Rust, lightweight UI, profiling budgets, minimal background polling |
| Measurement is unreliable | Loss of credibility | Use repeatable benchmarks and conservative reporting |
| OS API restrictions | Limited functionality | Design per-platform capability layers and degrade gracefully |
| Existing competitors | Hard differentiation | Focus on memory-pressure-aware, measurable, user-friendly governance |

---

## 15. Recommended Branding and Messaging

### Product name

**RAMShaper**

### Primary tagline

**Less memory pressure. More responsiveness. Longer PC life.**

### Alternative taglines

- **Keep the app you’re using responsive.**
- **A smarter way to use the RAM you already have.**
- **Reduce memory-pressure stalls without killing your apps.**
- **Built for computers that need every megabyte.**

### Homepage headline

> Your PC is not out of RAM. It is out of patience.  
> RAMShaper helps background work wait its turn.

### Honest positioning

RAMShaper does not add RAM. It helps your computer use the RAM it already has more intelligently by reducing avoidable interference from background workloads.

---

## 16. Final Recommendation

Build **RAMShaper for Windows first** as a lightweight Rust utility.

Start with observation and safe Focus Boost, not aggressive optimization. Use supported Windows mechanisms such as process memory priority and Job Objects. Add Linux support later using PSI and cgroups v2.

The product should be positioned as:

> **An open-source, workload-aware resource manager that helps low-memory computers stay responsive under pressure.**

This direction is technically credible, commercially differentiated, and much safer than another RAM cleaner. Its success will depend on three things:

1. Accurate pressure detection.
2. Conservative and reversible interventions.
3. Honest, measurable proof that the user’s active work becomes more responsive.

---

## 17. References

- Microsoft, `SetProcessInformation` function: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation [web:17]
- Microsoft, `MEMORY_PRIORITY_INFORMATION` structure: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/ns-processthreadsapi-memory_priority_information [web:18]
- Linux Kernel, Pressure Stall Information: https://www.kernel.org/doc/html/latest/accounting/psi.html [web:16]
- Linux Kernel, Control Group v2: https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html [web:19]
- Process Lasso official site: https://bitsum.com/ [web:35]
- Process Lasso overview and ProBalance description: https://en.wikipedia.org/wiki/Process_Lasso [web:36]
- Windows working-set and standby-memory explanation: https://learn.microsoft.com/en-us/answers/questions/2696389/physical-and-virtual-memory-in-windows-10 [web:31]
- Page faults and low-memory performance: https://www.lenovo.com/us/en/glossary/page-fault/ [web:32]
