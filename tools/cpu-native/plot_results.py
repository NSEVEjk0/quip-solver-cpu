#!/usr/bin/env python3
"""Render SVG and PNG figures from the committed CPU-native experiment records."""

import argparse
import json
import pathlib
import statistics

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

REPO = pathlib.Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--data", type=pathlib.Path, default=REPO / "docs/cpu-native/data")
parser.add_argument(
    "--output", type=pathlib.Path, default=REPO / "docs/cpu-native/figures"
)
args = parser.parse_args()
ROOT = args.data.resolve()
OUT = args.output.resolve()
OUT.mkdir(parents=True, exist_ok=True)
plt.rcParams.update(
    {
        "font.size": 11,
        "axes.spines.top": False,
        "axes.spines.right": False,
        "svg.fonttype": "none",
        "svg.hashsalt": "quip-cpu-native-20261121",
    }
)
colors = ["#8aa5bb", "#487da4", "#346486", "#173f60", "#d28445"]


def save(fig, name):
    fig.tight_layout()
    svg = OUT / (name + ".svg")
    fig.savefig(svg, metadata={"Date": None})
    svg.write_text(
        "\n".join(line.rstrip() for line in svg.read_text().splitlines()) + "\n"
    )
    fig.savefig(OUT / (name + ".png"), dpi=200)
    plt.close(fig)


r = json.loads((ROOT / "ablation-study/results.json").read_text())
variants = ["cache", "copy", "union", "fusion", "core"]
labels = [
    "Probability\nreuse",
    "Field\ntransfer",
    "Union\ninlining",
    "Adjacency\nfusion",
    "Combined\nexecution",
]
values = [
    [x["speedup"] for x in r if x["paired_variant"] == v and x["preset"] != "Baseline"]
    for v in variants
]
fig, ax = plt.subplots(figsize=(8.4, 4.5))
ax.bar(range(5), [statistics.median(x) for x in values], color=colors)
for i, x in enumerate(values):
    ax.scatter(
        [i + (j - 3.5) * 0.018 for j in range(len(x))],
        x,
        s=18,
        color="#233342",
        alpha=0.6,
    )
ax.axhline(1, color="#59636a", linewidth=1, linestyle="--")
ax.set_xticks(range(5), labels)
ax.set_ylabel("Paired execution speedup over reference")
ax.set_title(
    "CPU kernel ablation · 8 instances · 16 replicas × 2,048 sweeps\nBars: median · dots: paired instances"
)
ax.set_ylim(0.8, 1.5)
save(fig, "kernel-ablation")
r = json.loads((ROOT / "confirmation/results.json").read_text())
labels = ["reference", "native-execution", "native-archive", "native-beta10"]
fig, axes = plt.subplots(1, 2, figsize=(10.5, 4.2))
means = [statistics.mean(x["seconds"] for x in r if x["label"] == v) for v in labels]
axes[0].bar(range(4), means, color=colors[:4])
axes[0].set_ylabel("Mean elapsed seconds")
axes[0].set_xticks(
    range(4), ["Reference", "Native\nexecution", "Archive\nfull-edge", "Archive\nβ=10"]
)
for seed in sorted({x["seed"] for x in r}):
    xs = [
        next(x for x in r if x["seed"] == seed and x["label"] == v)["best_energy_milli"]
        / 1000
        for v in labels
    ]
    axes[1].plot(range(4), xs, color="#74818a", alpha=0.45, marker="o", markersize=3)
axes[1].set_xticks(
    range(4), ["Reference", "Native\nexecution", "Archive\nfull-edge", "Archive\nβ=10"]
)
axes[1].set_ylabel("Best original energy · lower is better")
fig.suptitle(
    "Full pipeline confirmation · 8 held-out instances · 16 × 8,192\nArchive evaluated before motif-domain scoring"
)
save(fig, "pipeline-confirmation")
r = json.loads((ROOT / "temperature-study/results.json").read_text())
betas = sorted({x["beta_max"] for x in r})
values = [
    [x["best_energy_milli"] / 1000 for x in r if x["beta_max"] == b] for b in betas
]
fig, ax = plt.subplots(figsize=(7.4, 4.5))
ax.errorbar(
    betas,
    [statistics.mean(x) for x in values],
    yerr=[statistics.stdev(x) for x in values],
    marker="o",
    capsize=4,
    color="#346486",
)
ax.set_xlabel("Geometric ladder endpoint β")
ax.set_ylabel("Mean best original energy · lower is better")
ax.set_title(
    "Temperature pilot · 6 instances · 16 × 2,048\nError bars show instance standard deviation"
)
save(fig, "temperature-study")
r = json.loads((ROOT / "motif-confirmation/results.json").read_text())
fig, ax = plt.subplots(figsize=(6.8, 4.5))
for seed in sorted({x["seed"] for x in r}):
    a = next(x for x in r if x["seed"] == seed and x["kind"] == "before")
    b = next(x for x in r if x["seed"] == seed and x["kind"] == "native")
    ax.plot(
        [0, 1], [a["seconds"], b["seconds"]], marker="o", color="#346486", alpha=0.6
    )
ax.set_xticks([0, 1], ["Full-edge archive scoring", "Motif-domain archive scoring"])
ax.set_ylabel("Pipeline elapsed seconds")
ax.set_title(
    "Exact archive scoring · 8 paired instances · 16 × 8,192\nAll candidate outputs identical"
)
save(fig, "motif-scoring")
print(str(OUT))
