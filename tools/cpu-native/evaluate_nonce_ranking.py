#!/usr/bin/env python3
"""Freeze the CPU nonce ranker on development seeds and assess fresh labels."""

import argparse
from collections import Counter
from itertools import combinations
import json
from math import comb
from pathlib import Path
import random
from statistics import median


FIRST, DEV_COUNT, TEST_COUNT = 2301, 24, 80
ABSOLUTE_MILLI = -5_090_000
GEOMETRIC = [
    f"{rank}.{score}"
    for rank in ("rank2", "rank4", "rank4_weak3")
    for score in ("relaxed_milli", "descended_best_milli")
]
STRUCTURAL = ["structure.weak_abs_field_score", "structure.frustrated_weak_triangles"]
SHORT = "probe4x64.best_milli"
MEDIUM = "probe4x256.best_milli"


def read_rows(labels, scores, seeds):
    result = []
    for seed in seeds:
        label = json.loads((labels / f"{seed}.json").read_text())
        score = json.loads((scores / f"{seed}.json").read_text())
        assert label["seed"] == score["seed"] == seed and label["nonce"] == score["nonce"]
        assert label["reads"] == 16 and label["sweeps"] == 8192
        assert label["best_energy_milli"] == min(label["energies_milli"])
        result.append({"seed": seed, "energy": label["best_energy_milli"], "score": score})
    return result


def predictor(row, path):
    block, field = path.split(".")
    return row["score"][block][field]


def concordance(rows, method):
    good, total = 0.0, 0
    for a, b in combinations(rows, 2):
        if a["energy"] == b["energy"]:
            continue
        low, high = (a, b) if a["energy"] < b["energy"] else (b, a)
        x, y = predictor(low, method), predictor(high, method)
        good += float(x < y) + 0.5 * float(x == y)
        total += 1
    return good / total if total else 0.5


def normalize(rows, method):
    scores = [predictor(r, method) for r in rows]
    center = median(scores)
    return [center, median(abs(s - center) for s in scores) or 1.0]


def calibrate(args):
    rows = read_rows(args.labels, args.scores, range(FIRST, FIRST + DEV_COUNT))
    trial = {method: concordance(rows, method) for method in GEOMETRIC + STRUCTURAL + [SHORT, MEDIUM]}
    geometric = max(GEOMETRIC, key=trial.get)
    structural = max(STRUCTURAL, key=trial.get)
    policy = {
        "development": [FIRST, FIRST + DEV_COUNT - 1],
        "holdout": [FIRST + DEV_COUNT, FIRST + DEV_COUNT + TEST_COUNT - 1],
        "chosen_geometry": geometric,
        "chosen_structure": structural,
        "blend_geometry_scale": normalize(rows, geometric),
        "blend_short_scale": normalize(rows, SHORT),
        "development_pair_concordance": trial,
        "primary_contrast": "chosen_geometry minus short_probe on holdout top-decile AUROC",
        "ranking_target_milli": ABSOLUTE_MILLI,
        "selection_budget": {"nonce_pool": 10, "full_solves": 3},
    }
    with args.output.open("x") as out:
        json.dump(policy, out, indent=2)
        out.write("\n")
    print(json.dumps(policy, indent=2))


def order_score(row, policy, method):
    if method != "blend":
        return predictor(row, method)
    a, b = policy["blend_geometry_scale"], policy["blend_short_scale"]
    return ((predictor(row, policy["chosen_geometry"]) - a[0]) / a[1]
            + (predictor(row, SHORT) - b[0]) / b[1])


def auc_table(rows, target, policy, method):
    positives = [order_score(r, policy, method) for r in rows if r["energy"] <= target]
    negatives = [order_score(r, policy, method) for r in rows if r["energy"] > target]
    return [[float(a < b) + 0.5 * float(a == b) for b in negatives] for a in positives]


def auc(table):
    if not table or not table[0]:
        return None
    return sum(map(sum, table)) / (len(table) * len(table[0]))


def interval(samples):
    samples.sort()
    return [samples[int(len(samples) * q)] for q in (0.025, 0.975)]


def auc_intervals(tables, rng, repeats=3000):
    sample = next(iter(tables.values()))
    p, n = len(sample), len(sample[0])
    draws = {name: [] for name in tables}
    deltas = {name: [] for name in tables if name != "short_probe"}
    for _ in range(repeats):
        pos = Counter(rng.choices(range(p), k=p))
        neg = Counter(rng.choices(range(n), k=n))
        current = {}
        for name, table in tables.items():
            value = sum(
                pi * sum(nj * table[i][j] for j, nj in neg.items())
                for i, pi in pos.items()
            ) / (p * n)
            current[name] = value
            draws[name].append(value)
        for name, samples in deltas.items():
            samples.append(current[name] - current["short_probe"])
    return {name: interval(values) for name, values in draws.items()}, {
        name: interval(values) for name, values in deltas.items()
    }


def expected_min(energies, k):
    # Uniform k-subsets whose earliest sorted index is i.
    energies = sorted(energies)
    total = comb(len(energies), k)
    return sum(
        e * comb(len(energies) - i - 1, k - 1)
        for i, e in enumerate(energies)
    ) / total


def expected_selected(group, target, policy, method, k):
    scores = [order_score(r, policy, method) for r in group]
    edge = sorted(scores)[k-1]
    forced = [r["energy"] for r, s in zip(group, scores, strict=True) if s < edge]
    tied = [r["energy"] for r, s in zip(group, scores, strict=True) if s == edge]
    # Average over ties at the selection cutoff without consulting their
    # energies to decide which nonces to keep.
    selections = [forced + list(extra) for extra in combinations(tied, k-len(forced))]
    expected_best = sum(min(selected) for selected in selections) / len(selections)
    expected_hits = sum(sum(e <= target for e in selected) for selected in selections) / len(selections)
    return expected_best, expected_hits


def seconds(row, method, policy):
    score = row["score"]
    generation = score["generation_seconds"]
    adjacency = score["adjacency_seconds"]
    greedy = score["greedy_setup_seconds"]
    if method == "blend":
        return seconds(row, policy["chosen_geometry"], policy) + score["probe4x64"]["seconds"]
    stage = method.split(".")[0]
    if stage == "structure":
        return generation + adjacency + score[stage]["seconds"]
    if stage.startswith("rank"):
        return generation + adjacency + greedy + score[stage]["seconds"]
    return generation + score[stage]["seconds"]


def evaluate(args):
    policy = json.loads(args.policy.read_text())
    rows = read_rows(args.labels, args.scores, range(FIRST + DEV_COUNT, FIRST + DEV_COUNT + TEST_COUNT))
    low, high = min(r["energy"] for r in rows), max(r["energy"] for r in rows)
    target = sorted(r["energy"] for r in rows)[(len(rows)+9)//10-1]
    absolute_positives = sum(r["energy"] <= ABSOLUTE_MILLI for r in rows)
    positives = sum(r["energy"] <= target for r in rows)
    methods = {
        "geometry": policy["chosen_geometry"],
        "structure": policy["chosen_structure"],
        "short_probe": SHORT,
        "medium_probe": MEDIUM,
        "blend": "blend",
    }
    tables = {name: auc_table(rows, target, policy, method) for name, method in methods.items()}
    rng = random.Random(281127)
    cis, delta_cis = auc_intervals(tables, rng)
    scores = {}
    groups = [rows[at:at+10] for at in range(0,len(rows),10)]
    random_hits = positives * 0.3
    for name, method in methods.items():
        gains, hits = [], []
        for group in groups:
            expect = expected_min([r["energy"] for r in group], 3)
            selected, hit = expected_selected(group, target, policy, method, 3)
            gains.append(expect - selected)
            hits.append(hit)
        gain_ci = interval([sum(sample) / len(sample)
                            for sample in (rng.choices(gains, k=len(gains)) for _ in range(5000))])
        roc = auc(tables[name])
        scores[name] = {
            "method": method,
            "absolute_auc90": auc(auc_table(rows,ABSOLUTE_MILLI,policy,method)),
            "top_decile_auroc": roc,
            "top_decile_auroc_ci95": cis[name],
            "delta_vs_short_auroc": (roc - auc(tables["short_probe"])) if name in delta_cis else None,
            "delta_vs_short_auroc_ci95": delta_cis.get(name),
            "budgeted_top_decile_hits_out_of": [sum(hits), positives],
            "expected_random_hits": random_hits,
            "mean_best_energy_gain_vs_random_milli": sum(gains) / len(gains),
            "budgeted_gain_ci95_milli": gain_ci,
            "group_best_gains_milli": gains,
            "score_seconds_median_from_run": median(seconds(r, method, policy) for r in rows),
        }
    result = {
        "n":len(rows),
        "full_energy_min_max_milli":[low,high],
        "absolute_target_milli":ABSOLUTE_MILLI,
        "absolute_target_positives":absolute_positives,
        "relative_top_decile_cutoff_milli":target,
        "relative_top_decile_positives":positives,
        "groups":len(groups),
        "shortlist_per_group":3,
        "scores":scores,
    }
    with args.output.open("x") as f:
        json.dump(result,f,indent=2)
        f.write("\n")
    print(json.dumps(result,indent=2))


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("phase",choices=("calibrate","evaluate"))
    p.add_argument("--labels",type=Path,required=True)
    p.add_argument("--scores",type=Path,required=True)
    p.add_argument("--output",type=Path,required=True)
    p.add_argument("--policy",type=Path)
    args=p.parse_args()
    if args.phase=="calibrate":
        calibrate(args)
    else:
        if args.policy is None:
            p.error("evaluate needs a frozen --policy")
        evaluate(args)


if __name__=="__main__":
    main()
