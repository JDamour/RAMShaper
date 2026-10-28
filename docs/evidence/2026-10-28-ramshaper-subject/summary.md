# ramshaper-subject — Stage B trial subject binary, 2026-10-28

Design-memo follow-up ticket 2 (docs/design/2026-10-28-stage-b-trial-design.md),
built **ahead of ADR-045 as inert capability**: a workload-runner binary the
future trial orchestrator spawns per run. The ADR gates whether the agent may
act on it (allowlist/classification) — not whether it exists; nothing in the
shipped surface references it yet.

## What was built

New crate `crates/ramshaper-subject`:

- Runs the SAME bounded-CPU task-latency workload as `ramshaper-bench::workload`
  — it calls `run_workload` itself, so spec fidelity is by construction, and the
  doc-08 minimum (>= 100 events) is enforced by the bench validator, never
  re-implemented.
- Spec input: closed-schema JSON (`SubjectSpec`, field names identical to
  `WorkloadSpec`, `deny_unknown_fields`, bench defaults) — the orchestrator
  (ticket 3) will write the same JSON for both sides.
- Always observe-only BY ITSELF: the treatment closure is a no-op; in a real
  trial the agent applies any priority change externally through its consent
  gate. The subject has no IPC, no OS mutation, no file writes.
- Output: one `ramshaper.subject-run.v1` JSON record (schema, version, run_id,
  spec echo, per-task latencies in ms, completion status). Exit 0 complete /
  1 stopped-early / 2 usage (nothing ran).
- Image name `ramshaper-subject.exe` is deliberately NOT in the agent's
  self-identification list (`ramshaper-agent/cli/ui.exe`) — it is a workload
  simulator, the subject of the experiment; that reasoning lands in ADR-045.

Two soak-constraint decisions recorded in the source docs: the crate depends on
the bench **lib** (unchanged, so the soak-held `ramshaper-bench.exe` was never
relinked — the release build compiled only the new binary, verified live), and
the spec is mirrored locally because the bench types carry no serde derives and
the bench crate is frozen during the soak.

## Files

- `trial-spec.json` — the demo spec (250 events, 10 warmup, default effort).
- `live-run.json` — the release binary's record: completion complete,
  250 measured events, p50 0.84 ms / p95 1.11 ms (the ~1 ms/task design
  target).

## Gates

fmt/clippy (`-D warnings`) clean; workspace **436 passed / 0 failed** serially
(+9: 4 unit, 5 integration over the real binary via
`CARGO_BIN_EXE_ramshaper-subject`, incl. the closed-schema refusal and the
spec-validation reason carried verbatim from the bench validator).
