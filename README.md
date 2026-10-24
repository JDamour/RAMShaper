# RAMShaper

RAMShaper is a Windows resource monitor and optional workload manager. Three increments are implemented and validated on Windows 11 build 26200.9711 as a standard user: the read-only CLI (RS-001 through RS-006, gate G0 passed), the observation increment (RS-007 through RS-009 plus the Stage A measurement harness; G1 partially verified — the 24-hour soak and declared hardware cohorts remain open), and the RS-030 session-agent/IPC slice (protocol v1 over a session-scoped named pipe with explicit user-only DACL). Evidence and limitations are in the [implementation evidence record](docs/docs/16_IMPLEMENTATION_EVIDENCE.md). Later features (native UI, policies, managed launch) have separate evidence gates and are not implemented.

Start with the [documentation index](docs/README.md) and [implementation readiness review](docs/docs/00_IMPLEMENTATION_READINESS.md). To build and run the CLI, see [BUILDING.md](BUILDING.md).

```text
ramshaper-cli info                 # one system memory snapshot + pressure state (text)
ramshaper-cli info --json          # same, as one versioned JSON object
ramshaper-cli low-memory-watch     # sample memory + low-memory condition until Ctrl+C
ramshaper-cli processes            # per-process memory/CPU/IO with two-scan rates
ramshaper-bench observer-overhead  # machine-local collector overhead baseline
```

The [runtime contracts](docs/docs/14_RUNTIME_CONTRACTS.md) define telemetry, consent, recovery and IPC behavior. The [acceptance matrix](docs/docs/15_ACCEPTANCE_TEST_MATRIX.md) connects requirements to tickets and observable tests.

Documentation revision: **0.3.0, October 24, 2026** (observation increment added). See the [documentation changelog](docs/CHANGELOG.md).
