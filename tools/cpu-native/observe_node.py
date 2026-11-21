#!/usr/bin/env python3
"""Collect CPU solver observations from Docker and the Quip coordinator API."""

import argparse
import datetime
import json
import os
from pathlib import Path
import re
import subprocess
import time
import urllib.request

ANSI = re.compile(r"\x1b\[[0-9;]*m")


def utc_now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def command(arguments):
    return subprocess.check_output(arguments, text=True).strip()


def number(line, key):
    match = re.search(r"\b" + re.escape(key) + r"=(-?[0-9]+(?:\.[0-9]+)?)", line)
    if match is None:
        return None
    return float(match[1]) if "." in match[1] else int(match[1])


def parse_logs(raw):
    rounds, solves, validation, cancellations, throughput = [], [], [], [], []
    generation, configuration = None, None
    errors = 0
    lines = sorted(ANSI.sub("", raw).splitlines())
    for line in lines:
        timestamp = line.split(" ", 1)[0]
        if "CPU population solver configured" in line:
            configuration = line.split("config=", 1)[-1]
        if "new round generation=" in line:
            generation = number(line, "generation")
            rounds.append(
                {
                    "timestamp": timestamp,
                    "generation": generation,
                    **{
                        key: number(line, key)
                        for key in (
                            "qblock_id",
                            "block",
                            "spec_version",
                            "nodes",
                            "edges",
                            "max_energy",
                            "min_solutions",
                            "min_diversity_milli",
                            "cancelled_jobs",
                        )
                    },
                }
            )
        if (
            ("CPU Funnel solve" in line or "CPU population solve" in line)
            and "best_energy_milli=" in line
            and "salt_index=" in line
        ):
            solves.append(
                {
                    "timestamp": timestamp,
                    "generation_at_log_time": generation,
                    **{
                        key: number(line, key)
                        for key in (
                            "salt_index",
                            "best_energy_milli",
                            "reads",
                            "sweeps",
                        )
                    },
                }
            )
        if "coordinator alive" in line and "results_validated=" in line:
            validation.append(
                {
                    "timestamp": timestamp,
                    **{
                        key: number(line, key)
                        for key in (
                            "generation",
                            "qblock_id",
                            "block",
                            "results_validated",
                            "our_best_energy_this_round",
                        )
                    },
                }
            )
        if "miner acknowledged cancel" in line:
            cancellations.append(
                {
                    "timestamp": timestamp,
                    **{
                        key: number(line, key)
                        for key in (
                            "max_generation",
                            "abandoned_generation",
                            "send_wait_ms",
                            "queue_delay_ms",
                            "ack_wait_ms",
                            "total_ms",
                        )
                    },
                }
            )
        if "miner throughput" in line:
            throughput.append(
                {
                    "timestamp": timestamp,
                    **{
                        key: number(line, key)
                        for key in (
                            "jobs_completed_total",
                            "jobs_rejected_total",
                            "staged",
                            "window",
                        )
                    },
                }
            )
        errors += int(bool(re.search(r"\bERROR\b", line)))
    return {
        "configuration_log": configuration,
        "rounds": rounds,
        "solve_samples": solves,
        "coordinator_validation_samples": validation,
        "cancellation_acknowledgments": cancellations,
        "throughput_samples": throughput,
        "last_validation_counter": (
            validation[-1]["results_validated"] if validation else 0
        ),
        "jobs_rejected_total": (
            throughput[-1]["jobs_rejected_total"] if throughput else None
        ),
        "best_reported_energy_milli": min(
            (row["best_energy_milli"] for row in solves), default=None
        ),
        "error_log_lines": errors,
    }


def api(url, path):
    with urllib.request.urlopen(url.rstrip("/") + path, timeout=15) as response:
        return json.load(response).get("data", {})


def inspect(container):
    template = (
        '{"id":{{json .Id}},"image_id":{{json .Image}},'
        '"started_at":{{json .State.StartedAt}},"running":{{json .State.Running}},'
        '"oom_killed":{{json .State.OOMKilled}},"restart_count":{{json .RestartCount}},'
        '"cpuset":{{json .HostConfig.CpusetCpus}},"memory_limit_bytes":{{json .HostConfig.Memory}}}'
    )
    return json.loads(command(["docker", "inspect", "--format", template, container]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--url",
        required=True,
        help="Coordinator API origin, usually the node's HTTPS URL",
    )
    parser.add_argument("--container", default="quip-cpu")
    parser.add_argument("--binary", default="/data/cpu-native/bin/quip-cpu-native")
    parser.add_argument("--source-revision")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--duration", type=float, default=300, help="Maximum observation seconds"
    )
    parser.add_argument("--interval", type=float, default=30)
    parser.add_argument("--stop-after-results", type=int)
    parser.add_argument("--require-round-transition", action="store_true")
    args = parser.parse_args()
    if args.duration <= 0 or not 1 <= args.interval <= 60:
        parser.error(
            "duration must be positive; interval must be between 1 and 60 seconds"
        )
    initial = inspect(args.container)
    binary_hash = command(
        ["docker", "exec", args.container, "sha256sum", args.binary]
    ).split()[0]
    samples = []
    deadline = time.monotonic() + args.duration
    cpu_model = next(
        (
            line.split(":", 1)[1].strip()
            for line in Path("/proc/cpuinfo").read_text().splitlines()
            if line.startswith("model name")
        ),
        None,
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    while True:
        state = inspect(args.container)
        if state["id"] != initial["id"]:
            raise RuntimeError(
                "container changed during observation; start a new record"
            )
        process = subprocess.run(
            [
                "docker",
                "logs",
                "--timestamps",
                "--since",
                initial["started_at"],
                args.container,
            ],
            capture_output=True,
            text=True,
            check=True,
        )
        parsed = parse_logs(process.stdout + process.stderr)
        resource = json.loads(
            command(
                [
                    "docker",
                    "stats",
                    "--no-stream",
                    "--format",
                    "{{json .}}",
                    args.container,
                ]
            )
        )
        samples.append(
            {
                "timestamp": utc_now(),
                "cpu_percent": float(resource["CPUPerc"].rstrip("%")),
                "memory_usage": resource["MemUsage"],
                "pids": int(resource["PIDs"]),
            }
        )
        record = {
            "backend": "quip-cpu-native",
            "started_at": initial["started_at"],
            "observed_until": utc_now(),
            "binary_sha256": binary_hash,
            "source_revision": args.source_revision,
            "cpu_model": cpu_model,
            "host_vcpus": os.cpu_count(),
            **state,
            **parsed,
            "resource_samples": samples,
        }
        try:
            status = api(args.url, "/api/v1/status")
            stats = api(args.url, "/api/v1/stats")
            record["coordinator_controller"] = stats.get("controller", {})
            record["chain_head_number"] = status.get("chain", {}).get("head_number")
            record["miner_registered"] = status.get("miner_registered")
            record["is_mining"] = status.get("is_mining")
            record["miner_proofs_won"] = status.get("miner_info", {}).get("proofs_won")
        except (OSError, ValueError) as error:
            record["api_observation_error"] = str(error)
        record["notes"] = [
            "results_validated is the coordinator's cumulative canonical-candidate counter.",
            "Solver salt indices are local to a lease; generation_at_log_time is a timestamp association.",
            "Cancel acknowledgement is control-protocol latency, separate from solver execution time.",
            "Controller duplicate-result drops are recorded separately from validation and job rejection.",
        ]
        temporary = args.output.with_suffix(args.output.suffix + ".tmp")
        temporary.write_text(json.dumps(record, indent=2) + "\n")
        temporary.replace(args.output)
        print(
            json.dumps(
                {
                    "observed_until": record["observed_until"],
                    "validated": record["last_validation_counter"],
                    "reported_solves": len(record["solve_samples"]),
                    "rounds": len(record["rounds"]),
                    "best_energy_milli": record["best_reported_energy_milli"],
                    "jobs_rejected": record["jobs_rejected_total"],
                }
            ),
            flush=True,
        )
        enough = (
            args.stop_after_results is not None
            and record["last_validation_counter"] >= args.stop_after_results
        )
        if args.require_round_transition:
            enough = (
                enough
                and len(record["rounds"]) > 1
                and any(
                    row["timestamp"] > record["rounds"][1]["timestamp"]
                    for row in record["solve_samples"]
                )
            )
        if enough or time.monotonic() >= deadline:
            break
        time.sleep(min(args.interval, max(0, deadline - time.monotonic())))


if __name__ == "__main__":
    main()
