# pressure_consumer.py - bounded stepped memory consumer for the P3 rehearsal.
#
# Allocates and TOUCHES (faults in) memory in steps, stopping at either the
# target or a hard commit-margin guard, printing progress for the driver.
# Writes its own PID so the driver can stop it by explicit PID (never by
# image name - the recorded multi-session gotcha). All allocations are
# released on exit.

import os
import sys
import time

CHUNK_BYTES = 3 * (1024**3)      # 3 GiB per step
MIN_FREE_VIRTUAL_KB = 3_000_000  # hard guard: stop with ~2.9 GiB commit margin
PID_FILE = sys.argv[1]
STOP_FILE = PID_FILE + ".stop"
MAX_STEPS = 6                    # 6 x 3 GiB = 18 GiB absolute cap

with open(PID_FILE, "w") as f:
    f.write(str(os.getpid()))

blocks = []
step = 0
while step < MAX_STEPS:
    if os.path.exists(STOP_FILE):
        print(f"stop file seen after {step} step(s)", flush=True)
        break
    # Commit-margin guard: ask the OS before each step.
    free_kb = int(
        os.popen(
            "powershell -NoProfile -Command \"(Get-CimInstance Win32_OperatingSystem).FreeVirtualMemory\""
        )
        .read()
        .strip()
    )
    if free_kb < MIN_FREE_VIRTUAL_KB + CHUNK_BYTES // 1024:
        print(f"commit guard at step {step}: free virtual {free_kb} KB", flush=True)
        break
    b = bytearray(CHUNK_BYTES)
    for i in range(0, len(b), 4096):
        b[i] = 1  # fault every page in
    blocks.append(b)
    step += 1
    print(f"step {step}: +3 GiB touched (total {step * 3} GiB; free virtual {free_kb} KB)", flush=True)
    time.sleep(25)  # settle window so the 1 Hz watch records the state

if os.path.exists(STOP_FILE):
    os.remove(STOP_FILE)
print("holding allocations until killed or 90 s elapse...", flush=True)
time.sleep(90)
print("releasing", flush=True)
blocks.clear()
print("released", flush=True)
time.sleep(20)
