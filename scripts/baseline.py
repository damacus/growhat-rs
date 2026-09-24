#!/usr/bin/env python3
"""Read-only Linux process baseline; send over SSH stdin to avoid installing it."""

import argparse
import datetime
import json
import os
from pathlib import Path
import statistics
import subprocess
import time


def process(pid):
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    io = {}
    try:
        io = {key: int(value) for key, value in
              (line.split(":", 1) for line in Path(f"/proc/{pid}/io").read_text().splitlines())}
    except PermissionError:
        pass
    return {"ticks": int(fields[11]) + int(fields[12]),
            "rss_bytes": int(fields[21]) * os.sysconf("SC_PAGE_SIZE"),
            "start_ticks": int(fields[19]), "io": io}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("units", nargs="+")
    parser.add_argument("--seconds", type=int, default=30)
    parser.add_argument("--state-file", type=Path)
    args = parser.parse_args()
    if not 1 <= args.seconds <= 60:
        parser.error("--seconds must be between 1 and 60")
    pids = {unit: int(subprocess.check_output(
        ["systemctl", "show", unit, "-p", "MainPID", "--value"], text=True).strip())
        for unit in args.units}
    if any(pid <= 0 for pid in pids.values()):
        raise RuntimeError("Every named unit must be running")
    initial = {unit: process(pid) for unit, pid in pids.items()}
    started_at = datetime.datetime.now(datetime.timezone.utc).isoformat()
    started = time.monotonic()
    rows = []
    while True:
        elapsed = time.monotonic() - started
        row = {"elapsed_s": elapsed, "processes": {u: process(p) for u, p in pids.items()}}
        if args.state_file:
            try:
                row["state"] = json.loads(args.state_file.read_text())
                row["state_age_s"] = time.time() - row["state"]["updated"]
            except (OSError, ValueError, KeyError) as error:
                row["state_error"] = str(error)
        rows.append(row)
        if elapsed >= args.seconds:
            break
        time.sleep(min(2, args.seconds - elapsed))
    summaries = {}
    for unit in args.units:
        first = initial[unit]
        last = rows[-1]["processes"][unit]
        if any(row["processes"][unit]["start_ticks"] != first["start_ticks"] for row in rows):
            raise RuntimeError(f"{unit} restarted during measurement")
        memories = [row["processes"][unit]["rss_bytes"] for row in rows]
        summaries[unit] = {
            "pid": pids[unit],
            "cpu_percent_one_core": 100 * (last["ticks"] - first["ticks"]) / os.sysconf("SC_CLK_TCK") / elapsed,
            "rss_mean_bytes": statistics.mean(memories), "rss_min_bytes": min(memories), "rss_max_bytes": max(memories),
            "io_delta": {key: value - first["io"][key] for key, value in last["io"].items() if key in first["io"]},
        }
    print(json.dumps({"started_at": started_at, "elapsed_s": elapsed, "clock_ticks_per_s": os.sysconf("SC_CLK_TCK"),
                      "summary": summaries, "samples": rows}, indent=2))


if __name__ == "__main__":
    main()
