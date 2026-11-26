# CPU-native build and operation

The binary runs through the standard Quip coordinator. It supports the
registered twisted-ice-dimer revision 1 family and retains the existing
CPU paths for other jobs. The evaluated build uses Rust 1.97.1 and the
dependency versions recorded in Cargo.lock.

## Build and install

```sh
cargo build --release --locked --features experimental --bin quip-cpu-native --example cpu_native_bench -j 2
```

From the node deployment directory:

```sh
mkdir -p data/cpu-native/bin
install -m 755 /path/to/quip-solver-cpu/target/release/quip-cpu-native data/cpu-native/bin/quip-cpu-native
```

The Linux x86_64 release binary requires glibc 2.34 or later and standard
GNU runtime libraries. It is accompanied by source and checksums.

## Evaluated backend configuration

```toml
[cpu]
binary = "/data/cpu-native/bin/quip-cpu-native"
num_cpus = 2
queue_depth = 2
native_screen = false
native_final_reads = 16
native_final_sweeps = 8192
native_tile_milli = 250
native_pair_milli = 250
native_weak_scale = 1.5
native_resamples = 512
native_houdayer_every = 0
native_shared_cache = true
native_cache_slots = 8192
native_copy_resampling = true
native_inline_union = true
native_fused_updates = true
native_archive = true
native_motif_score = true
native_polish_sweeps = 256
native_polish_beta = 20.0
```

For `nodes.quip.network`, the evaluated service uses
`QUIP_MINER_CPUSET=0,1` and `QUIP_MINER_MEM_LIMIT=1g`. Preserve the existing
operator identity and other configuration tables. After backing up the
current configuration, validate and recreate the CPU service:

```sh
docker compose --profile cpu config --quiet
docker compose --profile cpu up -d --no-deps --pull never --force-recreate cpu
```

The binary's effective configuration is logged on startup. `native_*` backend
keys override `QUIP_CPU_NATIVE_*` environment settings. Final read and sweep
overrides can be omitted to use the job budget. The four execution options
are independently switchable for ablations; candidate preservation and
temperature settings are evaluated separately.

### Optional tree polishing

Build the native solver with `tree-polish` in addition to the selected
execution features. Enable it with `native_tree_polish = true` in backend
TOML or set `QUIP_CPU_NATIVE_TREE_POLISH=true`. The default is off. The
cyclic-block variant uses the `cyclic-polish` build feature and the same
runtime setting. [Candidate quality measurements](EXPERIMENTS.md#full-budget-candidate-quality).

### Optional pair-aware worm polishing

Build the native solver with `pair-worm-polish` and enable it with
`native_pair_worm_polish = true` in backend TOML or
`QUIP_CPU_NATIVE_PAIR_WORM_POLISH=true`. Set `native_polish_sweeps = 256`
to use the evaluated regular polish before the final downhill pass. The
reported study enables the worm by itself. Tree polishing can be enabled
independently in binaries that include its feature.
[Matched measurements](EXPERIMENTS.md#canonical-energy-and-runtime-comparisons).

## Reproduce the kernel and pipeline studies

Run experiments on available CPUs, with the complete recorded inputs:

```sh
taskset -c 3 target/release/examples/cpu_native_bench --mode kernel --preset baseline --seed 1001 --reads 16 --sweeps 2048
taskset -c 3 target/release/examples/cpu_native_bench --mode kernel --preset core --seed 1001 --reads 16 --sweeps 2048
taskset -c 3 target/release/examples/cpu_native_bench --mode pipeline --preset archive --seed 1021 --reads 16 --sweeps 8192
```

For the complete paired suites, choose two available CPU IDs and a new
output directory for each run:

```sh
python3 tools/cpu-native/run_study.py --suite ablation --cpus 3 4 --output /tmp/quip-ablation
python3 tools/cpu-native/run_study.py --suite temperature --cpus 3 4 --output /tmp/quip-temperature
python3 tools/cpu-native/run_study.py --suite confirmation --cpus 3 4 --output /tmp/quip-confirmation
python3 tools/cpu-native/run_study.py --suite motif --cpus 3 4 --output /tmp/quip-motif
```

The runner records build and input hashes, CPU placement, commands, individual
results, and summaries. `--instances`, `--reads`, and `--sweeps` allow shorter
smoke runs. Confirmation retains full-edge archive scoring to reproduce the
historical experiment. The motif suite uses the same executable with and
without `--disable-motif-score`; its output digests must match.

The standalone harness constructs canonical instances locally and produces
JSON records. It performs no chain queries or submissions. Records include
trajectory digests, original energies, elapsed time, and operation counters.
Paired execution runs must match trajectory digests. Search variants use
original-energy comparisons on the same instances.

Regenerate the published SVG and PNG figures with:

```sh
python3 -m venv /tmp/quip-figures-env
/tmp/quip-figures-env/bin/pip install -r tools/cpu-native/requirements-figures.txt
/tmp/quip-figures-env/bin/python tools/cpu-native/plot_results.py
```

## Checks and operation

```sh
cargo test --release --locked --features experimental --lib families:: -j 2
cargo test --release --locked --features experimental --test otsuka_kernel --test native_conformance -j 2
cargo clippy --locked --features experimental --lib --bin quip-cpu-native --example cpu_native_bench --test otsuka_kernel --test native_conformance -- -D warnings
cargo fmt --all --check
```

Confirm coordinator validation counters, job rejection counts, affinity,
memory use, and work after a qblock transition. Canonical energy includes
the family offset in lease results. Block eligibility is evaluated against
the live target, solution count, and diversity requirements.

The [live observer](../../tools/cpu-native/observe_node.py) captures those
measurements from Docker and the coordinator's HTTPS API. Its results are
documented in [live operation](LIVE_OPERATION.md).

Rollback restores the prior CPU binary and configuration from the operator's
saved deployment snapshot. The research deployment retains a private backup
and a CPU-only rollback script.
