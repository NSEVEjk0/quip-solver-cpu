#!/usr/bin/env python3
"""Compare two CPU benchmark builds on matched canonical instances."""

import argparse
import concurrent.futures
import csv
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess

from run_study import REPO, bootstrap_median, capture, write_json

PERF_EVENTS = (
    "task-clock",
    "cycles",
    "instructions",
    "branches",
    "branch-misses",
    "cache-misses",
)


def read_perf(path):
    counters = {}
    with path.open(newline="") as stream:
        for fields in csv.reader(stream):
            if len(fields) < 5:
                continue
            value, _, event, _, measured = fields[:5]
            if event not in PERF_EVENTS:
                continue
            try:
                count = float(value) if event == "task-clock" else int(value)
                measured_percent = float(measured)
            except ValueError as error:
                raise RuntimeError(f"perf did not report {event} in {path}") from error
            if measured_percent < 99.0:
                raise RuntimeError(
                    f"perf measured {event} for only {measured_percent}% of the run in {path}"
                )
            counters[event] = count
    if set(counters) != set(PERF_EVENTS):
        raise RuntimeError(f"Missing performance counters in {path}")
    return counters


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference-binary", type=Path, required=True)
    parser.add_argument("--candidate-binary", type=Path, required=True)
    parser.add_argument("--label", required=True)
    parser.add_argument("--cpus", nargs=2, type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--mode", choices=("kernel", "pipeline"), default="kernel")
    parser.add_argument("--preset", choices=("core", "archive"), default="core")
    parser.add_argument("--seed-start", type=int, default=1031)
    parser.add_argument("--instances", type=int, default=8)
    parser.add_argument("--reads", type=int, default=16)
    parser.add_argument("--sweeps", type=int, default=2048)
    parser.add_argument("--selective-resampling", action="store_true")
    parser.add_argument("--cache-slots", type=int, default=8192)
    parser.add_argument("--reference-cache-slots", type=int, default=8192)
    parser.add_argument(
        "--compare-energy",
        action="store_true",
        help="compare original candidate energies when the solver may return different spins",
    )
    parser.add_argument("--reference-sweeps", type=int)
    parser.add_argument("--candidate-polish-sweeps", type=int)
    parser.add_argument("--candidate-single-every", type=int)
    parser.add_argument(
        "--serial",
        action="store_true",
        help="run both builds on the same CPU in balanced sequential order",
    )
    parser.add_argument(
        "--perf",
        action="store_true",
        help="record Linux perf counters for each process",
    )
    args = parser.parse_args()
    if min(args.instances, args.reads, args.sweeps) < 1 or args.seed_start < 0:
        parser.error("positive budgets and a nonnegative seed are required")
    if args.reference_sweeps is not None and args.reference_sweeps < 1:
        parser.error("reference sweeps must be positive")
    if args.candidate_polish_sweeps is not None and args.candidate_polish_sweeps < 0:
        parser.error("candidate polish sweeps must be nonnegative")
    if args.compare_energy and args.mode != "pipeline":
        parser.error("--compare-energy requires the pipeline's original-energy verification")
    if args.candidate_single_every is not None and args.candidate_single_every < 0:
        parser.error("Metropolis cadence must be nonnegative")
    if not args.compare_energy and any(
        option is not None
        for option in (args.reference_sweeps, args.candidate_polish_sweeps, args.candidate_single_every)
    ):
        parser.error("different solve budgets require --compare-energy")
    if len(set(args.cpus)) != 2 or min(args.cpus) < 0:
        parser.error("select two different available CPU IDs")
    if hasattr(os, "sched_getaffinity") and not set(args.cpus).issubset(
        os.sched_getaffinity(0)
    ):
        parser.error("CPU IDs are outside this process's affinity")
    if not all(
        path.is_file() for path in (args.reference_binary, args.candidate_binary)
    ):
        parser.error("both benchmark executables must exist")
    if args.perf and shutil.which("perf") is None:
        parser.error("--perf requires Linux perf")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if (output / "results.json").exists():
        parser.error("select a new output directory")
    binaries = {
        "reference": args.reference_binary.resolve(),
        "candidate": args.candidate_binary.resolve(),
    }
    metadata = {
        "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "label": args.label,
        "source_revision": capture(["git", "rev-parse", "HEAD"]),
        "source_dirty": bool(capture(["git", "status", "--porcelain"])),
        "platform": platform.platform(),
        "rustc": capture(["rustc", "-vV"]),
        "cpus": args.cpus,
        "concurrent_processes": 1 if args.serial else 2,
        "mode": args.mode,
        "preset": args.preset,
        "reads": args.reads,
        "sweeps": args.sweeps,
        "seed_start": args.seed_start,
        "instances": args.instances,
        "candidate_selective_resampling": args.selective_resampling,
        "candidate_cache_slots": args.cache_slots,
        "reference_cache_slots": args.reference_cache_slots,
        "binary_sha256": {
            role: hashlib.sha256(path.read_bytes()).hexdigest()
            for role, path in binaries.items()
        },
        "input_sha256": {
            name: hashlib.sha256((REPO / name).read_bytes()).hexdigest()
            for name in (
                "tools/cpu-funnel/inputs/current-topology.spec.json",
                "tools/cpu-funnel/inputs/current-family.params",
            )
        },
    }
    if args.compare_energy:
        metadata.update(
            comparison="energy",
            reference_sweeps=args.reference_sweeps or args.sweeps,
            candidate_polish_sweeps=args.candidate_polish_sweeps,
            candidate_single_every=args.candidate_single_every,
        )
    if args.perf:
        metadata["perf_events"] = PERF_EVENTS
        metadata["perf_task_clock_unit"] = "ms"
    write_json(output / "metadata.json", metadata)
    rows, ratios, improvements = [], [], []
    exact_pairs = 0

    def run(role, seed, cpu):
        slots = args.reference_cache_slots if role == "reference" else args.cache_slots
        sweeps = (args.reference_sweeps or args.sweeps) if role == "reference" else args.sweeps
        command = [
            str(binaries[role]),
            "--mode",
            args.mode,
            "--preset",
            args.preset,
            "--seed",
            str(seed),
            "--reads",
            str(args.reads),
            "--sweeps",
            str(sweeps),
            "--cache-slots",
            str(slots),
        ]
        if role == "candidate" and args.selective_resampling:
            command.append("--selective-resampling")
        if role == "candidate" and args.candidate_polish_sweeps is not None:
            command.extend(("--polish-sweeps", str(args.candidate_polish_sweeps)))
        if role == "candidate" and args.candidate_single_every is not None:
            command.extend(("--single-every", str(args.candidate_single_every)))
        if args.perf:
            perf_path = output / f"{seed}-{role}.perf.csv"
            command = [
                "perf",
                "stat",
                "-x",
                ",",
                "-e",
                ",".join(PERF_EVENTS),
                "--output",
                str(perf_path),
                "--",
                *command,
            ]
        command = ["taskset", "-c", str(cpu), "nice", "-n", "10", *command]
        result = subprocess.run(
            command,
            cwd=REPO,
            text=True,
            capture_output=True,
            env={**os.environ, "LC_ALL": "C"} if args.perf else None,
            check=True,
        )
        row = json.loads(result.stdout)
        row.update(role=role, label=args.label, cpu=cpu, command=command)
        if args.perf:
            row["perf"] = read_perf(perf_path)
        if result.stderr:
            (output / f"{seed}-{role}.log").write_text(result.stderr)
        return row

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        for number, seed in enumerate(
            range(args.seed_start, args.seed_start + args.instances)
        ):
            left, right = args.cpus if number % 2 == 0 else reversed(args.cpus)
            if args.serial:
                roles = ("reference", "candidate")
                order = roles if number % 4 < 2 else reversed(roles)
                by_role = {}
                for position, role in enumerate(order, start=1):
                    row = run(role, seed, left)
                    row["serial_order"] = position
                    by_role[role] = row
                pair = [by_role[role] for role in roles]
            else:
                futures = [
                    pool.submit(run, "reference", seed, left),
                    pool.submit(run, "candidate", seed, right),
                ]
                pair = [future.result() for future in futures]
            ratio = pair[0]["seconds"] / pair[1]["seconds"]
            same = pair[0]["trajectory_digest"] == pair[1]["trajectory_digest"]
            if pair[0]["nonce"] != pair[1]["nonce"]:
                raise RuntimeError(f"different canonical instances at seed {seed}")
            if any(len(row["energies_milli"]) != args.reads for row in pair):
                raise RuntimeError(f"candidate count differs from the requested reads at seed {seed}")
            energy_change = {}
            if args.compare_energy:
                ref, candidate = [sorted(row["energies_milli"]) for row in pair]
                energy_change = {
                    "best_gain_milli": ref[0] - candidate[0],
                    "median_gain_milli": statistics.median(ref) - statistics.median(candidate),
                    "worst_gain_milli": ref[-1] - candidate[-1],
                }
                improvements.append(energy_change)
            for row in pair:
                row.update(paired_speedup=ratio, exact_output=same, **energy_change)
                write_json(output / f"{seed}-{row['role']}.json", row)
            rows.extend(pair)
            write_json(output / "results.json", rows)
            if not same and not args.compare_energy:
                raise RuntimeError(
                    f"candidate output differs at seed {seed}; records retained"
                )
            ratios.append(ratio)
            exact_pairs += same
            print(
                json.dumps(
                    {
                        "label": args.label,
                        "seed": seed,
                        "speedup": ratio,
                        "reference_seconds": pair[0]["seconds"],
                        "candidate_seconds": pair[1]["seconds"],
                        "exact_output": same,
                        **energy_change,
                    }
                ),
                flush=True,
            )
    summary = {
        "label": args.label,
        "pairs": len(ratios),
        "median_speedup": statistics.median(ratios),
        "bootstrap95_median": bootstrap_median(ratios),
        "mean_reference_seconds": statistics.mean(
            row["seconds"] for row in rows if row["role"] == "reference"
        ),
        "mean_candidate_seconds": statistics.mean(
            row["seconds"] for row in rows if row["role"] == "candidate"
        ),
        "exact_output_pairs": exact_pairs,
        "mean_best_original_energy": statistics.mean(
            row["best_energy_milli"] / 1000
            for row in rows
            if row["role"] == "candidate"
        ),
    }
    if args.compare_energy:
        summary["energy"] = {}
        for key in ("best_gain_milli", "median_gain_milli", "worst_gain_milli"):
            values = [row[key] for row in improvements]
            summary["energy"][key] = {
                "median": statistics.median(values),
                "mean": statistics.mean(values),
                "range": [min(values), max(values)],
                "improved": sum(value > 0 for value in values),
                "tied": values.count(0),
                "regressed": sum(value < 0 for value in values),
            }
    if args.perf:
        counter_ratios = {}
        for event in PERF_EVENTS:
            candidates = [row["perf"][event] for row in rows[1::2]]
            counter_ratios[event] = None
            if all(count > 0 for count in candidates):
                counter_ratios[event] = statistics.median(
                    reference["perf"][event] / candidate
                    for reference, candidate in zip(rows[::2], candidates)
                )
        summary["perf_median_reference_over_candidate"] = counter_ratios
    write_json(output / "summary.json", summary)
    metadata["completed_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    write_json(output / "metadata.json", metadata)
    print(json.dumps(summary), flush=True)


if __name__ == "__main__":
    main()
