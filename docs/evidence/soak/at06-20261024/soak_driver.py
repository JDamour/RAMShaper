# soak_driver.py - AT-06 24-hour soak driver, attempt 3 (python host).
#
# Same artifacts and record schema as soak-driver.ps1 (attempts 1-2), so
# analyze_soak.py works unchanged. The host was moved from powershell.exe to
# python because both powershell attempts were terminated externally while
# idle-sleeping between ticks (attempt 1 via schtasks, attempt 2 via WMI,
# ~3 min / ~9 min lifetimes; the ramshaper-cli watch child survived both).
#
# Launched detached via WMI Win32_Process.Create running pythonw.exe (no
# console window; all children get CREATE_NO_WINDOW). Writes:
#   soak.jsonl    one JSON record per window ({type: start|report|error|done})
#   driver.log    human-readable progress lines
#   watch.jsonl   persistent 1 Hz low-memory-watch sample stream (channel 2)
#   watch.err.log watch stderr (start banner; no graceful summary if hard-killed)
#   watch.pid     watch process id
#
# Pass criteria: PASS_CRITERIA.md in this directory (declared before the run).

import ctypes
import json
import os
import subprocess
import sys
import time
from ctypes import wintypes
from datetime import datetime, timezone

SOAK_DIR = r"F:\_RAMShaper\docs\evidence\soak\at06-20261024"
BENCH_EXE = r"F:\_RAMShaper\target\release\ramshaper-bench.exe"
CLI_EXE = r"F:\_RAMShaper\target\release\ramshaper-cli.exe"
ITERATIONS = 500
TICK_SECONDS = 600
DURATION_HOURS = 24
CREATE_NO_WINDOW = 0x08000000

SOAK_JSONL = os.path.join(SOAK_DIR, "soak.jsonl")
DRIVER_LOG = os.path.join(SOAK_DIR, "driver.log")
WATCH_STDOUT = os.path.join(SOAK_DIR, "watch.jsonl")
WATCH_STDERR = os.path.join(SOAK_DIR, "watch.err.log")
WATCH_PID_FILE = os.path.join(SOAK_DIR, "watch.pid")
TMP_OUT = os.path.join(SOAK_DIR, ".tmp-bench-out.json")
TMP_ERR = os.path.join(SOAK_DIR, ".tmp-bench-err.txt")

TS_FMT = "%Y-%m-%dT%H:%M:%SZ"
kernel32 = ctypes.windll.kernel32
psapi = ctypes.windll.psapi


def now_utc_str():
    return datetime.now(timezone.utc).strftime(TS_FMT)


def log(msg):
    with open(DRIVER_LOG, "a", encoding="utf-8", newline="\n") as f:
        f.write(f"{now_utc_str()} {msg}\n")


def append_jsonl(obj):
    with open(SOAK_JSONL, "a", encoding="utf-8", newline="\n") as f:
        f.write(json.dumps(obj, separators=(",", ":")) + "\n")


class _ProcMemCounters(ctypes.Structure):
    _fields_ = [
        ("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t), ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t), ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t), ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t), ("PeakPagefileUsage", ctypes.c_size_t),
    ]


def process_memory(pid):
    """(working_set, pagefile/private) bytes, or (None, None)."""
    handle = kernel32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
    if not handle:
        return None, None
    try:
        pmc = _ProcMemCounters()
        pmc.cb = ctypes.sizeof(pmc)
        if not psapi.GetProcessMemoryInfo(handle, ctypes.byref(pmc), pmc.cb):
            return None, None
        return pmc.WorkingSetSize, pmc.PagefileUsage
    finally:
        kernel32.CloseHandle(handle)


def keep_system_awake():
    # ES_CONTINUOUS | ES_SYSTEM_REQUIRED - no persistent power-setting change.
    kernel32.SetThreadExecutionState.argtypes = [wintypes.UINT]
    kernel32.SetThreadExecutionState.restype = wintypes.UINT
    kernel32.SetThreadExecutionState(0x80000001)


def clear_keep_awake():
    kernel32.SetThreadExecutionState(0x80000000)  # ES_CONTINUOUS only


def graceful_stop(pid):
    """Broadcast CTRL_C to the pid's console (fails cleanly without one)."""
    try:
        kernel32.FreeConsole()
        if not kernel32.AttachConsole(wintypes.DWORD(pid)):
            return False
        kernel32.SetConsoleCtrlHandler(None, True)
        ok = bool(kernel32.GenerateConsoleCtrlEvent(0, 0))  # CTRL_C_EVENT
        kernel32.FreeConsole()
        return ok
    except Exception:
        return False


def main():
    if os.path.exists(SOAK_JSONL):
        log(f"ERROR: {SOAK_JSONL} already exists - refusing to start a second soak over it")
        sys.exit(1)

    keep_system_awake()

    started = datetime.now(timezone.utc)
    planned_end = started.timestamp() + DURATION_HOURS * 3600

    append_jsonl({
        "type": "start",
        "started_at_utc": now_utc_str(),
        "planned_end_utc": datetime.fromtimestamp(planned_end, timezone.utc).strftime(TS_FMT),
        "tick_seconds": TICK_SECONDS,
        "iterations": ITERATIONS,
        "bench_path": BENCH_EXE,
        "cli_path": CLI_EXE,
        "machine": os.environ.get("COMPUTERNAME", ""),
        "user": os.environ.get("USERNAME", ""),
        "driver_pid": os.getpid(),
        "driver_host": "pythonw.exe (attempt 3)",
    })
    log(f"soak start; planned end {datetime.fromtimestamp(planned_end, timezone.utc).strftime(TS_FMT)}; "
        f"tick={TICK_SECONDS}s; iterations={ITERATIONS}; pid={os.getpid()}")

    # Persistent watch (channel 2: RS-007/008/009 at 1 Hz).
    watch = None
    try:
        watch = subprocess.Popen(
            [CLI_EXE, "low-memory-watch", "--json", "--interval-ms", "1000"],
            stdout=open(WATCH_STDOUT, "wb"), stderr=open(WATCH_STDERR, "wb"),
            creationflags=CREATE_NO_WINDOW)
        with open(WATCH_PID_FILE, "w", encoding="ascii") as f:
            f.write(str(watch.pid))
        time.sleep(2)
        if watch.poll() is not None:
            log(f"ERROR: watch exited immediately (exit code {watch.returncode}) - see watch.err.log; bench channel continues")
            watch = None
        else:
            log(f"watch started pid={watch.pid}")
    except Exception as e:
        log(f"ERROR: watch failed to start: {e!r}")
        watch = None

    # Main loop: one bench report per window, anchored to the start wall clock.
    window = 0
    while True:
        tick_start = started.timestamp() + window * TICK_SECONDS
        wait_s = tick_start - datetime.now(timezone.utc).timestamp()
        if wait_s > 0:
            time.sleep(wait_s)
        if datetime.now(timezone.utc).timestamp() >= planned_end:
            break

        watch_alive = False
        watch_ws = None
        watch_priv = None
        if watch is not None and watch.poll() is None:
            watch_alive = True
            watch_ws, watch_priv = process_memory(watch.pid)

        t0 = time.monotonic()
        exit_code = None
        report = None
        err_text = None
        try:
            with open(TMP_OUT, "wb") as fo, open(TMP_ERR, "wb") as fe:
                bp = subprocess.run(
                    [BENCH_EXE, "observer-overhead", "--iterations", str(ITERATIONS), "--json"],
                    stdout=fo, stderr=fe, creationflags=CREATE_NO_WINDOW)
            exit_code = bp.returncode
            if exit_code == 0:
                with open(TMP_OUT, "rb") as f:
                    report = json.loads(f.read())
            else:
                with open(TMP_ERR, "rb") as f:
                    err_text = f.read().decode("utf-8", "replace")
        except Exception as e:
            err_text = f"driver exception while running bench: {e!r}"
        duration_ms = int((time.monotonic() - t0) * 1000)

        if report is not None:
            append_jsonl({
                "type": "report",
                "window": window,
                "scheduled_at_utc": datetime.fromtimestamp(tick_start, timezone.utc).strftime(TS_FMT),
                "captured_at_utc": now_utc_str(),
                "duration_ms": duration_ms,
                "exit_code": exit_code,
                "watch_alive": watch_alive,
                "watch_ws_bytes": watch_ws,
                "watch_private_bytes": watch_priv,
                "report": report,
            })
            ps = report["process_scan"]
            log(f"window={window} ok scan_p50={ps['p50_ms']:.3f} scan_p95={ps['p95_ms']:.3f} "
                f"snap_p50={report['system_snapshot']['p50_ms']:.3f} procs={report['process_count_last']} "
                f"bench_ws={report['bench_self_working_set_bytes']} watch_ws={watch_ws}")
        else:
            append_jsonl({
                "type": "error",
                "window": window,
                "scheduled_at_utc": datetime.fromtimestamp(tick_start, timezone.utc).strftime(TS_FMT),
                "captured_at_utc": now_utc_str(),
                "duration_ms": duration_ms,
                "exit_code": exit_code,
                "stderr": err_text or "",
                "watch_alive": watch_alive,
                "watch_ws_bytes": watch_ws,
                "watch_private_bytes": watch_priv,
            })
            log(f"window={window} FAILED exit={exit_code}")

        window += 1
        if datetime.now(timezone.utc).timestamp() >= planned_end:
            break

    # Stop the watch: graceful CTRL_C if it has a console, hard kill otherwise.
    stop_mode = "not-running"
    watch_exit = None
    if watch is not None:
        if watch.poll() is None:
            graceful = graceful_stop(watch.pid)
            if graceful:
                try:
                    watch.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    pass
            if watch.poll() is None:
                watch.kill()
                watch.wait()
                stop_mode = "forced-after-graceful-timeout" if graceful else "forced"
            else:
                stop_mode = "graceful" if graceful else "unexpected-exit"
        else:
            stop_mode = "already-exited"
        watch_exit = watch.returncode

    append_jsonl({
        "type": "done",
        "completed_at_utc": now_utc_str(),
        "windows_recorded": window,
        "watch_stop_mode": stop_mode,
        "watch_exit_code": watch_exit,
    })
    log(f"SOAK COMPLETE windows={window} watch_stop={stop_mode} watch_exit={watch_exit}")

    clear_keep_awake()
    sys.exit(0)


if __name__ == "__main__":
    main()
