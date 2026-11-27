# CPU-native research log

Benchmarks and implementation notes for the Quip CPU solver on
`quip/twisted-ice-dimer` revision 1. The benchmarked graph has 4,560 spins
and 41,205 edges. Each study records the build features and runtime settings
used in its comparisons.

## Quick navigation

Wall-time ratios divide reference time by candidate time. Lower canonical
energy is better.

| Finding | Summary | Result | Details |
|---|---|---|---|
| [C30–C38](FINDINGS.md#c30) | Live operation, storage, and timing experiments | C38 returned identical outputs in eight pairs. The observed median wall-time ratio was 1.028×. The timing gain remains inconclusive. | [C30–C38 timing](C30-C38-timing.md) |
| [C41](FINDINGS.md#c41) | Packed local fields and component construction | 1.341× median over C39 on the same CPU. The median against the earlier native build was 1.516× side by side. | [C41 full-budget comparisons](C41-full-budget.md) |
| [C53](FINDINGS.md#c53) | Exact optimization of up to 2,385 Zephyr spins with the boundary fixed | Best energy improved on five of eight full-budget instances and tied on three. The median best-energy gain was 90 milli units across all eight. | [C51–C55 Zephyr studies](C51-C55-zephyr-exact.md#c53-full-budget-quality) |
| [C55](FINDINGS.md#c55) | Batched Zephyr conditional optimization | 1.440× median for the conditional stage over its scalar implementation. All 12 paired outputs matched exactly. | [C55 stage timing](C51-C55-zephyr-exact.md#c55-conditional-stage-timing) |

## Find by number

The complete index gives the subject and result of every numbered finding. [Findings index](FINDINGS.md#entry-index)

| Range | Subject | Direct links |
|---|---|---|
| C01–C08 | CPU execution and candidate preservation | [C01](FINDINGS.md#c01) · [C02](FINDINGS.md#c02) · [C03](FINDINGS.md#c03) · [C04](FINDINGS.md#c04) · [C05](FINDINGS.md#c05) · [C06](FINDINGS.md#c06) · [C07](FINDINGS.md#c07) · [C08](FINDINGS.md#c08) |
| C09–C18 | Temperature, screening, and solve budgets | [C09](FINDINGS.md#c09) · [C10](FINDINGS.md#c10) · [C11](FINDINGS.md#c11) · [C12](FINDINGS.md#c12) · [C13](FINDINGS.md#c13) · [C14](FINDINGS.md#c14) · [C15](FINDINGS.md#c15) · [C16](FINDINGS.md#c16) · [C17](FINDINGS.md#c17) · [C18](FINDINGS.md#c18) |
| C19–C30 | Protocol, validation, and live operation | [C19](FINDINGS.md#c19) · [C20](FINDINGS.md#c20) · [C21](FINDINGS.md#c21) · [C22](FINDINGS.md#c22) · [C23](FINDINGS.md#c23) · [C24](FINDINGS.md#c24) · [C25](FINDINGS.md#c25) · [C26](FINDINGS.md#c26) · [C27](FINDINGS.md#c27) · [C28](FINDINGS.md#c28) · [C29](FINDINGS.md#c29) · [C30](FINDINGS.md#c30) |
| C31–C38 | Storage, resampling, and residual scans | [C31](FINDINGS.md#c31) · [C32](FINDINGS.md#c32) · [C33](FINDINGS.md#c33) · [C34](FINDINGS.md#c34) · [C35](FINDINGS.md#c35) · [C36](FINDINGS.md#c36) · [C37](FINDINGS.md#c37) · [C38](FINDINGS.md#c38) |
| C39–C46 | Component construction, polishing, and CPU trials | [C39](FINDINGS.md#c39) · [C40](FINDINGS.md#c40) · [C41](FINDINGS.md#c41) · [C42](FINDINGS.md#c42) · [C43](FINDINGS.md#c43) · [C44](FINDINGS.md#c44) · [C45](FINDINGS.md#c45) · [C46](FINDINGS.md#c46) |
| C47–C50 | Worm polishing and replica experiments | [C47](FINDINGS.md#c47) · [C48](FINDINGS.md#c48) · [C49](FINDINGS.md#c49) · [C50](FINDINGS.md#c50) |
| C51–C55 | Graph cuts, alternative solvers, and Zephyr optimization | [C51](FINDINGS.md#c51) · [C52](FINDINGS.md#c52) · [C53](FINDINGS.md#c53) · [C54](FINDINGS.md#c54) · [C55](FINDINGS.md#c55) |

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
