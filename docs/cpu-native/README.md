# CPU-native research log

Benchmarks and implementation notes for the Quip CPU solver on
`quip/twisted-ice-dimer` revision 1. The benchmarked graph has 4,560 spins
and 41,205 edges. Each study records the build features and runtime settings
used in its comparisons.

## Solid results

The full-budget studies used 16 replicas × 8,192 annealing sweeps on the
canonical 4,560-spin graph. Timing ratios divide reference time by candidate
time. The timing studies ran on one AMD EPYC 7663 VPS.

| Finding | Result | Evidence |
|---|---|---|
| C41 packed fields and component membership | **1.341×** median pipeline speedup over C39 at full budget, with identical outputs in eight same-CPU pairs. | [Full-budget comparisons](C41-full-budget.md) |
| C53 exact Zephyr optimization | Verified ten fixed-boundary optima independently, including a 2,385-spin block at elimination width nine. At full budget, the best canonical energy improved on five of eight instances and tied on three versus C41 with worm polish. Median gain was 90 milli units across all eight. | [Exactness and full-budget quality](C51-C55-zephyr-exact.md#c53-full-budget-quality) |
| C43 tree descent and recombination | Lower best canonical energy on 15 of 20 full-budget instances versus C41, with five ties and a median gain of 90 milli units. | [Full-budget pairs](EXPERIMENTS.md#full-budget-candidate-quality) |
| C47 pair-aware worm polish | Lower best canonical energy on 14 of 20 full-budget instances versus C41, with six ties and a median gain of 145 milli units. | [Full-budget pairs](EXPERIMENTS.md#canonical-energy-and-runtime-comparisons) |
| C55 batching and C57 plan reuse | With 16 random starts and 200 blocks per read, the conditional stage ran **1.440×** as fast as scalar in C55 and **1.561×** as fast as the C55 batched stage in C57. Each study matched all 12 paired outputs. C57 whole-solver timing remains inconclusive. | [C55 stage timing](C51-C55-zephyr-exact.md#c55-conditional-stage-timing) · [C57 plan reuse](EXPERIMENTS.md#reusing-zephyr-neighborhood-plans) |

## Find by number

The complete index gives the subject and result of every numbered finding. [Findings index](FINDINGS.md#entry-index)
The C38 wall-time effect remains inconclusive. [C30–C38 timing](C30-C38-timing.md#c38-wall-time)

| Range | Subject | Direct links |
|---|---|---|
| C01–C08 | CPU execution and candidate preservation | [C01](FINDINGS.md#c01) · [C02](FINDINGS.md#c02) · [C03](FINDINGS.md#c03) · [C04](FINDINGS.md#c04) · [C05](FINDINGS.md#c05) · [C06](FINDINGS.md#c06) · [C07](FINDINGS.md#c07) · [C08](FINDINGS.md#c08) |
| C09–C18 | Temperature, screening, and solve budgets | [C09](FINDINGS.md#c09) · [C10](FINDINGS.md#c10) · [C11](FINDINGS.md#c11) · [C12](FINDINGS.md#c12) · [C13](FINDINGS.md#c13) · [C14](FINDINGS.md#c14) · [C15](FINDINGS.md#c15) · [C16](FINDINGS.md#c16) · [C17](FINDINGS.md#c17) · [C18](FINDINGS.md#c18) |
| C19–C30 | Protocol, validation, and live operation | [C19](FINDINGS.md#c19) · [C20](FINDINGS.md#c20) · [C21](FINDINGS.md#c21) · [C22](FINDINGS.md#c22) · [C23](FINDINGS.md#c23) · [C24](FINDINGS.md#c24) · [C25](FINDINGS.md#c25) · [C26](FINDINGS.md#c26) · [C27](FINDINGS.md#c27) · [C28](FINDINGS.md#c28) · [C29](FINDINGS.md#c29) · [C30](FINDINGS.md#c30) |
| C31–C38 | Storage, resampling, and residual scans | [C31](FINDINGS.md#c31) · [C32](FINDINGS.md#c32) · [C33](FINDINGS.md#c33) · [C34](FINDINGS.md#c34) · [C35](FINDINGS.md#c35) · [C36](FINDINGS.md#c36) · [C37](FINDINGS.md#c37) · [C38](FINDINGS.md#c38) |
| C39–C46 | Component construction, polishing, and CPU trials | [C39](FINDINGS.md#c39) · [C40](FINDINGS.md#c40) · [C41](FINDINGS.md#c41) · [C42](FINDINGS.md#c42) · [C43](FINDINGS.md#c43) · [C44](FINDINGS.md#c44) · [C45](FINDINGS.md#c45) · [C46](FINDINGS.md#c46) |
| C47–C50 | Worm polishing and replica experiments | [C47](FINDINGS.md#c47) · [C48](FINDINGS.md#c48) · [C49](FINDINGS.md#c49) · [C50](FINDINGS.md#c50) |
| C51–C55 | Graph cuts, alternative solvers, and Zephyr optimization | [C51](FINDINGS.md#c51) · [C52](FINDINGS.md#c52) · [C53](FINDINGS.md#c53) · [C54](FINDINGS.md#c54) · [C55](FINDINGS.md#c55) |
| C56–C58 | Anneal budgets, Zephyr plan reuse, and edge scans | [C56](FINDINGS.md#c56) · [C57](FINDINGS.md#c57) · [C58](FINDINGS.md#c58) |

## How to reproduce

From the repository root, run one short offline benchmark on a canonical
instance. The pinned Rust toolchain is 1.97.1. The command emits a JSON
record with candidate energies, a trajectory digest, and elapsed time.

```sh
cargo run --release --locked -j 2 --features experimental --example cpu_native_bench -- \
  --mode pipeline --preset archive --seed 1021 --reads 4 --sweeps 128
```

For the published comparisons, use the source revisions, feature flags,
and inputs recorded for each study. [Reproduction commands](EXPERIMENTS.md#reproduction-and-evaluation-scope) · [Zephyr commands](EXPERIMENTS.md#reproduce-the-zephyr-tests)

## Reference

- Full methods and raw benchmark pairs. [Experiments](EXPERIMENTS.md)
- Solver design and state invariants. [Technical report](TECHNICAL_REPORT.md)
- Builds, feature settings, and validation commands. [Operator guide](OPERATOR_GUIDE.md)
- Coordinator and proof behavior. [Node research](NODE_RESEARCH.md) · [Live operation](LIVE_OPERATION.md)
