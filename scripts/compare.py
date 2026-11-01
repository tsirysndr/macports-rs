#!/usr/bin/env python3
"""Time port-rs against the installed MacPorts `port` on read-only queries.

Both read the same ports tree and registry. Each command runs once to warm
caches, then 20 times per binary, interleaved; the table shows the best run.

    python3 scripts/compare.py target/release/port
"""
import os
import subprocess
import sys
import time

UPSTREAM = "/opt/local/bin/port"
TREE = "/opt/local/var/macports/sources/rsync.macports.org/macports/release/tarballs/ports"
COMMANDS = [
    ["version"], ["info", "curl"], ["search", "--name", "curl"], ["list", "curl"],
    ["deps", "curl"], ["variants", "curl"], ["rdeps", "curl"], ["echo", "all"],
    ["installed"], ["outdated"],
]
RUNS = 20


def best(binary, args, env):
    subprocess.run([binary, *args], env=env, capture_output=True)
    times = []
    for _ in range(RUNS):
        start = time.perf_counter()
        subprocess.run([binary, *args], env=env, capture_output=True)
        times.append(time.perf_counter() - start)
    return min(times) * 1000


def main():
    ours = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "target/release/port")
    env = dict(os.environ, PORT_RS_INDEX=f"{TREE}/PortIndex")
    rows = [("Command", "MacPorts", "port-rs", "Speedup")]
    for args in COMMANDS:
        upstream, rust = best(UPSTREAM, args, env), best(ours, args, env)
        rows.append((f"`port {' '.join(args)}`", f"{upstream:.0f} ms", f"{rust:.0f} ms",
                     f"{upstream / rust:.1f}×"))
    widths = [max(len(row[i]) for row in rows) for i in range(4)]

    def line(row):
        cells = [row[0].ljust(widths[0])]
        cells += [cell.rjust(width) for cell, width in zip(row[1:], widths[1:])]
        return "| " + " | ".join(cells) + " |"

    print(line(rows[0]))
    print("| " + " | ".join(["-" * widths[0]] + ["-" * (w - 1) + ":" for w in widths[1:]]) + " |")
    for row in rows[1:]:
        print(line(row))


if __name__ == "__main__":
    main()
