#!/usr/bin/env python3
"""Compare frozen release headless_ui_perf executables without rebuilding them."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import statistics
import subprocess


def measurements(text):
    values = {}
    startup = re.search(
        r"first rows Some\(([\d.]+)\).*?first after-paint Some\(([\d.]+)\)", text
    )
    if startup is None:
        raise ValueError("benchmark did not report startup measurements")
    values["startup_first_rows_ms"] = float(startup[1])
    values["startup_first_paint_ms"] = float(startup[2])
    for query, applied, painted in re.findall(
        r'search latency "([^"]+)": applied ([\d.]+) ms, painted ([\d.]+) ms', text
    ):
        values[f"search_applied_ms:{query}"] = float(applied)
        values[f"search_painted_ms:{query}"] = float(painted)
    for name, duration in re.findall(r"^  (.*?)\s+([\d.]+) ms$", text, re.MULTILINE):
        values[f"worst_dispatch_ms:{name}"] = float(duration)
    return values


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--rounds", type=int, default=3)
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("--rounds must be positive")
    binaries = {"before": args.before.resolve(), "after": args.after.resolve()}
    hashes = {label: hashlib.sha256(path.read_bytes()).hexdigest()
              for label, path in binaries.items()}
    if hashes["before"] == hashes["after"]:
        parser.error("before and after executables are identical; rebuild and freeze each revision")
    args.output.mkdir(parents=True, exist_ok=True)
    samples = {label: [] for label in binaries}
    env = dict(os.environ, RSCLIP_REQUIRE_HEADLESS_UI="1")
    for index in range(args.rounds):
        order = ["before", "after"] if index % 2 == 0 else ["after", "before"]
        for label in order:
            path = args.output / f"{label}-{index + 1}.log"
            with path.open("w") as log:
                subprocess.run(
                    ["dbus-run-session", "--", str(binaries[label]), "--nocapture"],
                    env=env,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                    check=True,
                    timeout=180,
                )
            samples[label].append(measurements(path.read_text()))
            print(f"{label} round {index + 1} finished", flush=True)
    medians = {
        label: {
            key: statistics.median(run[key] for run in runs)
            for key in runs[0]
        }
        for label, runs in samples.items()
    }
    result = {"binaries": {k: str(v) for k, v in binaries.items()}, "sha256": hashes,
              "rounds": args.rounds, "samples_ms": samples, "medians_ms": medians}
    (args.output / "results.json").write_text(json.dumps(result, indent=2) + "\n")
    for key, before in medians["before"].items():
        after = medians["after"][key]
        print(f"{key:50s} {before:7.2f} -> {after:7.2f} ms", flush=True)


if __name__ == "__main__":
    main()
