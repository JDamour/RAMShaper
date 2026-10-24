#!/usr/bin/env python3
"""AT-06 24-hour soak analyzer - evaluates the run against PASS_CRITERIA.md.

Usage (any time, works on partial data):
    python analyze_soak.py

Reads soak.jsonl and watch.jsonl from this script's directory. Exit codes:
0 = PASS, 1 = FAIL (criterion violated), 2 = interim (run not finished).
The verdict is only final after the driver has written its 'done' record.
"""
import json
import math
import os
import sys
from datetime import datetime, timezone
from statistics import mean, median

SOAK_DIR = os.path.dirname(os.path.abspath(__file__))
TICK_SECONDS = 600
EXPECTED_WINDOWS = 144
MIB = 1024 * 1024

# C2/C3 thresholds (PASS_CRITERIA.md)
WS_LIMIT_BYTES = 25 * MIB
SCAN_P50_FLOOR_MS = 20.0     # floors = idle-machine absolute values
SCAN_P99_FLOOR_MS = 35.0
SCAN_MAX_FLOOR_MS = 60.0
SCAN_P50_PER_PROC_MS = 0.061  # baseline 0.049 ms/proc + 25% headroom
SCAN_P99_PER_PROC_MS = 0.088  # baseline 0.063 ms/proc + 40% headroom
SCAN_MAX_PER_PROC_MS = 0.123  # baseline 0.077 ms/proc + 60% headroom
SNAP_P50_LIMIT_MS = 0.15
SNAP_P99_LIMIT_MS = 0.5
SNAP_MAX_LIMIT_MS = 2.0
PROC_COUNT_LIMIT = 100
DEGRADED_FACTOR = 1.25
SLOPE_BENCH_KIB_H = 50.0
SLOPE_WATCH_KIB_H = 100.0

# C1 thresholds
MIN_CAPTURED_FRACTION = 1 - 2 / EXPECTED_WINDOWS  # >= 142 of 144
MAX_CONSECUTIVE_DROPPED = 2
WATCH_GAP_LIMIT_S = 5.0
WATCH_PRESENT_FRACTION = 0.99
LP_COUNT = 20  # logical processors on this machine (derived-CPU estimate only)


def load_jsonl(path):
    records, bad = [], 0
    if not os.path.exists(path):
        return records, bad
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                records.append(json.loads(line))
            except json.JSONDecodeError:
                bad += 1
    return records, bad


def parse_ts(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00")).astimezone(timezone.utc)


def nearest_rank(sorted_vals, q):
    n = len(sorted_vals)
    if n == 0:
        return None
    return sorted_vals[max(1, math.ceil(q * n)) - 1]


def slope_kib_per_hour(samples):
    """Least-squares slope (KiB/hour) over (window_index, bytes) samples."""
    if len(samples) < 3:
        return None
    n = len(samples)
    xs = [s[0] for s in samples]
    ys = [s[1] for s in samples]
    mx, my = mean(xs), mean(ys)
    den = sum((x - mx) ** 2 for x in xs)
    if den == 0:
        return None
    per_tick = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / den
    return per_tick * (3600.0 / TICK_SECONDS) / 1024.0


class Verdict:
    def __init__(self):
        self.rows = []

    def add(self, name, measured, threshold, ok, note=""):
        self.rows.append((name, measured, threshold, ok, note))

    @property
    def failed(self):
        return [r for r in self.rows if r[3] is False]

    def render(self):
        w = max(len(r[0]) for r in self.rows) + 2
        for name, measured, threshold, ok, note in self.rows:
            mark = "PASS" if ok is True else ("FAIL" if ok is False else "SKIP")
            print(f"  [{mark}] {name:<{w}} measured={measured}  limit={threshold}  {note}".rstrip())


def main():
    records, bad_lines = load_jsonl(os.path.join(SOAK_DIR, "soak.jsonl"))
    if not records:
        print("no soak.jsonl records yet - the driver has not started (or wrong directory)")
        return 2

    start = next((r for r in records if r.get("type") == "start"), None)
    reports = [r for r in records if r.get("type") == "report"]
    errors = [r for r in records if r.get("type") == "error"]
    done = next((r for r in records if r.get("type") == "done"), None)
    if start is None:
        print("soak.jsonl has no 'start' record - file is incomplete/corrupt")
        return 1

    started = parse_ts(start["started_at_utc"])
    planned_end = parse_ts(start["planned_end_utc"])
    now = datetime.now(timezone.utc)
    elapsed_s = max(0.0, (now - started).total_seconds())
    expected = min(EXPECTED_WINDOWS, int(elapsed_s // TICK_SECONDS) + 1)
    finished = done is not None

    print(f"AT-06 soak: started {start['started_at_utc']}, planned end {start['planned_end_utc']}")
    print(f"  windows captured={len(reports)} failed={len(errors)} expected-so-far={expected}"
          f" of {EXPECTED_WINDOWS}  (unparseable lines: {bad_lines})")
    print(f"  status: {'COMPLETE' if finished else 'IN PROGRESS'}"
          f"  elapsed={elapsed_s / 3600.0:.2f} h")
    if done:
        print(f"  watch stop mode: {done.get('watch_stop_mode')} exit={done.get('watch_exit_code')}")
    print()

    v = Verdict()

    # ---- C1 bench channel: dropped / failed windows ----
    captured_windows = {r.get("window") for r in reports}
    dropped = expected - len(captured_windows)
    streak = worst = 0
    for w in range(expected):
        if w in captured_windows:
            streak = 0
        else:
            streak += 1
            worst = max(worst, streak)
    v.add("C1 bench: dropped windows", f"{dropped} dropped", f"<= {EXPECTED_WINDOWS - int(MIN_CAPTURED_FRACTION * EXPECTED_WINDOWS)}",
          (dropped <= EXPECTED_WINDOWS - int(MIN_CAPTURED_FRACTION * EXPECTED_WINDOWS)) if expected >= 12 else None,
          "checked once >=12 windows elapsed")
    v.add("C1 bench: consecutive dropped", f"max streak {worst}", f"<= {MAX_CONSECUTIVE_DROPPED}",
          worst <= MAX_CONSECUTIVE_DROPPED if expected >= 3 else None)

    # ---- C1 watch channel: sequence continuity, cadence gaps, presence ----
    watch_records, watch_bad = load_jsonl(os.path.join(SOAK_DIR, "watch.jsonl"))
    seqs, gaps_over, max_gap = [], 0, 0.0
    prev_t = None
    for r in watch_records:
        if not isinstance(r, dict) or "sample_sequence" not in r:
            continue
        seqs.append(int(r["sample_sequence"]))
        t = parse_ts(r["generated_at_utc"])
        if prev_t is not None:
            gap = (t - prev_t).total_seconds()
            max_gap = max(max_gap, gap)
            if gap > WATCH_GAP_LIMIT_S:
                gaps_over += 1
        prev_t = t
    if seqs:
        continuous = seqs == list(range(seqs[0], seqs[0] + len(seqs)))
        expected_samples = max(1, int(elapsed_s))
        present = len(seqs) / float(expected_samples)
        v.add("C1 watch: sample_sequence continuity",
              f"{'continuous' if continuous else 'BROKEN'} ({len(seqs)} samples, first={seqs[0]}, last={seqs[-1]})",
              "1..N no gaps", continuous)
        v.add("C1 watch: cadence gaps > 5 s", f"{gaps_over} (max {max_gap:.2f} s)", "0",
              gaps_over == 0 if len(seqs) >= 60 else None, "checked once >=60 samples")
        v.add("C1 watch: samples present vs 1 Hz expectation", f"{present * 100:.2f}%",
              f">= {WATCH_PRESENT_FRACTION * 100:.1f}%",
              present >= WATCH_PRESENT_FRACTION if len(seqs) >= 60 else None)
    else:
        v.add("C1 watch: samples present", "no samples", ">= 99%", False, "watch channel produced nothing")

    # ---- C2: overhead budget (NFR-01 provisional) ----
    scan_p50s = [r["report"]["process_scan"]["p50_ms"] for r in reports]
    scan_p95s = [r["report"]["process_scan"]["p95_ms"] for r in reports]
    scan_maxs = [r["report"]["process_scan"]["max_ms"] for r in reports]
    snap_p50s = [r["report"]["system_snapshot"]["p50_ms"] for r in reports]
    snap_p95s = [r["report"]["system_snapshot"]["p95_ms"] for r in reports]
    snap_maxs = [r["report"]["system_snapshot"]["max_ms"] for r in reports]
    proc_counts = [r["report"]["process_count_last"] for r in reports]
    bench_ws = [(r["window"], r["report"]["bench_self_working_set_bytes"]) for r in reports]
    bench_priv = [(r["window"], r["report"]["bench_self_private_commit_bytes"]) for r in reports]
    watch_ws = [(r["window"], r["watch_ws_bytes"]) for r in reports if r.get("watch_ws_bytes")]

    if reports:
        max_bench_ws = max(w for _, w in bench_ws)
        max_bench_priv = max(p for _, p in bench_priv)
        max_watch_ws = max(w for _, w in watch_ws) if watch_ws else None
        cpu_est = mean(scan_p50s) / 1000.0 / LP_COUNT * 100.0
        v.add("C2: max bench working set", f"{max_bench_ws / MIB:.2f} MiB", "< 25 MiB", max_bench_ws < WS_LIMIT_BYTES)
        v.add("C2: max bench private commit", f"{max_bench_priv / MIB:.2f} MiB", "< 25 MiB", max_bench_priv < WS_LIMIT_BYTES)
        v.add("C2: max watch working set",
              f"{max_watch_ws / MIB:.2f} MiB" if max_watch_ws is not None else "n/a",
              "< 25 MiB", max_watch_ws < WS_LIMIT_BYTES if max_watch_ws is not None else None)
        v.add("C2: derived normalized CPU (estimate)",
              f"{cpu_est:.3f}%", "< 1%", cpu_est < 1.0,
              f"= mean(scan p50)/1000/{LP_COUNT} LPs at 1 Hz; estimate, not a measurement")

        # ---- C3: observer latency stability (per-process scan thresholds) ----
        soak_scan_p50 = median(scan_p50s)
        soak_scan_p99 = nearest_rank(sorted(scan_p95s), 0.99)  # p99 proxy from per-report p95
        soak_scan_max = max(scan_maxs)
        soak_snap_p50 = median(snap_p50s)
        soak_snap_p99 = nearest_rank(sorted(snap_p95s), 0.99)
        soak_snap_max = max(snap_maxs)
        n_procs = median(proc_counts)
        scan_p50_limit = max(SCAN_P50_FLOOR_MS, SCAN_P50_PER_PROC_MS * n_procs)
        scan_p99_limit = max(SCAN_P99_FLOOR_MS, SCAN_P99_PER_PROC_MS * n_procs)
        scan_max_limit = max(SCAN_MAX_FLOOR_MS, SCAN_MAX_PER_PROC_MS * n_procs)
        v.add("C3: scan soak p50 (median of report p50s)", f"{soak_scan_p50:.3f} ms",
              f"<= {scan_p50_limit:.1f} ms (0.061 x N={n_procs:.0f})",
              soak_scan_p50 <= scan_p50_limit)
        v.add("C3: scan soak p99 (p99 of report p95s)", f"{soak_scan_p99:.3f} ms",
              f"<= {scan_p99_limit:.1f} ms (0.088 x N={n_procs:.0f})",
              soak_scan_p99 <= scan_p99_limit)
        v.add("C3: scan absolute max", f"{soak_scan_max:.3f} ms",
              f"<= {scan_max_limit:.1f} ms (0.123 x N={n_procs:.0f})",
              soak_scan_max <= scan_max_limit)
        v.add("C3: snapshot soak p50", f"{soak_snap_p50:.3f} ms", f"<= {SNAP_P50_LIMIT_MS} ms",
              soak_snap_p50 <= SNAP_P50_LIMIT_MS)
        v.add("C3: snapshot soak p99", f"{soak_snap_p99:.3f} ms", f"<= {SNAP_P99_LIMIT_MS} ms",
              soak_snap_p99 <= SNAP_P99_LIMIT_MS)
        v.add("C3: snapshot absolute max", f"{soak_snap_max:.3f} ms", f"<= {SNAP_MAX_LIMIT_MS} ms",
              soak_snap_max <= SNAP_MAX_LIMIT_MS)
        v.add("C3: min process_count_last", f"{min(proc_counts)}", f">= {PROC_COUNT_LIMIT}",
              min(proc_counts) >= PROC_COUNT_LIMIT)
        if len(reports) >= 24:
            first12 = median(scan_p50s[:12])
            last12 = median(scan_p50s[-12:])
            v.add("C3: degradation last12 vs first12",
                  f"last={last12:.3f} first={first12:.3f} (x{last12 / first12:.2f})",
                  f"<= x{DEGRADED_FACTOR}", last12 <= first12 * DEGRADED_FACTOR)
        else:
            v.add("C3: degradation last12 vs first12", "n/a", "needs >= 24 reports", None)

        # ---- C4: no unbounded growth ----
        b_slope = slope_kib_per_hour(bench_ws)
        w_slope = slope_kib_per_hour(watch_ws)
        v.add("C4: bench WS slope", f"{b_slope:+.1f} KiB/h" if b_slope is not None else "n/a",
              f"< +{SLOPE_BENCH_KIB_H} KiB/h",
              b_slope < SLOPE_BENCH_KIB_H if b_slope is not None else None)
        v.add("C4: watch WS slope",
              f"{w_slope:+.1f} KiB/h" if w_slope is not None else "n/a",
              f"< +{SLOPE_WATCH_KIB_H} KiB/h",
              w_slope < SLOPE_WATCH_KIB_H if w_slope is not None else None,
              "leak check for RS-008 bounded history")
        if watch_ws:
            v.add("C4: final watch WS", f"{watch_ws[-1][1] / MIB:.2f} MiB", "< 25 MiB",
                  watch_ws[-1][1] < WS_LIMIT_BYTES)

    print("Criteria (PASS_CRITERIA.md):")
    v.render()
    print()

    if not finished:
        print(f"VERDICT: INTERIM - run in progress ({len(reports)}/{EXPECTED_WINDOWS} windows); "
              f"ends {start['planned_end_utc']}. Failure rows above are already final for the data so far.")
        return 2

    note = ""
    if done.get("watch_stop_mode") not in ("graceful",):
        note = (" Note: watch was not stopped gracefully (stop mode "
                f"'{done.get('watch_stop_mode')}'); history counters derived from the "
                "JSONL stream, not the CLI exit summary (declared limitation).")
    if v.failed:
        print(f"VERDICT: FAIL - {len(v.failed)} criterion/criteria violated.{note}")
        return 1
    print(f"VERDICT: PASS - all C1-C4 criteria met.{note}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
