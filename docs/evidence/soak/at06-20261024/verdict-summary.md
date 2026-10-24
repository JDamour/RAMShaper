# AT-06 soak verdict summary - generated 2026-10-24T14:24:57Z

**Status: INTERIM (run in progress)** (analyzer exit 2)

- Soak start: 2026-10-24T12:27:44Z, planned end: 2026-10-25T12:27:44Z
- Windows: 12 reports, 0 error records
- Driver host: pythonw.exe (attempt 3)
- Cadence: 600 s / window, 500 iterations per report

## Analyzer output (authoritative criteria table)

```
AT-06 soak: started 2026-10-24T12:27:44Z, planned end 2026-10-25T12:27:44Z
  windows captured=12 failed=0 expected-so-far=12 of 144  (unparseable lines: 0)
  status: IN PROGRESS  elapsed=1.95 h

Criteria (PASS_CRITERIA.md):
  [PASS] C1 bench: dropped windows                       measured=0 dropped  limit=<= 2  checked once >=12 windows elapsed
  [PASS] C1 bench: consecutive dropped                   measured=max streak 0  limit=<= 2
  [PASS] C1 watch: sample_sequence continuity            measured=continuous (7036 samples, first=1, last=7036)  limit=1..N no gaps
  [PASS] C1 watch: cadence gaps > 5 s                    measured=0 (max 1.02 s)  limit=0  checked once >=60 samples
  [PASS] C1 watch: samples present vs 1 Hz expectation   measured=100.06%  limit=>= 99.0%
  [PASS] C2: max bench working set                       measured=4.92 MiB  limit=< 25 MiB
  [PASS] C2: max bench private commit                    measured=1.34 MiB  limit=< 25 MiB
  [PASS] C2: max watch working set                       measured=4.94 MiB  limit=< 25 MiB
  [PASS] C2: derived normalized CPU (estimate)           measured=0.164%  limit=< 1%  = mean(scan p50)/1000/20 LPs at 1 Hz; estimate, not a measurement
  [FAIL] C3: scan soak p50 (median of report p50s)       measured=31.596 ms  limit=<= 26.8 ms (0.061 x N=440)
  [FAIL] C3: scan soak p99 (p99 of report p95s)          measured=54.244 ms  limit=<= 38.7 ms (0.088 x N=440)
  [FAIL] C3: scan absolute max                           measured=80.905 ms  limit=<= 60.0 ms (0.123 x N=440)
  [PASS] C3: snapshot soak p50                           measured=0.074 ms  limit=<= 0.15 ms
  [PASS] C3: snapshot soak p99                           measured=0.246 ms  limit=<= 0.5 ms
  [PASS] C3: snapshot absolute max                       measured=1.987 ms  limit=<= 2.0 ms
  [PASS] C3: min process_count_last                      measured=436  limit=>= 100
  [SKIP] C3: degradation last12 vs first12               measured=n/a  limit=needs >= 24 reports
  [PASS] C4: bench WS slope                              measured=-19.3 KiB/h  limit=< +50.0 KiB/h
  [PASS] C4: watch WS slope                              measured=+76.1 KiB/h  limit=< +100.0 KiB/h  leak check for RS-008 bounded history
  [PASS] C4: final watch WS                              measured=4.94 MiB  limit=< 25 MiB

VERDICT: INTERIM - run in progress (12/144 windows); ends 2026-10-25T12:27:44Z. Failure rows above are already final for the data so far.
```

## Binary provenance (SHA-256)

- `ramshaper-bench.exe` `8206d2368128a67d3440e5be3be6f619e99632817f3ecde5fb74a416e63cc5fa`
- `ramshaper-cli.exe` `59d0bd53dd733b215c2cba776a022f525df631546cc2ba2c160cdeac75b5bc8e`

## Raw artifacts (size, lines, SHA-256)

| file | bytes | lines | sha256 |
|---|---|---|---|
| soak.jsonl | 13388 | 13 | `b09689292fd959870dbc0534d69b43114aa2a9c0a4ed6172a7392dc1f8b82020` |
| driver.log | 1639 | 14 | `8147eb7a7239802684684df5c2f70198ce3bb7fd0237152d2df742e4a25497f3` |
| watch.jsonl | 14715276 | 7036 | `16693699969a818f5052e839147507b1fbb0a445ec06edab445fcf0897d876d1` |
| watch.err.log | 66 | 1 | `88ae7d02b292f695007f9ee524189bb1d639388777a09377d7404d7acdd9e52d` |

## Limitations

- Single machine, non-cohort (jmpat workstation, 64 GiB); AT-21's declared 4/8/16 GiB cohorts remain unavailable and unaffected.
- Driver attempts 1-2 (powershell hosts via schtasks / WMI) were terminated externally within minutes; attempt 3 (pythonw host, WMI launch) ran to completion. Details in RUN_LOG.md; dead-attempt artifacts in attempt1-schtask/ and attempt2-wmi-ps/.
- The bench channel measures collector paths in short-lived processes; long-lived collector behavior (RS-008 bounded history) comes from the 1 Hz watch channel.
- The watch was not stopped via its console Ctrl+C handler (no console in the CREATE_NO_WINDOW launch configuration), so the CLI's own exit history summary (stored/gaps/evicted/expired/rejected) is not available; sequence continuity and cadence were derived from the watch.jsonl stream, and the final process state from the driver's termination record. Declared limitation in PASS_CRITERIA.md.

---

## Block 1 - paste into docs/docs/16_IMPLEMENTATION_EVIDENCE.md (G1 area)

### AT-06 - 24-hour bounded soak (evidence)

Status: **In progress** (this section is a draft rendered by record_verdict.py; review before adopting)

- Run window: 2026-10-24T12:27:44Z to in progress (planned 2026-10-25T12:27:44Z), 12 of 144 expected windows captured, 0 error records.
- Channels: `ramshaper-bench observer-overhead --iterations 500 --json` every 10 minutes (RS-024A/RS-025) plus a persistent `ramshaper-cli low-memory-watch --json --interval-ms 1000` 1 Hz channel exercising RS-007/008/009 throughout.
- Pass criteria were declared in `docs/evidence/soak/at06-20261024/PASS_CRITERIA.md` before launch: C1 dropped samples/sequence continuity, C2 NFR-01 provisional budgets (WS < 25 MiB, derived normalized CPU < 1 %), C3 per-process scan p50/p99 (thresholds scale with process count; baseline AT-21 = 0.049/0.063/0.077 ms per process at ~347 processes), C4 no-growth slopes (bench WS and long-lived watch WS; the RS-008 leak check).
- Full criteria table: see the analyzer output above; raw artifacts and hashes listed above; PASS_CRITERIA.md declares the thresholds and limitations up front.

## Block 2 - replacement sentence for docs/docs/15_ACCEPTANCE_TEST_MATRIX.md header

AT-01-AT-05 **Passed** 2026-10-23; AT-06 (deterministic portion), AT-07, AT-08 (core/config portion) **Passed** 2026-10-24; the 24-hour soak of AT-06, the AT-21 declared cohorts, and all later cases are **Not run**. See 16_IMPLEMENTATION_EVIDENCE.md.

