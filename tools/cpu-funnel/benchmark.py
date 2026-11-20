#!/usr/bin/env python3
"""Reproduce CPU Funnel trials without chain access, signing, or submissions."""
import argparse
import json
from pathlib import Path
import statistics
import subprocess

REPO = Path(__file__).resolve().parents[2]
IMAGE = "registry.gitlab.com/quip.network/quip-solver/v0.3/quip-solver@sha256:4050700c2d00ab5ace4fbcc87ab59e3381054d939124ee39dac4a21284c30525"


def trials(suite):
    if suite == "screening":
        for number, seed in enumerate((722, 723, 724)):
            for screen in ((True, False) if number % 2 == 0 else (False, True)):
                yield {
                    "seed": seed, "arm": "screen" if screen else "direct",
                    "screen": screen, "reads": 16, "sweeps": 8192 if screen else 9216,
                    "batch": 128, "keep": 4, "probe_reads": 4, "probe_sweeps": 1024,
                }
    elif suite == "stock":
        for arm, sweeps in (("stock-8192", 8192), ("tuned-8192", 8192), ("tuned-32768", 32768)):
            yield {"seed": 725, "arm": arm, "screen": False, "reads": 16, "sweeps": sweeps}
    else:
        for screen in (False, True):
            yield {
                "seed": 726, "arm": "smoke-screen" if screen else "smoke-direct",
                "screen": screen, "reads": 4, "sweeps": 64,
                "batch": 8, "keep": 2, "probe_reads": 4, "probe_sweeps": 32,
            }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", choices=("screening", "stock", "smoke"), default="smoke")
    parser.add_argument("--cpus", default="0,1", help="Two logical CPUs available for the benchmark")
    parser.add_argument("--image", default=IMAGE)
    parser.add_argument("--output", type=Path, default=Path("cpu-funnel-results"))
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    output = args.output.resolve()
    if not args.dry_run:
        output.mkdir(parents=True, exist_ok=True)
    results = []
    for trial in trials(args.suite):
        screen = trial["screen"]
        name = f"{trial['seed']}-{trial['arm']}"
        binary = "quip-cpu-otsuka" if trial["arm"].startswith("stock") else "quip-cpu-funnel"
        if not args.dry_run and not (REPO / "target/release" / binary).is_file():
            raise SystemExit(f"Build target/release/{binary} first; see README.cpu-funnel.md")
        lease_size = trial.get("batch", 128) if screen else 1
        command = [
            "docker", "run", "--rm", "--network", "none", "--cpuset-cpus", args.cpus,
            "--memory", "768m", "-v", f"{REPO}:/cpu:ro", "-v", f"{output}:/results",
            "-e", f"QUIP_CPU_FUNNEL_SCREEN={str(screen).lower()}",
            "-e", f"QUIP_CPU_FUNNEL_FINAL_READS={trial['reads']}",
            "-e", f"QUIP_CPU_FUNNEL_FINAL_SWEEPS={trial['sweeps']}",
        ]
        for key in ("batch", "keep", "probe_reads", "probe_sweeps"):
            if key in trial:
                command.extend(("-e", f"QUIP_CPU_FUNNEL_{key.upper()}={trial[key]}"))
        command.extend([
            "--entrypoint", "quip-coordinator", args.image, "drive", "--miner",
            f"/cpu/target/release/{binary}", "--source", "lease", "--topology",
            "/cpu/tools/cpu-funnel/inputs/current-topology.spec.json", "--family-id",
            "quip/twisted-ice-dimer", "--family-params",
            "/cpu/tools/cpu-funnel/inputs/current-family.params", "--count", str(2 * lease_size),
            "--lease-size", str(lease_size), "--seed", str(trial["seed"]),
            "--num-reads", str(trial["reads"]), "--num-sweeps", str(trial["sweeps"]),
            "--target-energy", "-5092", "--report", f"/results/{name}.jsonl",
            "--log-level", "info",
        ])
        if args.dry_run:
            print(json.dumps(command))
            continue
        print(f"Starting {name}", flush=True)
        with (output / f"{name}.log").open("w") as log:
            subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True)
        rows = [json.loads(line) for line in (output / f"{name}.jsonl").read_text().splitlines()]
        aggregate = rows[-1]
        energies = [row["raw_best_energy_milli"] / 1000 for row in rows[:-1]]
        if aggregate["rejected"] or len(energies) != 2:
            raise RuntimeError(f"Invalid results in {name}; inspect its log")
        sweeps = trial["reads"] * trial["sweeps"]
        if screen:
            sweeps += trial["keep"] * trial["probe_reads"] * trial["probe_sweeps"]
        result = {
            **trial, "energies": energies, "round_best": min(energies),
            "mean_energy": statistics.mean(energies),
            "wall_seconds": aggregate["wall_ms_total"] / 1000,
            "salts_done": aggregate["salts_done"], "rejected": aggregate["rejected"],
            "replica_sweeps_total": 2 * sweeps,
        }
        results.append(result)
        (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
        print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
