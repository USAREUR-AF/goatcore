#!/usr/bin/env python3
"""Sample one process's CPU time and resident memory once per interval.

Runs on the relay machine:  sampler.py <pid> <out.csv> [interval seconds]

Each line: unix time in ms, process user+system CPU in clock ticks, resident
set size in KiB, high-water resident set size in KiB, and the host's busy and
total CPU ticks from /proc/stat. Stops when the process exits or on SIGTERM.
"""
import os
import signal
import sys
import time

pid = int(sys.argv[1])
out = sys.argv[2]
interval = float(sys.argv[3]) if len(sys.argv) > 3 else 1.0
running = True


def stop(_sig, _frm):
    global running
    running = False


signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)


def proc_cpu():
    with open(f"/proc/{pid}/stat") as f:
        rest = f.read().rsplit(")", 1)[1].split()
    # fields after the command: state is rest[0]; utime is field 14 -> rest[11]
    return int(rest[11]) + int(rest[12])


def proc_mem():
    rss = hwm = 0
    with open(f"/proc/{pid}/status") as f:
        for line in f:
            if line.startswith("VmRSS:"):
                rss = int(line.split()[1])
            elif line.startswith("VmHWM:"):
                hwm = int(line.split()[1])
    return rss, hwm


def host_cpu():
    with open("/proc/stat") as f:
        vals = [int(v) for v in f.readline().split()[1:]]
    idle = vals[3] + vals[4]
    total = sum(vals[:8])
    return total - idle, total


with open(out, "w", buffering=1) as f:
    f.write(f"# clk_tck={os.sysconf('SC_CLK_TCK')} ncpu={os.cpu_count()} pid={pid}\n")
    f.write("t_ms,proc_ticks,rss_kib,hwm_kib,host_busy,host_total\n")
    while running:
        try:
            c = proc_cpu()
            rss, hwm = proc_mem()
        except (FileNotFoundError, ProcessLookupError):
            break
        busy, total = host_cpu()
        f.write(f"{int(time.time() * 1000)},{c},{rss},{hwm},{busy},{total}\n")
        time.sleep(interval)
