# C41. Packed fields at full budget

[CPU-native research log](README.md)

The final C41 build combines packed full and residual local fields with
shallow union roots, linked component membership, and the C39 tile lookup.
The timer covers the complete offline solver call at 16 replicas × 8,192
sweeps, including graph preparation and archive polishing.
[Implementation and test setup](EXPERIMENTS.md#packed-residual-fields)

| Reference and run order | Median speedup | Paired bootstrap 95% median interval | Identical outputs | Raw pairs |
|---|---:|---:|---:|---|
| C39, sequential on the same CPU | 1.341× | 1.291–1.398× | 8/8 | [Seeds 1231–1238](data/cluster-fields/serial-final-c39/results.json) |
| C39, side by side | 1.420× | 1.346–1.480× | 8/8 | [Seeds 1221–1228](data/cluster-fields/full-final-c39/results.json) |
| Earlier native build used as the C38 reference, side by side | 1.516× | 1.418–1.674× | 8/8 | [Seeds 1241–1248](data/cluster-fields/full-final-native/results.json) |

Each comparison used eight fresh instances on one EPYC 7663 VPS and
checked the ordered candidate spins and canonical energies.
[Comparison details](EXPERIMENTS.md#full-budget-comparisons)

## Reproduce

The build manifest pins the three executable configurations. The paired
commands cover the side-by-side runs and the sequential same-CPU repeat.
[Build manifest](data/cluster-fields/build.json) · [Reproduction commands](EXPERIMENTS.md#reproduction-and-evaluation-scope)
