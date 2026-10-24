# record_verdict.py - renders the AT-06 soak verdict into an evidence summary
# ready to merge into the canonical docs (16 evidence, 15 test matrix).
#
# Usage:
#   python record_verdict.py             # dry run: writes verdict-summary.md only
#
# Run any time (interim render) or after the driver writes its 'done' record
# (final render, marked FINAL). The canonical numbered docs are hand-maintained;
# this script never edits them - it produces verdict-summary.md with two
# ready-to-paste blocks:
#   1. a "### AT-06 - 24-hour soak (evidence)" section for doc 16
#   2. the replacement status sentence for doc 15's header
# Merging those two blocks is a human/session decision, not this script's.

import hashlib
import json
import os
import subprocess
import sys
from datetime import datetime, timezone

SOAK_DIR = os.path.dirname(os.path.abspath(__file__))
REPO = r"F:\_RAMShaper"
BENCH_EXE = os.path.join(REPO, r"target\release\ramshaper-bench.exe")
CLI_EXE = os.path.join(REPO, r"target\release\ramshaper-cli.exe")


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main():
    # 1. authoritative verdict from the analyzer (exit 0 pass / 1 fail / 2 interim)
    proc = subprocess.run(
        [sys.executable, os.path.join(SOAK_DIR, "analyze_soak.py")],
        capture_output=True, text=True, encoding="utf-8")
    analyzer_out = proc.stdout.strip()
    verdict_code = proc.returncode
    verdict_label = {0: "PASS", 1: "FAIL"}.get(verdict_code, "INTERIM")

    # 2. soak.jsonl facts
    start_rec = done_rec = None
    n_report = n_error = 0
    with open(os.path.join(SOAK_DIR, "soak.jsonl"), encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            rec = json.loads(line)
            t = rec.get("type")
            if t == "start":
                start_rec = rec
            elif t == "done":
                done_rec = rec
            elif t == "report":
                n_report += 1
            elif t == "error":
                n_error += 1

    final = done_rec is not None
    status = "FINAL" if final else "INTERIM (run in progress)"
    if final:
        status = f"FINAL - {verdict_label}" if verdict_code in (0, 1) else f"FINAL - analyzer said {verdict_label}"

    # 3. artifact inventory + hashes
    artifacts = []
    for name in ("soak.jsonl", "driver.log", "watch.jsonl", "watch.err.log"):
        path = os.path.join(SOAK_DIR, name)
        if not os.path.exists(path):
            continue
        with open(path, "rb") as f:
            lines = sum(1 for _ in f)
        artifacts.append((name, os.path.getsize(path), lines, sha256(path)))

    binaries = []
    for name, path in (("ramshaper-bench.exe", BENCH_EXE), ("ramshaper-cli.exe", CLI_EXE)):
        if os.path.exists(path):
            binaries.append((name, sha256(path)))

    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

    # 4. render verdict-summary.md
    out = []
    out.append(f"# AT-06 soak verdict summary - generated {now}")
    out.append("")
    out.append(f"**Status: {status}** (analyzer exit {verdict_code})")
    out.append("")
    out.append(f"- Soak start: {start_rec['started_at_utc']}, planned end: {start_rec['planned_end_utc']}")
    out.append(f"- Windows: {n_report} reports, {n_error} error records")
    if done_rec:
        out.append(f"- Completion record: {done_rec['completed_at_utc']}, "
                   f"watch stop mode: {done_rec.get('watch_stop_mode')}, watch exit: {done_rec.get('watch_exit_code')}")
    out.append(f"- Driver host: {start_rec.get('driver_host', 'see RUN_LOG.md')}")
    out.append(f"- Cadence: {start_rec['tick_seconds']} s / window, {start_rec['iterations']} iterations per report")
    out.append("")
    out.append("## Analyzer output (authoritative criteria table)")
    out.append("")
    out.append("```")
    out.append(analyzer_out)
    out.append("```")
    out.append("")
    out.append("## Binary provenance (SHA-256)")
    out.append("")
    for name, digest in binaries:
        out.append(f"- `{name}` `{digest}`")
    out.append("")
    out.append("## Raw artifacts (size, lines, SHA-256)")
    out.append("")
    out.append("| file | bytes | lines | sha256 |")
    out.append("|---|---|---|---|")
    for name, size, lines, digest in artifacts:
        out.append(f"| {name} | {size} | {lines} | `{digest}` |")
    out.append("")

    limitations = [
        "Single machine, non-cohort (jmpat workstation, 64 GiB); AT-21's declared 4/8/16 GiB cohorts remain unavailable and unaffected.",
        "Driver attempts 1-2 (powershell hosts via schtasks / WMI) were terminated externally within minutes; attempt 3 (pythonw host, WMI launch) ran to completion. Details in RUN_LOG.md; dead-attempt artifacts in attempt1-schtask/ and attempt2-wmi-ps/.",
        "The bench channel measures collector paths in short-lived processes; long-lived collector behavior (RS-008 bounded history) comes from the 1 Hz watch channel.",
    ]
    if not final or (done_rec and done_rec.get("watch_stop_mode") != "graceful"):
        limitations.append(
            "The watch was not stopped via its console Ctrl+C handler (no console in the CREATE_NO_WINDOW "
            "launch configuration), so the CLI's own exit history summary (stored/gaps/evicted/expired/rejected) "
            "is not available; sequence continuity and cadence were derived from the watch.jsonl stream, and the "
            "final process state from the driver's termination record. Declared limitation in PASS_CRITERIA.md.")
    out.append("## Limitations")
    out.append("")
    for lim in limitations:
        out.append(f"- {lim}")
    out.append("")

    # 5. ready-to-paste blocks for the canonical docs
    out.append("---")
    out.append("")
    out.append("## Block 1 - paste into docs/docs/16_IMPLEMENTATION_EVIDENCE.md (G1 area)")
    out.append("")
    out.append("### AT-06 - 24-hour bounded soak (evidence)")
    out.append("")
    out.append(f"Status: **{'Passed' if verdict_label == 'PASS' else verdict_label if final else 'In progress'}** "
               f"(this section is a draft rendered by record_verdict.py; review before adopting)")
    out.append("")
    out.append(f"- Run window: {start_rec['started_at_utc']} to "
               f"{(done_rec or {}).get('completed_at_utc', 'in progress (planned ' + start_rec['planned_end_utc'] + ')')}, "
               f"{n_report} of 144 expected windows captured, {n_error} error records.")
    out.append("- Channels: `ramshaper-bench observer-overhead --iterations 500 --json` every 10 minutes "
               "(RS-024A/RS-025) plus a persistent `ramshaper-cli low-memory-watch --json --interval-ms 1000` "
               "1 Hz channel exercising RS-007/008/009 throughout.")
    out.append("- Pass criteria were declared in `docs/evidence/soak/at06-20261024/PASS_CRITERIA.md` before "
               "launch: C1 dropped samples/sequence continuity, C2 NFR-01 provisional budgets (WS < 25 MiB, "
               "derived normalized CPU < 1 %), C3 per-process scan p50/p99 (thresholds scale with process "
               "count; baseline AT-21 = 0.049/0.063/0.077 ms per process at ~347 processes), C4 no-growth "
               "slopes (bench WS and long-lived watch WS; the RS-008 leak check).")
    out.append("- Full criteria table: see the analyzer output above; raw artifacts and hashes listed above; "
               "PASS_CRITERIA.md declares the thresholds and limitations up front.")
    out.append("")
    out.append("## Block 2 - replacement sentence for docs/docs/15_ACCEPTANCE_TEST_MATRIX.md header")
    out.append("")
    doc15 = ("AT-01-AT-05 **Passed** 2026-10-23; AT-06 (deterministic portion), AT-07, AT-08 (core/config "
             "portion) **Passed** 2026-10-24")
    soak_clause = ("the AT-06 24-hour soak **Passed** 2026-10-25" if verdict_label == "PASS" and final
                   else f"the AT-06 24-hour soak **{verdict_label}** 2026-10-25" if final
                   else "the 24-hour soak of AT-06")
    out.append(f"{doc15}; {soak_clause}, the AT-21 declared cohorts, and all later cases are "
               f"**{'Not run' if not final else 'as shown'}**. See 16_IMPLEMENTATION_EVIDENCE.md.")
    out.append("")

    out_path = os.path.join(SOAK_DIR, "verdict-summary.md")
    with open(out_path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(out) + "\n")
    print(f"wrote {out_path} (status: {status})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
