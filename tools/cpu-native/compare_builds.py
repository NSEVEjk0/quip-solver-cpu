#!/usr/bin/env python3
"""Compare two CPU benchmark builds on matched canonical instances."""

import argparse
import concurrent.futures
import csv
import datetime
import hashlib
import json
import math
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
    parser.add_argument("--candidate-backend", choices=("discrete-sb", "zephyr-spine"))
    parser.add_argument("--candidate-spine-blocks", type=int)
    parser.add_argument("--candidate-spine-span", type=int, choices=range(1, 5))
    parser.add_argument("--candidate-spine-plateaus", action="store_true")
    parser.add_argument("--candidate-spine-perturbation", type=int)
    spine_comparison = parser.add_mutually_exclusive_group()
    spine_comparison.add_argument(
        "--compare-spine-batch",
        action="store_true",
        help="use the same spine search on both arms and batch the candidate's tables",
    )
    spine_comparison.add_argument(
        "--compare-spine-plan-cache",
        action="store_true",
        help="batch both arms and retain repeated neighborhood plans in the candidate",
    )
    spine_comparison.add_argument(
        "--compare-spine-batched-builds",
        action="store_true",
        help="use the same batched spine search on both arms while comparing two builds",
    )
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
    parser.add_argument("--candidate-beta-min", type=float)
    parser.add_argument("--candidate-resamples", type=int)
    parser.add_argument("--candidate-polish-sweeps", type=int)
    parser.add_argument("--candidate-single-every", type=int)
    parser.add_argument(
        "--serial",
        action="store_true",
        help="run both builds on the same CPU in balanced sequential order",
    )
    parser.add_argument(
        "--bracket",
        action="store_true",
        help="time each build twice per instance in balanced ABBA order on the same CPU",
    )
    parser.add_argument(
        "--perf",
        action="store_true",
        help="record Linux perf counters for each process",
    )
    args = parser.parse_args()
    spine_parity = (
        args.compare_spine_batch
        or args.compare_spine_plan_cache
        or args.compare_spine_batched_builds
    )
    if min(args.instances, args.reads, args.sweeps) < 1 or args.seed_start < 0:
        parser.error("positive budgets and a nonnegative seed are required")
    if args.reference_sweeps is not None and args.reference_sweeps < 1:
        parser.error("reference sweeps must be positive")
    if args.candidate_beta_min is not None and (
        not math.isfinite(args.candidate_beta_min) or args.candidate_beta_min <= 0.0
    ):
        parser.error("candidate initial beta must be finite and positive")
    if args.candidate_resamples is not None and args.candidate_resamples < 0:
        parser.error("candidate resampling count must be nonnegative")
    if args.candidate_polish_sweeps is not None and args.candidate_polish_sweeps < 0:
        parser.error("candidate polish sweeps must be nonnegative")
    if args.candidate_spine_blocks is not None and args.candidate_spine_blocks < 1:
        parser.error("candidate spine block count must be positive")
    if (
        args.candidate_spine_span is not None
        or args.candidate_spine_plateaus
        or args.candidate_spine_perturbation is not None
        or args.candidate_backend == "zephyr-spine"
    ) and args.candidate_spine_blocks is None:
        parser.error("candidate spine options and backend require a block count")
    if args.candidate_spine_perturbation is not None and not (
        0 <= args.candidate_spine_perturbation <= 65_535
    ):
        parser.error("candidate perturbation must be in 0..=65535 milli units")
    if args.compare_energy and args.mode != "pipeline":
        parser.error("--compare-energy requires the pipeline's original-energy verification")
    if spine_parity and (
        args.mode != "pipeline"
        or args.compare_energy
        or args.candidate_spine_blocks is None
        or args.candidate_spine_plateaus
        or args.candidate_spine_perturbation is not None
    ):
        parser.error("spine parity comparisons require the same strictly downhill search in pipeline mode")
    if args.bracket and not args.serial:
        parser.error("--bracket requires --serial")
    if args.candidate_single_every is not None and args.candidate_single_every < 0:
        parser.error("Metropolis cadence must be nonnegative")
    if not args.compare_energy and any(
        option is not None
        for option in (
            args.reference_sweeps,
            args.candidate_beta_min,
            args.candidate_resamples,
            args.candidate_polish_sweeps,
            args.candidate_single_every,
            None if spine_parity else args.candidate_backend,
            None if spine_parity else args.candidate_spine_blocks,
        )
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
            candidate_beta_min=args.candidate_beta_min,
            candidate_resamples=args.candidate_resamples,
            candidate_polish_sweeps=args.candidate_polish_sweeps,
            candidate_single_every=args.candidate_single_every,
        )
    if args.candidate_backend:
        metadata["candidate_backend"] = args.candidate_backend
    if args.candidate_spine_blocks:
        metadata["candidate_spine_blocks"] = args.candidate_spine_blocks
        metadata["candidate_spine_span"] = args.candidate_spine_span or 1
        metadata["candidate_spine_plateaus"] = args.candidate_spine_plateaus
        metadata["candidate_spine_perturbation_milli"] = args.candidate_spine_perturbation or 0
    if args.compare_spine_batch:
        metadata["comparison"] = "exact-spine-batch"
    if args.compare_spine_plan_cache:
        metadata["comparison"] = "exact-spine-plan-cache"
    if args.compare_spine_batched_builds:
        metadata["comparison"] = "exact-spine-batched-builds"
    if args.bracket:
        metadata["repeats_per_build"] = 2
    if args.perf:
        metadata["perf_events"] = PERF_EVENTS
        metadata["perf_task_clock_unit"] = "ms"
    write_json(output / "metadata.json", metadata)
    rows, ratios, improvements, spine_ratios = [], [], [], []
    exact_pairs = 0

    def run(role, seed, cpu, repetition=None):
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
        use_spine_options = spine_parity or role == "candidate"
        batched = spine_parity and (not args.compare_spine_batch or role == "candidate")
        cached = args.compare_spine_plan_cache and role == "candidate"
        if use_spine_options and args.candidate_backend:
            command.extend(("--backend", args.candidate_backend))
        if use_spine_options and args.candidate_spine_blocks:
            command.extend(("--spine-blocks", str(args.candidate_spine_blocks)))
            if args.candidate_spine_span:
                command.extend(("--spine-span", str(args.candidate_spine_span)))
            if args.candidate_spine_plateaus:
                command.append("--spine-plateaus")
            if args.candidate_spine_perturbation:
                command.extend(("--spine-perturbation", str(args.candidate_spine_perturbation)))
            if batched:
                command.append("--spine-batch")
            if cached:
                command.append("--spine-cache-plans")
        if role == "candidate" and args.candidate_polish_sweeps is not None:
            command.extend(("--polish-sweeps", str(args.candidate_polish_sweeps)))
        if role == "candidate" and args.candidate_beta_min is not None:
            command.extend(("--beta-min", repr(args.candidate_beta_min)))
        if role == "candidate" and args.candidate_resamples is not None:
            command.extend(("--resamples", str(args.candidate_resamples)))
        if role == "candidate" and args.candidate_single_every is not None:
            command.extend(("--single-every", str(args.candidate_single_every)))
        stem = f"{seed}-{role}" if repetition is None else f"{seed}-{role}-{repetition}"
        if args.perf:
            perf_path = output / f"{stem}.perf.csv"
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
        if row["seed"] != seed or row["sweeps"] != sweeps or row["reads"] != args.reads:
            raise RuntimeError(f"reported solve budget differs for {role} at seed {seed}")
        if row["mode"].lower() != args.mode or row["preset"].lower() != args.preset:
            raise RuntimeError(f"reported solve preset differs for {role} at seed {seed}")
        if use_spine_options:
            if args.candidate_backend and row.get("backend") != args.candidate_backend:
                raise RuntimeError(f"reported candidate backend differs at seed {seed}")
            if args.candidate_spine_blocks:
                metrics = row.get("metrics") or {}
                if (
                    metrics.get("spine_blocks") != args.candidate_spine_blocks
                    or metrics.get("spine_span") != (args.candidate_spine_span or 1)
                    or metrics.get("spine_plateaus") != args.candidate_spine_plateaus
                    or metrics.get("spine_perturbation_milli") != (args.candidate_spine_perturbation or 0)
                    or metrics.get("spine_batch") != batched
                    or metrics.get("spine_cache_plans", False) != cached
                ):
                    raise RuntimeError(f"reported spine budget differs at seed {seed}")
        if role == "candidate":
            if args.candidate_beta_min is not None and row.get("beta_min") != args.candidate_beta_min:
                raise RuntimeError(f"reported candidate initial beta differs at seed {seed}")
            if args.candidate_resamples is not None and row.get("resamples") != args.candidate_resamples:
                raise RuntimeError(f"reported candidate resampling count differs at seed {seed}")
            if args.candidate_polish_sweeps is not None and row.get("polish_sweeps") != args.candidate_polish_sweeps:
                raise RuntimeError(f"reported candidate polishing budget differs at seed {seed}")
            if args.candidate_single_every is not None and row.get("single_every") != args.candidate_single_every:
                raise RuntimeError(f"reported candidate Metropolis cadence differs at seed {seed}")
        row.update(role=role, label=args.label, cpu=cpu, command=command)
        if args.perf:
            row["perf"] = read_perf(perf_path)
        if result.stderr:
            (output / f"{stem}.log").write_text(result.stderr)
        return row

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        for number, seed in enumerate(
            range(args.seed_start, args.seed_start + args.instances)
        ):
            left, right = args.cpus if number % 2 == 0 else reversed(args.cpus)
            if args.serial:
                roles = ("reference", "candidate")
                order = roles if number % 4 < 2 else tuple(reversed(roles))
                if args.bracket:
                    order = (*order, *reversed(order))
                by_role = {role: [] for role in roles}
                for position, role in enumerate(order, start=1):
                    repeat = position if args.bracket else None
                    row = run(role, seed, left, repeat)
                    row["serial_order"] = position
                    by_role[role].append(row)
                pair = []
                for role in roles:
                    first = by_role[role][0]
                    if not args.bracket:
                        pair.append(first)
                        continue
                    second = by_role[role][1]
                    if first["trajectory_digest"] != second["trajectory_digest"]:
                        raise RuntimeError(f"repeat outputs differ for {role} at seed {seed}")
                    first = {**first, "runs": [first, second]}
                    first["seconds"] = statistics.mean(row["seconds"] for row in first["runs"])
                    if "spine_seconds" in (first.get("metrics") or {}):
                        first["metrics"] = dict(first["metrics"])
                        first["metrics"]["spine_seconds"] = statistics.mean(
                            row["metrics"]["spine_seconds"] for row in first["runs"]
                        )
                    if args.perf:
                        first["perf"] = {
                            event: statistics.mean(row["perf"][event] for row in first["runs"])
                            for event in PERF_EVENTS
                        }
                    pair.append(first)
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
            if any(row["best_energy_milli"] != min(row["energies_milli"]) for row in pair):
                raise RuntimeError(f"best energy does not match the candidate list at seed {seed}")
            if args.candidate_spine_blocks:
                before = pair[1]["metrics"]["spine_initial_energies_milli"]
                after = pair[1]["energies_milli"]
                if (
                    len(before) != len(after)
                    or any(new > old for old, new in zip(before, after))
                    or sum(before) - sum(after) != pair[1]["metrics"]["spine_gain_milli"]
                ):
                    raise RuntimeError(f"conditional spine energy audit failed at seed {seed}")
                if (
                    not spine_parity
                    and args.candidate_backend is None
                    and (args.reference_sweeps or args.sweeps) == args.sweeps
                    and not args.selective_resampling
                    and args.candidate_single_every is None
                    and args.candidate_polish_sweeps is None
                    and args.candidate_beta_min is None
                    and args.candidate_resamples is None
                    and args.cache_slots == args.reference_cache_slots
                    and binaries["candidate"] == binaries["reference"]
                    and before != pair[0]["energies_milli"]
                ):
                    raise RuntimeError(f"conditional spine starts differ at seed {seed}")
            if spine_parity:
                reference, candidate = (row["metrics"] for row in pair)
                if reference["spine_initial_energies_milli"] != candidate["spine_initial_energies_milli"]:
                    raise RuntimeError(f"batched spine starts differ at seed {seed}")
                for key in ("spine_gain_milli", "spine_accepted", "spine_max_width", "spine_min_free", "spine_max_free"):
                    if reference[key] != candidate[key]:
                        raise RuntimeError(f"batched spine metric {key} differs at seed {seed}")
                spine_ratios.append(reference["spine_seconds"] / candidate["spine_seconds"])
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
    if spine_parity:
        summary["spine_stage_median_speedup"] = statistics.median(spine_ratios)
        summary["spine_stage_bootstrap95"] = bootstrap_median(spine_ratios)
        summary["spine_stage_speedup_range"] = [min(spine_ratios), max(spine_ratios)]
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
