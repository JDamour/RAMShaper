# RAMShaper — Linux and macOS Implementation Roadmap

**Scope clarification (2026-10-22):** All entries are planned capabilities, not implemented support. Linux/macOS are deferred; Windows features arrive in separate release slices specified in [00_IMPLEMENTATION_READINESS.md](00_IMPLEMENTATION_READINESS.md). Do not create placeholder platform crates in the first CLI increment.

## Linux target: native cgroup/PSI integration

### Supported capabilities

- Read `/proc/pressure/memory`, `/proc/pressure/cpu`, `/proc/pressure/io`; `some` measures some tasks blocked, `full` captures complete stalls where defined. Use `avg10`, `avg60`, `avg300` and total, with proper semantics [S07].
- cgroup v2: observe `memory.current`, `memory.events`, `memory.stat` and per-group PSI; use `memory.low` protection and `memory.high` soft throttling only with correct delegation/permissions [S06].
- `memory.max` enforces a hard limit and can cause OOM kills — advanced explicit mode only. No silent OOM controls [S06].
- zram is compressed RAM-backed block storage and can be useful for swap under some conditions; do **not** advertise it as free physical RAM, and do not overwrite existing host zram settings [S18].

### Safe implementation steps

1. Probe kernel and cgroup v2 availability; collect OS/distribution/init system details.
2. Start with read-only PSI/cgroup monitoring and benchmark dashboard.
3. Discover delegated user slices/cgroups; prefer systemd/user delegation when available instead of writing arbitrary global cgroup paths.
4. Launch a test background workload under a RAMShaper-owned delegated cgroup.
5. Implement alert thresholds, actions log, and cancellation. Distinguish `memory.high` throttling from `memory.max` termination risk.
6. Introduce one optional, reversible pressure-aware intervention with privilege boundaries, then profile overhead and failure behavior.
7. Test several distros/kernel versions; do not assume distributions configure systemd/zram identically.

### Linux validation targets

- PSI event correctly tracks controlled memory pressure.
- Non-root install supports observation; unsupported delegated management disables with explanation.
- Existing zram/swap state unchanged unless user explicitly configures a standalone future feature.
- cgroup removal/restart cannot strand managed apps, corrupt service tree or trigger unplanned OOM kills.
- Negative and neutral benchmark results preserved honestly.

## macOS target: diagnostics first

### What is grounded

- `DISPATCH_SOURCE_TYPE_MEMORYPRESSURE` exposes OS memory-pressure events [S08].
- `NSWorkspace.runningApplications` enumerates running applications, and `frontmostApplication` identifies the app currently active at the UI level [S09].
- The exact ability to change another process's memory behavior depends on OS policy and permissions; universal third-party “containers” are **not committed**.

### Proposed implementation progression

1. Native read-only app/session diagnostics using documented system and AppKit APIs.
2. Carefully tested process metrics with appropriate permission handling; never promise complete visibility of all processes.
3. User-controlled *RAMShaper-launched* app workflows only if public APIs and macOS security architecture support them safely.
4. Distribute as a signed/notarized native app with a clearly documented permissions model if platform feasibility succeeds.

### Avoid

- Disabling SIP, bypassing sandbox/entitlements, private kernel interfaces, injecting into arbitrary apps, deceptive accessibility permissions, or marketing parity with Windows.

## Cross-platform parity guide

| Feature | Windows v0 | Linux future | macOS future |
|---|---|---|---|
| Memory dashboard | Yes | Yes | Yes, within access |
| Pressure signal | Windows event + derived evidence | PSI native stall percentages | Memory-pressure dispatch events |
| Ordinary process list | Yes, permissions-dependent | Yes, permissions-dependent | App/process visibility constrained |
| Temporary priority policy | Selected Win32 processes | Selected delegated groups/processes | Discovery required |
| Managed app groups | Job Objects for new launches | Delegated cgroups for launched workloads | Not committed |
| Hard RAM caps | Explicit advanced only, post-MVP | Explicit advanced only, OOM risk | Not committed |
| Protected OS modifications | Excluded | Excluded | Excluded |

The brand is cross-platform, but **feature parity is not a product promise**. Each platform must declare capabilities at runtime via a `Capabilities` struct rather than rendering nonfunctional controls.

## References

See `12_SOURCES.md` S06–S09, S18.
