# CPU baseline measurements — 20 November 2026

These records establish the initial CPU configuration and tuning baseline.
Current CPU-native implementation measurements are documented in
[the native experiment report](../cpu-native/EXPERIMENTS.md).

Hardware: six AMD EPYC 7663 vCPUs, about 5.8 GiB RAM, shared with other
services. Every benchmark used two logical CPUs, a 768 MiB container limit,
and the same release build. The live trial uses two CPUs and a 1 GiB limit.
This is a VPS measurement; other CPUs and host contention can change timing.

## Instance and protocol

The read-only chain snapshot uses runtime spec 121, `quip/twisted-ice-dimer`
revision 1, weak 45, 1350 pairs, 4560 nodes, and 41205 edges. Topology:
`0x2d130ce6df0fceacff26b90a06811829707bde8227d45e04ef71c804a8a5345c`.
Complete [inputs](../../tools/cpu-funnel/inputs) are included.

Different miner salts still produce different family instances. Screening
is an attempt to spend solver time on favorable instances; its value must
be measured for the specific recipe, hardware, and budget.

[Published validator code](https://gitlab.com/quip.network/quip-validator/-/blob/main/pallets/quantum-pow/src/lib.rs#L1077) binds nonces to the previous proof block hash,
miner account, and salt, and rejects old-round nonces. The device timing
[field is self-reported](https://gitlab.com/quip.network/quip-validator/-/blob/main/pallets/quantum-pow/src/types.rs#L35); the analysis uses measured wall time. The implementation reports CPU hardware and
scores every proof against the original coefficients.

## Screening at equal sweep counts

Each arm used 294912 total replica-sweeps across two workers. Per worker,
screening used four 4-read / 1024-sweep probes plus one 16-read / 8192-sweep
final solve. Direct used one 16-read / 9216-sweep solve. Both used the tuned
kernel and final polishing. Generation, structural filters, exact scoring,
and polishing add work beyond the sweep count; reported wall time includes it.
Arm order alternated across three seeds.

| Seed | Screen best | Direct best | Screen seconds | Direct seconds |
|---|---:|---:|---:|---:|
| 722 | -5050.265 | -5058.375 | 57.525 | 52.996 |
| 723 | -5050.335 | -5058.895 | 57.137 | 52.200 |
| 724 | -5052.885 | -5045.055 | 55.431 | 53.622 |

Mean round best: screen -5051.162, direct -5054.108. Mean wall time:
56.698 versus 52.939 seconds, approximately 7% longer for screening.
All six runs had zero rejected results. Three rounds are insufficient
to infer a reliable advantage or win rate. Lower energy is better.

## Stock and longer-budget comparison

Seed 725 generated the same two nonce instances in all three arms.
Each used 16 reads and two vCPUs.

| Solver | Sweeps | Mean energy | Best energy | Wall seconds |
|---|---:|---:|---:|---:|
| Stock Otsuka | 8192 | -5033.700 | -5036.485 | 37.270 |
| Tuned direct | 8192 | -5049.000 | -5050.575 | 47.155 |
| Tuned direct | 32768 | -5056.390 | -5057.465 | 180.438 |

At equal sweeps, tuned direct gained 15.3 energy units (15,300 milli units)
while taking 26.5% longer. Quadrupling its sweep budget gained another
7.39 energy units (7,390 milli units), at 3.83 times the
wall time. The live trial starts at 8192 sweeps to try more instances per
round. This two-instance sample supports testing the tradeoff, not a
general speed or win-rate claim. All three runs had zero rejected results
and remained above the -5092 benchmark target.

## Live trial

The previous live miner used one CPU with `quip-cpu-sa` and usually reached
approximately -4760. The new trial uses the same operator identity and
coordinator, two CPU workers, 16 reads / 8192 sweeps, and screening off.
The observed best reached -5052.435. RAM snapshots were approximately
62 and 77 MiB, with about two vCPUs in use. The observed new-round target was approximately -5093;
block eligibility is evaluated separately against the round target.
At qblock 1490, the coordinator recorded a 1.6 ms cancellation acknowledgement
and fresh results followed in the new round. No rejected work was observed.
The [observation record](data/live-observation.json) contains the sampled
energies, coordinator validation counters, round transitions, and resource
snapshot. Cancellation acknowledgement measures the control protocol;
it is not a solver-time or hardware-performance claim.
The observation ended at 18:20:56 UTC, with 31 validated results at the
latest coordinator counter sample and 11 new-round solves reported.

## Reproduce and inspect

Build and run commands are in [README.cpu-funnel.md](../../README.cpu-funnel.md).
The [harness](../../tools/cpu-funnel/benchmark.py) pins this coordinator image:
`registry.gitlab.com/quip.network/quip-solver/v0.3/quip-solver@sha256:4050700c2d00ab5ace4fbcc87ab59e3381054d939124ee39dac4a21284c30525`.
It runs with `--network none`, has no signer or live node data, and makes
no submissions. The `smoke` suite checks the protocol with small budgets;
use `screening` and `stock` to reproduce the measured suites.

Raw records and aggregates are in [data](data). Coordinator lease fields
such as `passed` and `winners` describe accounting, not threshold hits.
The analysis uses `raw_best_energy_milli`, actual wall time, and rejection
counts. Gates in the topology input are synthetic benchmark gates.

Validation passed: cached/canonical generation across 64 nonces, exact
original-energy scoring, monotonic polishing, family/plain session
conformance including stale leases and deadlines, all 19 Otsuka kernel
tests, Clippy with warnings denied, and formatting. Source, build settings,
and AGPL attribution are provided in this branch.
