# C30–C38. Operation and timing

[CPU-native research log](README.md)

| Finding | What the evidence showed | Records |
|---|---|---|
| [C30](FINDINGS.md#c30) | The extended live observation recorded 715 coordinator validations across 41 observed rounds. | [Observation record](data/live-observation-extended.json) |
| [C31](FINDINGS.md#c31) | 32-bit fields halved live field storage. The median kernel elapsed-time ratio was 0.964×. | [Paired runs](data/storage-resampling/fields32/results.json) |
| [C32](FINDINGS.md#c32) | 16-bit cache stamps reduced the table payload. The median elapsed-time ratio was 0.962×. | [Paired runs](data/storage-resampling/stamps16/results.json) |
| [C33](FINDINGS.md#c33) | Selective resampling skipped 86% of field restores at the median. Elapsed time varied on both sides of parity. | [Paired runs](data/storage-resampling/selective/results.json) |
| [C34](FINDINGS.md#c34) | The combined storage and resampling variants preserved all output digests. Their timing intervals spanned parity. | [Ablations](EXPERIMENTS.md#storage-and-resampling-pilots) |
| [C35](FINDINGS.md#c35) | The cluster-boundary update measured 1.129× in a kernel pilot and 0.996× in the full pipeline. | [Timing by budget](EXPERIMENTS.md#cluster-boundary-field-updates) |
| [C36](FINDINGS.md#c36) | Packing component labels and spin signs preserved outputs. The fresh instrumented runs had a 0.939× median elapsed-time ratio. | [Timing and counters](EXPERIMENTS.md#packed-component-labels-and-spin-signs) |
| [C37](FINDINGS.md#c37) | Grouping residual edges by endpoint retired 10.6% fewer instructions at the median. The full-pipeline elapsed-time ratio was 0.997×. | [Timing and counters](EXPERIMENTS.md#row-grouped-residual-energy-scan) |
| [C38](FINDINGS.md#c38) | Masking interior edges preserved all eight outputs at full budget. | [C38 wall-time evidence](#c38-wall-time) |

## C38 wall time

The full-pipeline comparison used 16 replicas × 8,192 sweeps. Its median
wall-time ratio was 1.028×, with a paired bootstrap 95% median interval of
1.006–1.061× for those eight pairs. A subsequent same-binary C39 control on
the same VPS produced ratios from 0.952× to 1.120×. The C38 elapsed-time
gain remains inconclusive. Whole-process branch misses and cycles fell by
28.8% and 5.1% at the median in the C38 comparison.
[C38 pairs](data/residual-edge-scan/confirm-masked/results.json) · [Same-binary control](EXPERIMENTS.md#same-binary-timing-control)

## Reproduce

The experiment report lists the reference and candidate build flags, seeds,
and commands for the storage, residual-edge, and masking comparisons.
[Reproduction commands](EXPERIMENTS.md#reproduction-and-evaluation-scope)
