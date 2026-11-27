# C51–C55. Conditional optimization on Zephyr

[CPU-native research log](README.md)

| Finding | Experiment | Measured result | Evidence |
|---|---|---|---|
| [C51](FINDINGS.md#c51) | Exact cuts on balanced signed neighborhoods | Best energy improved on three of six short-budget instances and tied on three. | [Graph-cut pairs](data/conditional-search/min-cut-pilot/results.json) |
| [C52](FINDINGS.md#c52) | Standalone discrete simulated bifurcation | Best energy was higher on all four tested instances than with C41 plus worm. | [Bifurcation pairs](data/conditional-search/dsb-pilot/results.json) |
| [C53](FINDINGS.md#c53) | Exact conditional Zephyr neighborhoods | Up to 2,385 free spins at elimination width nine. Ten independent conditional optimum checks matched. | [Exact audits](data/conditional-search/exact-audit.json) |
| [C54](FINDINGS.md#c54) | Random starts, equal-energy moves, and exploratory fields | Random starts fell short of the incumbent on all four instances. The plateau pass matched the strict pass's best energies on six pilot instances. | [Methods and comparisons](EXPERIMENTS.md#random-starts-plateaus-and-perturbations) |
| [C55](FINDINGS.md#c55) | Batched conditional optimization | 1.440× median elapsed-time ratio over the scalar conditional stage, with identical outputs in 12 of 12 pairs. | [Batched confirmation pairs](data/conditional-search/spine-batch-confirm/results.json) |

## C53 full-budget quality

With the outside spins fixed, a four-spine block frees 2,191–2,385 spins.
Adding 50 exact four-spine moves per candidate to C41 plus worm at
16 replicas × 8,192 sweeps improved best energy on five instances and tied
on three. The median best-energy gain across all eight was 90 milli units.
The median candidate improved on all eight. The full-pipeline timing
interval spans parity.
[Method](EXPERIMENTS.md#exact-conditional-zephyr-neighborhoods) · [Full-budget results](data/conditional-search/spine-wide-full-summary.json)

## C55 conditional stage timing

The two arms used identical starting spins and 200 four-spine blocks per
read. Each of the 12 instances ran twice per arm on the same pinned CPU
on one EPYC 7663 VPS. Batching eight reads gave a 1.440× median speedup
for the conditional stage, equivalent to 30.6% less elapsed time. The
paired bootstrap 95% median interval was 1.226–1.537×.
[Method](EXPERIMENTS.md#batched-zephyr-conditional-optimization) · [Timing summary](data/conditional-search/spine-batch-confirm/summary.json)

## Reproduce

The experiment report includes the benchmark commands, build flags, and
an independent check for a single conditional optimum.
[Zephyr reproduction commands](EXPERIMENTS.md#reproduce-the-zephyr-tests)
