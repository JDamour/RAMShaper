# AT-06 24-hour soak — pass criteria (declared before the run)

Defined 2026-10-24 before the run started. Machine: jmpat dev workstation —
Windows 11 Pro 10.0.26200.9711, i9-12900HK (20 logical CPUs), 64 GiB, standard
user. This is a **non-cohort** machine (AT-21 declared 4/8/16 GiB cohorts are
unavailable); the run extends the AT-21 machine-local baseline over time, it
does not establish a cohort claim.

## Run definition

- Primary channel (RS-024A/RS-025): `ramshaper-bench observer-overhead
  --iterations 500 --json` (release build), one report per 10-minute window for
  24 hours → up to **144 reports**, 500 samples per report (72,000 samples per
  metric over the run).
- Secondary channel: persistent `ramshaper-cli low-memory-watch --json
  --interval-ms 1000` running for the full window, exercising RS-007 telemetry,
  RS-008 bounded history and RS-009 pressure continuously at 1 Hz (≈ 86,400
  samples). Its JSONL stream carries `sample_sequence` and `generated_at_utc`
  (millisecond precision) per line.
- Driver: `soak-driver.ps1` under a Windows scheduled task
  ("RAMShaper AT-06 Soak"), writing `soak.jsonl` (one JSON record per window),
  `driver.log` (progress), `watch.jsonl` / `watch.err.log` (watch channel).
- Verdict tool: `python analyze_soak.py` in this directory (works on partial
  data for interim checks; final only after the driver writes its `done`
  record).

## Baseline (AT-21, 2026-10-24, same machine, 500 iterations, release build)

| metric | p50 | p95 | max |
|---|---|---|---|
| system snapshot | 0.060 ms | 0.083 ms | 0.372 ms |
| process scan (~347 processes) | 17.0 ms | 21.8 ms | 26.7 ms |
| bench self working set | 4.6 MiB | — | — |

Budget sources: NFR-01 provisional hidden-agent targets (doc 08 §5: mean
steady-state working set < 25 MiB, mean normalized CPU < 1%). No soak pass
thresholds existed before this document; the headroom multiples below are the
declared criteria.

## C1 — Completion / dropped samples

- Bench channel: every 10-minute window must produce a captured record with
  `exit_code == 0` and a parseable `ramshaper.observer_overhead.v1` report.
  **PASS** if ≥ 142 of 144 windows are captured (≤ 2 dropped ≈ 1.4%), and no
  more than 2 consecutive windows are dropped or failed.
- Watch channel: **PASS** if `sample_sequence` is continuous `1..N` (no missing
  numbers), ≥ 99 % of the expected 1 Hz samples over the elapsed window are
  present, and no wall-clock gap between consecutive samples exceeds 5 s
  (5× cadence — flags missed ticks or machine suspension; single-tick misses
  under load are within tolerance at this threshold).

## C2 — Overhead budget (NFR-01 provisional)

- **Working set < 25 MiB** for every bench report (`bench_self_working_set_bytes`
  and `bench_self_private_commit_bytes`) and for every watch working-set sample
  taken by the driver.
- **Normalized CPU < 1 %**, derived (not directly measured): at the 1 Hz observe
  cadence on 20 logical CPUs, `mean(scan p50) / 1000 / 20`. The baseline implies
  ≈ 0.09 %. Recorded as an estimate; a true CPU measurement (RS-024 Stage B /
  perf counters) is out of scope for this run and stated as such.

## C3 — Observer latency stability (soak p50 / p99, nearest-rank per doc 08 §4)

Scan thresholds are **per-process** because scan cost scales with inventory
size: the AT-21 baseline is 17.0 ms / 21.8 ms / 26.7 ms at ~347 processes
(0.049 / 0.063 / 0.077 ms per process), and a pre-launch smoke check on the
same day measured 457 processes. A fixed millisecond threshold would fail on
workload growth rather than observer degradation; the invariant under test is
the cost per observed process.

With N = median of `process_count_last` over the run:

- Process scan: soak p50 (median over per-report p50s) ≤ **max(20 ms,
  0.061 × N)**; soak p99 (nearest-rank p99 over per-report p95 values)
  ≤ **max(35 ms, 0.088 × N)**; absolute max ≤ **max(60 ms, 0.123 × N)**.
  Headroom: +25 % / +40 % / +60 % over the baseline per-process rates; the
  absolute floors are the idle-machine values used before the per-process
  model was adopted.
- System snapshot (inventory-size independent): soak p50 **≤ 0.15 ms**,
  soak p99 **≤ 0.5 ms**, max **≤ 2 ms**.
- `process_count_last` ≥ 100 in every report (the scan must remain a real
  inventory; baseline ≈ 347–457).
- Degradation guard: median scan p50 of the last 12 windows (2 h) ≤ median of
  the first 12 windows × 1.25.

## C4 — No unbounded growth

- Bench working set: max < 25 MiB (C2) **and** least-squares slope of bench WS
  over the 24 h series < +50 KiB/hour. Each bench run is a fresh process, so
  this measures cost stability across the day, not process-lifetime growth.
- Watch working set: slope < +100 KiB/hour and final WS < 25 MiB. The watch is
  the long-lived observer — this is the actual leak check for RS-008 bounded
  history.

## Verdict rule

**PASS requires C1, C2, C3 and C4 to pass simultaneously.** Any failure fails
the soak; the analyzer quotes the failing numbers. An interim analyzer run over
partial data reports progress but the verdict is only final after the driver's
`done` record.

## Known limitations (declared up front)

- Single machine, non-cohort. AT-21's declared 4/8/16 GiB cohort matrix remains
  open and unaffected by this run.
- The bench measures collector paths in short-lived processes; long-lived
  collector behavior (RS-008 history) comes from the watch channel only.
- If the graceful console-event stop at the end fails, the watch is hard-killed:
  history counters (stored/gaps/evicted/expired/rejected) are then derived from
  the JSONL stream (sequence continuity + cadence) rather than the CLI's own
  exit summary, and this limitation is recorded in the verdict.
- The driver holds `ES_SYSTEM_REQUIRED` (SetThreadExecutionState) for its
  lifetime so an idle sleep timer cannot suspend the machine mid-soak. It
  changes no persistent power setting; display-off is not prevented. Other
  load on the machine (compiles, antivirus scans) is part of the environment
  being soaked and is not controlled.
