# RAMShaper

RAMShaper is a Windows resource monitor and an optional, consent-gated
workload manager. Implemented and validated on Windows 11 build
26200.9711 as a standard user:

- **Read-only CLI** (RS-001–006, gate **G0 passed**): memory snapshot,
  low-memory watch, process telemetry with two-scan rates.
- **Observation increment** (RS-007–009 + RS-024 Stage A, G1 partially
  verified): `pressure_v1` classification, bounded in-session history;
  the 24-hour soak and declared 4/8/16 GiB cohorts remain open.
- **Session agent + IPC** (RS-030, protocol v1): a per-logon-session
  agent serving snapshots, process inventories and action records over a
  named pipe with an explicit user-only DACL.
- **Native UI** (RS-010–013 slices): a read-only Slint window — Overview,
  Applications, Profiles, Actions — with visibility-aware pausing and the
  automated accessibility portions recorded (the interactive
  Narrator/DPI pass remains open).
- **Policy stage, code-complete behind consent** (RS-015–019, RS-031,
  RS-016/017/018 engines, G3 glue): eligibility matrix with
  unknown-denies, deterministic dry-run, SQLite action journal, restore
  engine with bounded retries, guardrails, and the four policy IPC
  commands. **Nothing can write a process setting until the operator
  types the exact consent word (`ENABLE`) on the agent console** — the
  compatibility allowlist ships empty because no workload has passed
  Stage B testing yet.
- **Managed-launch spike** (RS-020): creation-time job attributes
  validated on this OS build; the G4 ticket track is separate.
- **Benchmark harness** (RS-024): observer-overhead baseline and the
  Stage B counterbalanced A/B workload runner feeding the AT-20 analysis.

Evidence and limitations for every claim are in the
[implementation evidence record](docs/docs/16_IMPLEMENTATION_EVIDENCE.md).
Nothing here creates physical RAM, guarantees performance gains, or
security-sandboxes arbitrary applications.

Start with the [documentation index](docs/README.md) and the
[implementation readiness review](docs/docs/00_IMPLEMENTATION_READINESS.md).
Build and run instructions are in [BUILDING.md](BUILDING.md); the
[privacy FAQ](FAQ_AND_PRIVACY.md) covers what the tool stores, sends
(nothing) and requires consent for; contribution expectations are in
[CONTRIBUTING.md](CONTRIBUTING.md).

```text
ramshaper-cli info                 # one system memory snapshot + pressure state (text)
ramshaper-cli info --json          # same, as one versioned JSON object
ramshaper-cli low-memory-watch     # sample memory + low-memory condition until Ctrl+C
ramshaper-cli processes            # per-process memory/CPU/IO with two-scan rates
ramshaper-cli config export|import # inert profile/exclusion config (validated, atomic)

ramshaper-agent serve              # per-session agent on \\.\pipe\... (read-only monitor)
ramshaper-agent serve --policy-journal <PATH>
                                   # + policy gate present but DISABLED; startup
                                   #   reconciliation runs; the operator may type
                                   #   ENABLE on that console to allow confirmed
                                   #   CPU-priority actions for THIS session only
ramshaper-agent probe              # one-shot pipe client for manual checks

ramshaper-ui --page overview|applications|profiles|actions
                                   # read-only Slint window over the agent's pipe

ramshaper-bench observer-overhead  # machine-local collector overhead baseline
ramshaper-bench workload-ab        # RS-024 Stage B counterbalanced CPU pilot
ramshaper-bench analyze-ab         # AT-20 A/B analysis (four honest labels)
```

The [runtime contracts](docs/docs/14_RUNTIME_CONTRACTS.md) define
telemetry, consent, journal, recovery and IPC behavior. The
[acceptance matrix](docs/docs/15_ACCEPTANCE_TEST_MATRIX.md) connects
requirements to tickets and observable tests. Architecture decisions are
recorded in [doc 13](docs/docs/13_ARCHITECTURAL_DECISIONS.md).

Documentation revision: **0.3.36, October 26, 2026** (policy stage,
consent path and Actions UI documented; RS-028). See the
[documentation changelog](docs/CHANGELOG.md).
