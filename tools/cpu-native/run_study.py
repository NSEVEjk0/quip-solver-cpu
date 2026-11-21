#!/usr/bin/env python3
"""Run paired, offline CPU studies on canonical Quip instances (Python 3.9+)."""

import argparse
import concurrent.futures
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import shutil
import statistics
import subprocess

REPO = Path(__file__).resolve().parents[2]
DEFAULTS = {
    "ablation": (1001, 8, 2048),
    "temperature": (1011, 6, 2048),
    "confirmation": (1021, 8, 8192),
    "motif": (1021, 8, 8192),
}


def write_json(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def capture(command):
    result = subprocess.run(command, cwd=REPO, capture_output=True, text=True)
    return result.stdout.strip() if result.returncode == 0 else None


def bootstrap_median(values):
    """Percentile interval for the median of paired ratios, not a hardware CI."""
    rng = random.Random(20261121)
    medians = sorted(
        statistics.median(rng.choices(values, k=len(values))) for _ in range(10000)
    )
    return [medians[250], medians[9750]]


def summarize(suite, rows):
    summary = {"suite": suite, "instances": len({r["seed"] for r in rows})}
    if suite == "ablation":
        summary["variants"] = {
            variant: {
                "pairs": len(values),
                "median_speedup": statistics.median(values),
                "bootstrap95_median": bootstrap_median(values),
            }
            for variant in ("cache", "copy", "union", "fusion", "core")
            for values in [
                [
                    r["speedup"]
                    for r in rows
                    if r["paired_variant"] == variant and r["preset"] != "Baseline"
                ]
            ]
        }
    else:
        labels = sorted({r["label"] for r in rows})
        summary["settings"] = {
            label: {
                "mean_seconds": statistics.mean(
                    r["seconds"] for r in rows if r["label"] == label
                ),
                "mean_best_energy": statistics.mean(
                    r["best_energy_milli"] / 1000 for r in rows if r["label"] == label
                ),
                "best_energy": min(
                    r["best_energy_milli"] / 1000 for r in rows if r["label"] == label
                ),
            }
            for label in labels
        }
        field = {"confirmation": "execution_speedup", "motif": "motif_speedup"}.get(
            suite
        )
        if field:
            values = [
                r[field] for r in rows if r["label"] in ("native-execution", "native")
            ]
            summary["median_speedup"] = statistics.median(values)
            summary["bootstrap95_median"] = bootstrap_median(values)
            summary["exact_output_pairs"] = len(values)
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", choices=DEFAULTS, required=True)
    parser.add_argument(
        "--cpus", nargs=2, type=int, required=True, metavar=("LEFT", "RIGHT")
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--binary", type=Path, default=REPO / "target/release/examples/cpu_native_bench"
    )
    parser.add_argument("--seed-start", type=int)
    parser.add_argument("--instances", type=int)
    parser.add_argument("--reads", type=int, default=16)
    parser.add_argument("--sweeps", type=int)
    parser.add_argument(
        "--betas", nargs="+", type=float, default=[8, 10, 12, 16, 18, 24]
    )
    parser.add_argument("--polish-beta", type=float, default=20)
    args = parser.parse_args()
    seed_start, instances, sweeps = DEFAULTS[args.suite]
    seed_start = args.seed_start if args.seed_start is not None else seed_start
    instances = args.instances if args.instances is not None else instances
    sweeps = args.sweeps if args.sweeps is not None else sweeps
    if seed_start < 0 or min(instances, args.reads, sweeps) < 1:
        parser.error(
            "seed must be nonnegative; instances, reads, and sweeps must be positive"
        )
    if len(set(args.cpus)) != 2 or min(args.cpus) < 0:
        parser.error("choose two distinct available CPU IDs")
    if hasattr(os, "sched_getaffinity") and not set(args.cpus).issubset(
        os.sched_getaffinity(0)
    ):
        parser.error("requested CPU IDs are outside this process's affinity")
    if any(not shutil.which(name) for name in ("taskset", "nice")):
        parser.error("Linux taskset and nice are required for controlled CPU placement")
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error("benchmark binary is missing; build cpu_native_bench first")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if (output / "results.json").exists():
        parser.error("output already contains results; select a new directory")
    metadata = {
        "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "suite": args.suite,
        "source_revision": capture(["git", "rev-parse", "HEAD"]),
        "source_dirty": bool(capture(["git", "status", "--porcelain"])),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "rustc": capture(["rustc", "-vV"]),
        "platform": platform.platform(),
        "cpu_model": next(
            (
                line.split(":", 1)[1].strip()
                for line in Path("/proc/cpuinfo").read_text().splitlines()
                if line.startswith("model name")
            ),
            "unknown",
        ),
        "cpu_count": os.cpu_count(),
        "cpus": args.cpus,
        "concurrent_processes": 2,
        "seed_start": seed_start,
        "instances": instances,
        "reads": args.reads,
        "sweeps": sweeps,
        "polish_beta": args.polish_beta,
        "inputs": {
            name: hashlib.sha256((REPO / name).read_bytes()).hexdigest()
            for name in (
                "tools/cpu-funnel/inputs/current-topology.spec.json",
                "tools/cpu-funnel/inputs/current-family.params",
            )
        },
        "confirmation_archive_scoring": "full-edge (motif conversion disabled)",
        "timing_scope": "harness-reported elapsed time; startup and instance generation excluded",
    }
    write_json(output / "metadata.json", metadata)
    rows = []

    def run(preset, seed, cpu, label, mode="pipeline", beta=12, scalar_archive=False):
        command = [
            "taskset",
            "-c",
            str(cpu),
            "nice",
            "-n",
            "10",
            str(binary),
            "--mode",
            mode,
            "--preset",
            preset,
            "--seed",
            str(seed),
            "--reads",
            str(args.reads),
            "--sweeps",
            str(sweeps),
            "--beta-max",
            str(beta),
            "--polish-beta",
            str(args.polish_beta),
        ]
        if scalar_archive:
            command.append("--disable-motif-score")
        result = subprocess.run(
            command, cwd=REPO, capture_output=True, text=True, check=True
        )
        row = json.loads(result.stdout)
        row.update(cpu=cpu, label=label, command=command)
        write_json(output / f"{seed}-{label}.json", row)
        if result.stderr:
            (output / f"{seed}-{label}.log").write_text(result.stderr, encoding="utf-8")
        return row

    def pair(pool, a, b, field):
        futures = [pool.submit(run, **a), pool.submit(run, **b)]
        values = [future.result() for future in futures]
        if values[0]["trajectory_digest"] != values[1]["trajectory_digest"]:
            raise RuntimeError(
                f"candidate output mismatch: {a['label']} / {b['label']}"
            )
        ratio = values[0]["seconds"] / values[1]["seconds"]
        for row in values:
            row[field] = ratio
        return values

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        for number, seed in enumerate(range(seed_start, seed_start + instances)):
            left, right = args.cpus if number % 2 == 0 else reversed(args.cpus)
            base = {"seed": seed, "cpu": left}
            native = {"seed": seed, "cpu": right}
            if args.suite == "ablation":
                for variant in ("cache", "copy", "union", "fusion", "core"):
                    values = pair(
                        pool,
                        dict(
                            base,
                            preset="baseline",
                            label=f"baseline-{variant}",
                            mode="kernel",
                        ),
                        dict(native, preset=variant, label=variant, mode="kernel"),
                        "speedup",
                    )
                    for row in values:
                        row["paired_variant"] = variant
                    rows.extend(values)
                    write_json(output / "results.json", rows)
                    print(
                        json.dumps(
                            {
                                "seed": seed,
                                "variant": variant,
                                "speedup": values[1]["speedup"],
                                "exact_output": True,
                            }
                        ),
                        flush=True,
                    )
            elif args.suite == "temperature":
                for start in range(0, len(args.betas), 2):
                    futures = [
                        pool.submit(
                            run,
                            "core",
                            seed,
                            [left, right][i],
                            f"beta{beta:g}",
                            beta=beta,
                        )
                        for i, beta in enumerate(args.betas[start : start + 2])
                    ]
                    rows.extend(future.result() for future in futures)
            elif args.suite == "confirmation":
                values = pair(
                    pool,
                    dict(base, preset="baseline", label="reference"),
                    dict(native, preset="core", label="native-execution"),
                    "execution_speedup",
                )
                rows.extend(values)
                futures = [
                    pool.submit(
                        run,
                        "archive",
                        seed,
                        right,
                        "native-archive",
                        scalar_archive=True,
                    ),
                    pool.submit(
                        run,
                        "archive",
                        seed,
                        left,
                        "native-beta10",
                        beta=10,
                        scalar_archive=True,
                    ),
                ]
                rest = [future.result() for future in futures]
                if rest[0]["best_energy_milli"] > values[1]["best_energy_milli"]:
                    raise RuntimeError("archive lost the regular path's best candidate")
                rows.extend(rest)
            else:
                rows.extend(
                    pair(
                        pool,
                        dict(
                            base, preset="archive", label="before", scalar_archive=True
                        ),
                        dict(native, preset="archive", label="native"),
                        "motif_speedup",
                    )
                )
            write_json(output / "results.json", rows)
            print(
                json.dumps({"seed": seed, "complete": True, "records": len(rows)}),
                flush=True,
            )
    write_json(output / "summary.json", summarize(args.suite, rows))
    metadata["completed_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    write_json(output / "metadata.json", metadata)
    print(
        json.dumps({"suite": args.suite, "complete": True, "output": str(output)}),
        flush=True,
    )


if __name__ == "__main__":
    main()
