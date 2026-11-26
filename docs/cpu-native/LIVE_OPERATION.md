# CPU-native live operation

The CPU-native solver produced **17 coordinator-validated salt results**
during the recorded live window and continued solving after an actual qblock
transition. The [native observation record](data/live-observation.json)
contains exact timestamps, counters, reported original energies, configuration,
and resource snapshots for this build.

## Deployment and measured window

| Item | Observed value |
|---|---|
| Activation | 21 November 2026, 05:29:22 UTC |
| Observation ended | 05:35:54 UTC |
| Solver process | `/data/cpu-native/bin/quip-cpu-native`, supervised by the existing coordinator |
| Native binary SHA-256 | `fe569da1d7ff5a460b7c118341305ac725c86ba647e57806e52afef25eb5b40c` |
| Implementation source | `27815b4548d7c5b4c917183d41ae651338fe2d42` |
| Coordinator | v0.3.9-rc10, protocol 2 |
| Coordinator image digest | `sha256:4050700c2d00ab5ace4fbcc87ab59e3381054d939124ee39dac4a21284c30525` |
| CPU placement | CPUs 0 and 1; two lease workers |
| Memory limit | 1 GiB for the whole CPU service |
| Solve budget | 16 reads × 8,192 sweeps; default β=12, polish β=20 |
| Execution options | Shared cache, field transfer, union inlining, adjacency fusion enabled |
| Candidate options | Archive and exact motif-domain scoring enabled; structural screening disabled |

The deployment preserved the coordinator image and operator key content.
Other running services retained their container identities. Compose was
validated before the CPU service was recreated. A private baseline snapshot
and CPU-service rollback script were retained.

## Candidate validation and resource evidence

| Measurement | Recorded result |
|---|---:|
| Coordinator `results_validated` counter | **17** |
| Solver salt-result reports | 18 |
| Reports after the qblock transition | 12 |
| Job rejection counter | **0** |
| Best reported original energy | **−5054.175** (`−5054175` milli units) |
| CPU service memory snapshots | 95.01 and 96.75 MiB |
| Docker CPU snapshots | 200.09% and 201.45%, approximately two vCPUs |
| Container restarts / OOM kills | **0 / 0** |

The periodic validation counter was sampled before the final reported result;
the raw record preserves this timing difference. Resource measurements cover
the coordinator and native solver together. They are operational snapshots,
separate from the paired offline performance studies.

The generic controller API reported four `duplicate_result_drops`, zero
stale drops, and zero submission errors. This metric includes outcomes for
unknown or already-closed jobs, including `LeaseDone` settlement; it is
separate from canonical candidate validation and job rejection. The
[pinned coordinator settlement code](https://github.com/QuipNetwork/quip-solver/blob/b60c27117ae00219c4a03a1f77c2a591bd34292f/crates/quip-coordinator/src/coordinator/settle.rs#L380)
defines that distinction. All controller counters are preserved in the record.

## Qblock transition and cancellation

The coordinator observed qblock **1576 → 1577**, advancing its generation
from 1 to 2. At 05:31:25 UTC, the miner acknowledged cancellation through
generation 1 in **1.4 ms**. Fresh native solver reports followed at 05:32:04 UTC,
and 12 post-transition reports are present in the observation.

Cancellation acknowledgement measures the control protocol. It is distinct
from a solver's sweep cost or device access time. The family/plain protocol
conformance suite separately checks expired deadlines, stale leases, canonical
witness validation, and balanced credits. The [final validation records](data/validation/)
cover 30 family-library tests, 21 kernel tests, and the native session test.

## Extended observation

The same deployed binary was observed through 09:43:22 UTC on 21 November.
The coordinator counter reached 715 validated salt results across 41 observed
rounds. The solver logged 717 salt results, with a best original energy of
−5060.025. The record shows zero job rejections, stale drops, proof submissions,
submission errors, restarts, and OOM kills. The endpoint resource snapshot
was 62.2 MiB and approximately two vCPUs. [Extended observation record](data/live-observation-extended.json).

## Reproduce the observation

The [read-only observer](../../tools/cpu-native/observe_node.py) collects Docker
logs, resource snapshots, and the coordinator API without reading a keystore:

```sh
python3 tools/cpu-native/observe_node.py --url https://YOUR_NODE_HOST --output /tmp/quip-native-observation.json --duration 600 --interval 30 --stop-after-results 16 --require-round-transition
```

Candidate validation, eligible proof submission, and an included winning proof
are separate events. This observation validates native operation and round
handling; the paired CPU speed and quality comparisons are in the
[experiment report](EXPERIMENTS.md).
