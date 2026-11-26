# CPU solver findings and experiments

This log follows the implementation, parameter tests, and node investigation.
Entry numbers stay fixed so a result can be cited without depending on a
release date or a section title. The first six entries describe the initial
CPU implementation changes. Later entries record measurements, operating
decisions, and protocol checks.

The evaluated recipe is `quip/twisted-ice-dimer` revision 1: 4,560 nodes,
41,205 edges, weak magnitude 45, and 1,350 pairs. Energy units equal integer
milli units divided by 1,000; a lower energy is better. CPU timings come from
the harness clock. A coordinator-validated salt result is a candidate batch,
with block eligibility checked separately by the coordinator and runtime.

## Entry index

| Entry | Subject | Result or decision |
|---|---|---|
| [C01](#c01) | Packed adjacency fusion | Implemented; evaluated alone and in combination |
| [C02](#c02) | Monotone union-find specialization | Implemented; exact output matches |
| [C03](#c03) | Shared transition-probability cache | Implemented; exact scalar fallback |
| [C04](#c04) | Parent state and field transfer | Implemented; repeated field reconstruction removed |
| [C05](#c05) | Original-energy candidate archive | Improved two of eight held-out instances |
| [C06](#c06) | Exact motif-domain archive scoring | Same outputs; lower pipeline cost |
| [C07](#c07) | Execution interactions | Combined path measured separately from individual options |
| [C08](#c08) | Full-budget confirmation | 1.122× median speedup, eight exact output matches |
| [C09](#c09) | Temperature endpoint pilot | β=10 selected for further testing |
| [C10](#c10) | β=10 at the operational budget | Did not confirm; β=12 retained |
| [C11](#c11) | Colder search endpoints | No improvement in the two-instance pilot |
| [C12](#c12) | Colder original-objective polish | β=20 retained |
| [C13](#c13) | Cached generator admission | Canonical checks retained before reporting results |
| [C14](#c14) | Structural screening on CPU | No advantage established; disabled in the operating configuration |
| [C15](#c15) | Initial stock/tuned comparison | Deeper results at higher measured cost |
| [C16](#c16) | Longer CPU solves | More depth, with a substantial time cost |
| [C17](#c17) | CPU worker and configuration handling | Bounded workers; backend settings forwarded correctly |
| [C18](#c18) | Weak bias and temperature controls | Internal search stays within graph admission bounds |
| [C19](#c19) | Salt-dependent instances | Verified from the solver and validator contracts |
| [C20](#c20) | Round anchor and stale nonces | Previous-round nonce fails current binding |
| [C21](#c21) | Canonical acceptance gates | Energy, count, and diversity checked independently |
| [C22](#c22) | Device timing and CPU measurement | Host timing used for performance evidence |
| [C23](#c23) | Lease lifetime and cancellation | Deadline, generation, and credit checks pass |
| [C24](#c24) | Native live operation | Validated results and new-round work observed |
| [C25](#c25) | Service resources and isolation | CPU and memory limits checked; operator state preserved |
| [C26](#c26) | Controller counters | Validation, closed-job outcomes, and rejections distinguished |
| [C27](#c27) | Exactness tests and output digests | Move-level tests complement final-output hashing |
| [C28](#c28) | Contract and build versions | Evaluated dependency pins recorded explicitly |
| [C29](#c29) | Release and reproduction checks | Published artifacts match the tested code and inputs |
| [C30](#c30) | Extended native operation | 715 validations across 41 rounds; no errors or restarts |
| [C31](#c31) | 32-bit local fields | Field and snapshot payloads shrink; no speedup established |
| [C32](#c32) | 16-bit probability-cache stamps | Table payload shrinks; no speedup established |
| [C33](#c33) | Skip identity copies at resampling | 86% fewer field restores at the pilot budget |
| [C34](#c34) | Combined storage and resampling | All outputs matched; no speedup established |
| [C35](#c35) | Cluster-boundary field updates | 1.129× in the kernel pilot; full-pipeline speedup unconfirmed |

<a id="c01"></a>
## C01. Packed adjacency fusion

A cluster flip updates both neighboring local fields and residual component
energies. The original path walked two adjacency lists. A packed 64-bit entry
now carries the neighbor, full coupling, and residual coupling, so both updates
use one traversal. Rows are built lazily for each prepared graph.

The isolated eight-instance ablation measured 0.978× median speedup. The
combined execution path improved every paired run. Fusion is therefore kept
as part of the measured combination; the evidence does not establish it as
an independent speedup. [Design](TECHNICAL_REPORT.md#packed-adjacency-fusion),
[paired records](data/ablation-study/results.json).

<a id="c02"></a>
## C02. Monotone union-find specialization

Cluster construction repeatedly joins components. The specialized path
inlines path halving and joins the larger root beneath the smaller root.
This preserves component numbering, edge order, and random draws. It avoids
introducing a different union policy while changing execution cost.

The isolated median speedup was 1.028× over eight instances. Per-move and
population tests compare exact states with the scalar implementation.
[Design](TECHNICAL_REPORT.md#monotone-union-find-inlining),
[kernel checks](data/validation/otsuka_kernel-final.log).

<a id="c03"></a>
## C03. Shared transition-probability cache

Replicas at the same temperature repeatedly ask for probabilities at the
same integer energy changes. One temperature-local table now serves the
population. Entries are indexed without collisions; epoch tags invalidate
them when temperature changes. Odd or out-of-range changes use the original
scalar expression. Heat-bath branches and random draws remain unchanged.

At 8,192 slots, table payloads occupy 294,936 bytes. The isolated median
speedup was 1.033×. Hit, miss, fallback, and temperature counters are part of
the raw output. [Design](TECHNICAL_REPORT.md#temperature-local-probability-reuse),
[records](data/ablation-study/results.json).

<a id="c04"></a>
## C04. Parent state and local-field transfer

Resampling copies a state already maintained on the same immutable graph.
Its local fields are valid too. A reusable snapshot captures parent spins
and fields before any child is overwritten, then restores both together.
Repeated parents and ancestry cycles do not require adjacency reconstruction.

At 16 × 2,048, field rebuilds fell from 8,192 to 16. The 8,176 child field
copies avoided 673,784,160 coefficient visits. The isolated median speedup
was 1.094×. [Design](TECHNICAL_REPORT.md#exact-local-field-transfer-at-resampling),
[operation counts](data/ablation-study/results.json).

<a id="c05"></a>
## C05. Original-energy candidate archive

The search uses biased weak couplings, and resampling can discard an earlier
useful state. A lease-local archive scores replica record improvements on
the original objective, including cluster states before Metropolis. It keeps
the best observed original-energy candidate independently of ancestry.

Regular candidates retain their existing polish sequence. A distinct archive
candidate is refined with its own RNG stream afterward. On eight held-out
instances, the archive improved two results by 580 and 920 milli units and
preserved the reference best on the other six. It observes record states,
rather than every intermediate spin flip. [Design](TECHNICAL_REPORT.md#original-objective-candidate-preservation),
[confirmation](data/confirmation/results.json).

<a id="c06"></a>
## C06. Exact motif-domain archive scoring

For a uniform weak magnitude, tracked search energy and physical strong
motifs determine original energy exactly. If `S` is strong energy, `b` the
search weak magnitude, and `w` the original magnitude, the conversion is
`S + w * (E_search - S) / b`. Uniformity and divisibility are checked; other
cases use the full scorer. Returned candidates still receive full canonical
rescoring.

Eight paired pipeline runs had identical candidate outputs and a 1.085×
median speedup over full-edge archive scoring. This is a separate comparison
from C08. [Derivation](TECHNICAL_REPORT.md#motif-domain-original-energy-conversion),
[paired records](data/motif-confirmation/results.json).

<a id="c07"></a>
## C07. Execution changes interact

The kernel study used eight instances and five reference/variant pairs per
instance, for 40 pairs. With all four execution options enabled, median
speedup was 1.264× at 16 × 2,048. All paired candidate digests matched.
Individual options had smaller and more variable effects; fusion alone had
a small median slowdown. The selected combination is supported by its own
measurements. Individual ratios are not multiplied into a prediction.
[Study](EXPERIMENTS.md#profile-guided-execution-study).

<a id="c08"></a>
## C08. Full-budget execution confirmation

Eight held-out instances at 16 × 8,192 confirmed a 1.122221× median execution
speedup. The paired bootstrap 95% median interval was 1.103708–1.140505×.
Mean elapsed times were 43.649 seconds for the scalar reference and 39.293
seconds for native execution. All eight candidate output digests matched.

This result supports the operational-budget claim. The shorter kernel study
and this pipeline study time different regions at different budgets.
[Summary](data/confirmation/summary.json),
[measurement scope](EXPERIMENTS.md#hardware-model-and-measurement).

<a id="c09"></a>
## C09. Temperature endpoint pilot

Six instances at 16 × 2,048 tested geometric ladder endpoints 8, 10, 12,
16, 18, and 24 with original-objective polish at β=20. Mean best energy was
−5034.6467 at β=10 and −5026.4033 at β=12. The pilot selected β=10 for a
longer-budget check. It did not set the operating default.
[Pilot records](data/temperature-study/results.json).

<a id="c10"></a>
## C10. β=10 did not confirm at full budget

At 16 × 8,192 with the archive enabled, the eight held-out instances gave
mean best energy −5045.4613 at β=10 and −5049.1075 at β=12. The shorter pilot
did not transfer to this operating regime. β=12 remains the default.
[Confirmation](data/confirmation/results.json).

<a id="c11"></a>
## C11. Colder search endpoints

A two-instance archive-enabled pilot compared β=12 and 24 with colder
endpoints 48 and 64 at 16 × 2,048. With polish β=20, mean energies were
−5028.010, −5023.900, −5019.965, and −5018.115, respectively. The colder
settings did not improve this set and were not selected. The sample is too
small to establish a general temperature optimum.
[Records](data/thermal-pilot/results.json).

<a id="c12"></a>
## C12. Colder original-objective polish

The same two-instance pilot tested polish β=80 at search endpoints 48 and
64. Mean energy changed from −5019.965 to −5020.460 at endpoint 48, and
from −5018.115 to −5017.970 at endpoint 64. Neither matched the β=12 /
polish β=20 reference. Polish β=20 remains the operating setting.
[Records](data/thermal-pilot/results.json).

<a id="c13"></a>
## C13. Cached generation needs canonical admission

The retained cached generator is checked against the contract generator
every 64 salts and for every solved survivor. Couplings, edge order, and
tile metadata must agree. Tests cover 64 nonces and canonical energy scoring.
Generation failures and mismatches stop or skip work under the contract rules.

This is a validation boundary around a retained source component. Its
[provenance](../../ACKNOWLEDGMENTS.md) and the CPU integration checks are
documented separately. [Implementation](../../src/families/funnel/mod.rs),
[family tests](data/validation/family-library-final.log).

<a id="c14"></a>
## C14. Structural screening did not establish a CPU advantage

The initial study compared bounded screening with direct solves at equal
replica-sweep counts on three paired seeds. Mean round-best energies were
−5051.162 for screening and −5054.108 for direct solving. Mean times were
56.698 and 52.939 seconds. Screening also has generation and filter costs
outside the sweep budget. It remains available for experiments but is disabled
in the evaluated deployment. [Baseline study](../cpu-funnel/RESULTS.md#screening-at-equal-sweep-counts).

<a id="c15"></a>
## C15. Initial stock/tuned CPU comparison

The stock and tuned arms used the same two nonce instances from seed 725
at 16 × 8,192. Mean energy changed from −5033.700 to −5049.000, with elapsed
time increasing from 37.270 to 47.155 seconds. The tested configuration
combined search bias, population settings, and polishing; this experiment
does not isolate their individual effects. It established the initial tuned
reference used for subsequent CPU work. [Baseline records](../cpu-funnel/data/stock-comparison-results.json).

<a id="c16"></a>
## C16. Longer CPU solves buy depth at a time cost

On those same two instances, increasing tuned sweeps from 8,192 to 32,768
improved mean energy by 7.390 units, from −5049.000 to −5056.390. Elapsed
time rose from 47.155 to 180.438 seconds, a factor of 3.83. This is a small
sample of the depth/time tradeoff. The operating budget stays at 8,192 while
allocation across instances is investigated separately.
[Baseline study](../cpu-funnel/RESULTS.md#stock-and-longer-budget-comparison).

<a id="c17"></a>
## C17. CPU worker and backend configuration handling

Lease workers share a bounded slot pool. Waiting workers poll the lease stop
state; releasing a slot wakes waiting workers. The native backend translates
`native_*` settings into the population configuration and forwards only
`num_cpus` to the generic CPU parser. This removes duplicate unknown-field
warnings while retaining explicit backend options. The evaluated node uses
two CPU workers and queue depth two. [Adapter](../../src/families/native.rs),
[worker implementation](../../src/families/funnel/mod.rs).

<a id="c18"></a>
## C18. Search bias stays inside graph bounds

The current weak scale of 1.5 maps magnitude 45 to integer magnitude 67.
Biased weak values are clipped to ±999, so they cannot become unintended
strong couplings outside the admitted motifs. Original scoring uses the
unmodified instance. An optional weak-normalized temperature endpoint is
available for recipe tests; explicit job temperature ranges take precedence.
The release uses the measured β=12 default. [Preparation and schedule](../../src/families/funnel/mod.rs).

<a id="c19"></a>
## C19. Salt changes the instance

The nonce is `BLAKE3(last_proof_block_hash || miner_account || salt)`.
The family generator uses that nonce, so salts select different instances
on the same registered topology. A comparison of solver changes therefore
needs matched instances and seeds. The local harness constructs canonical
instances deterministically and makes no submissions.
[Pinned source anchors](NODE_RESEARCH.md#primary-source-anchors).

<a id="c20"></a>
## C20. The previous winning proof anchors the round

The validator derives the expected nonce from the persisted
`LastProofBlockHash`, signer, and salt. A nonce from the previous round fails
that binding after a new proof wins. Keeping the anchor in persistent state
also avoids losing it when old hashes leave the finite block-hash history.
[Runtime evidence](NODE_RESEARCH.md#nonce-and-round-binding).

<a id="c21"></a>
## C21. Canonical acceptance has several gates

The runtime resolves a registered mineable topology, reconstructs the family
instance, checks witnesses, and evaluates current difficulty. Energy must be
strictly below `max_energy_milli`; solution count and diversity are checked
independently. Locally validated candidates, submitted proofs, and winning
block inclusion are different events. Performance records identify which
event is counted. [Acceptance research](NODE_RESEARCH.md#candidate-validation-and-block-eligibility).

<a id="c22"></a>
## C22. Device time is observability

`device_access_time_us` is supplied by the miner and copied into its record.
It is not a consensus acceptance check in the reviewed runtime. The CPU
experiments use measured host wall time, operation counters, and profiling
samples. The baseline profile attributed 66.5% of sampled cycles to cluster
sweeps, 14.8% to union-find, 9.1% to Metropolis, and 3.14% to field reset.
[Timing research](NODE_RESEARCH.md#timing-and-measurement),
[profile](data/baseline-profile.txt).

<a id="c23"></a>
## C23. Lease lifetime and cancellation

One admitted lease consumes one credit. Completion, expiry, or covered
generation cancellation closes it under the session rules. Generation-zero
jobs survive generation cancellation. Native generation, replica sweeps, and
polishing check the lease stop state. Conformance covers stale leases,
deadlines, digest mismatches, and balanced credit accounting.
[Protocol research](NODE_RESEARCH.md#lease-lifetime-and-cancellation),
[native conformance](data/validation/native_conformance-final.log).

<a id="c24"></a>
## C24. Initial native live operation

The first recorded native window ran from 05:29:22 to 05:35:54 UTC on
21 November. The coordinator validation counter reached 17, with zero job
rejections. Eighteen salt results were reported by the solver. The coordinator
observed qblock 1576 → 1577; fresh work followed, with 12 post-transition
reports. Cancellation acknowledgement was 1.4 ms, measuring the control
protocol rather than kernel speed. [Observation](data/live-observation.json),
[operation report](LIVE_OPERATION.md).

<a id="c25"></a>
## C25. Service resources and isolation

The evaluated CPU service is pinned to CPUs 0 and 1 with a 1 GiB limit.
Initial measured memory snapshots were 95.01 and 96.75 MiB for the coordinator
and solver together. No restart or OOM kill occurred in that observation.
Operator key content was preserved, and other services retained their
container identities. A private configuration snapshot and CPU-only rollback
script remain available. [Operation report](LIVE_OPERATION.md#candidate-validation-and-resource-evidence).

<a id="c26"></a>
## C26. Controller counters have different meanings

The initial controller API reported four duplicate-result drops alongside
zero stale drops and zero submission errors. Its generic closed-job counter
also covers `LeaseDone` outcomes for unknown or already-completed jobs.
Canonical salt validation and job rejection use separate counters. Recording
all of them avoids treating a control outcome as an invalid energy result.
[Settlement source and recorded counters](LIVE_OPERATION.md#candidate-validation-and-resource-evidence).

<a id="c27"></a>
## C27. Exactness tests complement output hashes

The benchmark's `trajectory_digest` hashes returned spins and original
energies in order. It is a final-output digest. Separate tests compare each
cluster and Metropolis move, all 15 nonempty execution-option combinations,
canonical generation, archive non-regression, and motif/full-score parity.
The initial validation passed 30 family-library tests, 21 kernel tests, and
native family/plain session conformance.
[Validation logs](data/validation/), [test scope](EXPERIMENTS.md#correctness-and-protocol-checks).

<a id="c28"></a>
## C28. Contract and build versions matter

The evaluated build uses Rust 1.97.1, locked dependencies, solver-core
0.0.15, and protocol 2. The deployed coordinator is 0.3.9-rc10. Current
upstream documentation also describes 0.0.17 APIs; those were reviewed as
source context and are not part of this build's implementation claim.
[Build record](data/implementation-build.json),
[compatibility research](NODE_RESEARCH.md#compatibility-and-reproducibility).

<a id="c29"></a>
## C29. Reproduction and release checks

The public study runner records commands, CPU placement, input hashes,
original energies, and output digests. Release executables and corresponding
source are published with checksums. All downloaded assets matched their
prepared files. The packaged benchmark ran from the extracted source using
the committed inputs and reproduced its saved candidate digest. Study
comparisons remain separate: kernel, execution, archive quality, and archive
scoring measure different changes.
[Operator guide](OPERATOR_GUIDE.md),
[release verification](https://github.com/NSEVEjk0/quip-solver-cpu/releases/tag/cpu-native-2026-11-21).

<a id="c30"></a>
## C30. Extended native operation

The deployed binary continued from 05:29:22 to the observation at 09:43:22
UTC on 21 November. The coordinator validation counter reached 715; the
solver reported 717 salt results across 41 observed rounds. Job rejections,
error log lines, container restarts, OOM kills, stale drops, and submission
errors were all zero in the record. The best reported original energy was
−5060.025. The endpoint resource snapshot was 62.2 MiB and about two vCPUs.

The two candidate counts come from separate, periodically sampled counters.
The controller recorded zero proof submissions during this window.
[Extended record](data/live-observation-extended.json).

<a id="c31"></a>
## C31. 32-bit local fields

The admitted graph has distinct neighbors and `|J| ≤ 1000`, giving a maximum
absolute field of `2,097,151,000` milli units. The experimental build uses
32-bit fields and keeps move deltas and total energies in 64 bits. A high-degree
fixture exercises a move whose energy change exceeds the 32-bit range.

At 16 replicas, live field storage fell from 583,680 to 291,840 bytes and
snapshot payload fell from 656,640 to 364,800 bytes. Eight paired kernel runs
matched exactly, with a 0.964× median elapsed-time ratio.
[Records](data/storage-resampling/fields32/results.json).

<a id="c32"></a>
## C32. 16-bit probability-cache stamps

Cache stamps identify entries belonging to the current temperature. Using
16-bit stamps reduces the table payload from 294,936 to 245,780 bytes at
8,192 slots. On wrap, both stamp arrays clear before epoch 1 is reused.
The eight paired output digests matched. The median elapsed-time ratio was
0.962×. [Records](data/storage-resampling/stamps16/results.json).

<a id="c33"></a>
## C33. Skip identity copies at resampling

When resampling assigns a replica to itself, its spins and fields are
already in place. Capture only parents of children that change ancestry,
before restoring any child. Capturing first preserves repeated parents and
cycles among the assignments.

The eight runs required 1,055–1,269 field restores per run, down from 8,176.
The median run skipped 86% of restores. All candidate digests matched. The
median elapsed-time ratio was 1.010×, with timing variation on both sides of
parity. [Records](data/storage-resampling/selective/results.json).

<a id="c34"></a>
## C34. Combined storage and resampling

Testing both storage changes together gave a 0.974× median elapsed-time
ratio. Adding selective resampling gave 1.021×. Both variants matched all
eight reference digests and had timing intervals spanning parity. The build
features and the `native_selective_resampling` option remain opt-in.
[Paired comparisons](EXPERIMENTS.md#storage-and-resampling-pilots).

<a id="c35"></a>
## C35. Cluster-boundary field updates

When a cluster flips, both spins flip on each internal edge, so only boundary
edges change the component's total energy. Each member field is transformed
once from its old field and the contribution of its outside neighbors. The
update still visits every neighbor of an accepted member.

The kernel pilot gave a 1.129× median speedup on eight instances. At the
operational budget on eight fresh instances, the full pipeline gave 0.996×.
All 16 pairs returned the same candidate spins and energies. Keep this
alternative as an experimental build. [Derivation](TECHNICAL_REPORT.md#c35-cluster-boundary-field-updates),
[paired study](EXPERIMENTS.md#cluster-boundary-field-updates).
