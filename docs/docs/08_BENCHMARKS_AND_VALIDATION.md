# RAMShaper — Benchmark and Validation Protocol

**Revision:** 0.2 • **Date:** 2026-10-22 • **Status:** proposed experiment protocol; no measured RAMShaper results.

## 1. Questions and measurement boundaries

Does a specific intervention improve a defined foreground task's p95 latency without unacceptable foreground/background completion cost, instability or observer overhead? Does the result replicate on the declared hardware?

Measure low-memory conditions as context. Do not infer success from available RAM or working-set changes. Process I/O is not physical disk traffic; total faults are not hard faults. Record unsupported instruments as missing, never zero.

Define each task's start and completion marker before measurement. For app switching, use a reproducible fixture and an acknowledged UI-ready marker, not only a focus event. A focus event alone does not prove interactivity. Instrumentation/calibration must be included in both conditions.

## 2. Hardware and experiment manifest

| Cohort | Memory | Storage | Representative workload |
|---|---|---|---|
| A | 4 GB | HDD | Local browser pages + editor + bounded background fixture |
| B | 4 GB | SSD | Same inputs to examine storage sensitivity |
| C | 8 GB | SSD | Fixed IDE/project/browser plus synthetic optional workload |
| D | 16 GB | SSD | No-pressure control and observer effect |

Record OS edition/build/updates, CPU/logical processors, RAM, storage, drivers, power plan/source, thermal conditions, startup load, pagefile configuration, security software, toolchain and code revision. Do not pool cohorts, OS builds or different policy versions into one benefit number. A VM is useful for faults/crashes but does not replace physical HDD/SSD overhead evidence. Missing hardware stays unverified.

Preregister a manifest with workload/input/script hashes, policy/TTL/rule version, timing markers, warmup, random seed/order, run count, abort rules, missing-data handling, primary endpoint, guardrails and analysis version. Freeze it before seeing condition results.

## 3. Trial design

- Observer study: A = agent absent; B = Observe Only. Keep the same external measurement instrument in both.
- Policy study: A = Observe Only; B = one specified policy with consent and identical collector settings.
- Counterbalance paired runs using randomized AB/BA blocks. Keep workload data, timing, app versions and load generation fixed.
- Separate cold launch, warm launch and app-switch experiments. Apply resets consistently according to the manifest.
- Pilot minimum: 10 paired blocks. Each run should contain at least 100 valid repeated task events for a descriptive p95; otherwise label tail estimates exploratory. A pilot count is not proof of statistical power.
- Start with a fixed 2-minute warmup excluded by design, then a fixed measurement duration/event schedule. Different workload needs must be declared before the trial, not after inspecting results.
- Observe-only and treatment runs must have the same capture duration, even if a job finishes early; record completion separately.
- Synthetic pressure workloads run only in a disposable test environment, with a bounded allocation/disk workload, stop control and predeclared abort threshold. Do not induce pressure on the user's live workspace.

Do not silently drop failed or aborted treatment runs. Preserve their condition/reason and count them against safety/completeness. Replace a measurement-invalid pair only under a preregistered rule and retain the original in the dataset.

## 4. Endpoints and decision rule

Use **per-run p95 task latency** as the primary endpoint. Compute p50/p95 by nearest rank: sort n valid events and take position ceil(q × n), with q = 0.50/0.95 (one-based). Report n and absolute milliseconds. Do not calculate a cohort p95 by averaging run percentiles or treating every event as an independent run.

Pair A/B runs by block. Report each run, the median paired relative change and a 95% bootstrap interval obtained by resampling whole paired blocks (10,000 resamples, recorded seed). Do not resample correlated individual events as if independent. Label small-sample intervals exploratory.

Planning thresholds below must be frozen or replaced with a justified workload-specific rule before data collection:

- Meaningful primary benefit: at least 5% lower paired p95 latency, with the 95% interval excluding zero improvement.
- Foreground task completion guardrail: upper 95% interval bound for percentage slowdown no greater than 2%.
- Background task completion guardrail: upper bound no greater than 10%; report throughput and the user's actual tradeoff.
- Any unexpected kill, forced suspension, data corruption or reproducible app instability: safety failure regardless of speed.
- No-pressure control should propose no action; observer study must meet the resource targets separately.

For a lower-is-better measure, per-pair improvement is `100 * (A - B) / A`, only when A > 0. For completion slowdown use `100 * (B - A) / A`. Report absolute values beside percentages. Missing/nonpositive denominators produce unavailable deltas, never infinity.

| Label | Rule |
|---|---|
| Validated improvement within tested conditions | Primary threshold/interval and all safety/completion/overhead guardrails pass |
| Inconclusive | Underpowered/insufficient/invalid measurements or uncertainty cannot resolve benefit or guardrail |
| No measured improvement | Adequate protocol completed but meaningful gain criterion is not met, without an established regression |
| Regression | Established latency/completion regression, guardrail violation or safety failure; disable implicated policy pending review |

A point estimate alone cannot pass a noisy guardrail. Do not interpret lack of significance as proof of equivalence. User benchmark results cannot automatically change machine-wide policies.

## 5. Observer overhead and numerical correctness

Measure agent working set, private commit, normalized CPU, handles, wakeups and disk I/O. On each cohort: 5-minute warmup, 30-minute idle capture, repeated three times; separately perform a 24-hour bounded-retention/handle soak. Report mean/p95/max and changes over time. Agent-hidden and UI-visible runs are separate, with the same baseline load.

Provisional hidden-agent targets: mean steady-state working set <25 MiB and mean normalized CPU <1%. These are goals, not current results. Report peak and private commit too, so average RSS cannot hide spikes or retained allocations. UI budget is selected after the Slint prototype (the prototype was measured 2026-10-24; a proposed budget with rationale is prepared in [design/2026-10-24-ui-budget-memo.md](../design/2026-10-24-ui-budget-memo.md), pending selection); no unsupported tiny-footprint claim.

Fixture math must be exact. For live memory comparisons, pair the displayed conversion with the same raw API capture; use a separately timed Windows tool only as a labeled approximation. Do not assert an arbitrary tolerance without recording timestamps, raw units and expected sampling variation.

## 6. Export schema v1

Use a manifest plus separate event and sample files; one summary row cannot reproduce a tail percentile. Schema/version and missing-value semantics must be documented beside every export.

```text
manifest.json:
  schema_version, session_id, source_revision, documentation_revision,
  hardware, os_build, workload_hash, script_hash, policy, rule_version,
  random_seed, run_order, warmup, event_schedule, endpoints, guardrails,
  missing_data_rule, analysis_version, artifacts_and_sha256

runs.csv:
  run_id,pair_id,condition,order,start_utc,duration_s,planned_events,valid_events,
  missing_events,aborted,abort_reason,foreground_completion_s,background_completion_s,
  error_count,action_id

events.csv:
  run_id,event_id,task_kind,start_monotonic_ns,duration_ms,status,error_code

samples.csv:
  run_id,sequence,elapsed_ms,metric,value,unit,status,source
```

Condition is `absent`, `observe` or `policy`. Monotonic values are local to a run and never joined across restarts. CSV missing numeric fields are empty with a non-OK status; JSON uses null/status. Include units in every metric definition. Preserve raw sampling gaps and failed events.

By default invalidate primary analysis for a run with >5% missing scheduled events or any missing primary completion outcome; count/investigate it and its pair, do not claim improvement from the surviving tail. A different threshold must be preregistered. Default exports exclude identifying paths/arguments and recovery journals. Mark synthetic datasets `synthetic=true` prominently.

## 7. Reports and release evidence

Every report includes date/operator, environment, script/input hashes, raw files/checksums, condition/order, all run/event counts, missing/abort reasons, absolute p50/p95, paired interval, foreground/background cost, overhead, crashes and result label. Publish neutral and negative outcomes alongside positive ones.

Validate three workload classes with independently rerunnable scripts before broad performance claims. AT-20–AT-22 define the evidence gates. There are no result rows in this document because no RAMShaper experiment has run.