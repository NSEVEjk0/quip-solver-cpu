# Experimental CPU Funnel adaptation

`quip-cpu-funnel` adapts the selection pipeline and annealing ideas from
[BudFox / ntllinh2511's Funnel branch](https://github.com/ntllinh2511/quip-solver-cuda/tree/funnel)
to the CPU Otsuka kernel. It supports `quip/twisted-ice-dimer` revision 1
and declares CPU hardware. Other families and ordinary jobs use the existing
CPU solvers.

This adaptation uses CPU population annealing, configurable cluster weights,
weak-coupling bias, and a final polish against the original coefficients.
Optional selection screens a bounded batch, trials a few survivors, and
deep-solves the best trial. Pools are local to a lease; cancellation stops
screening, annealing, and polishing when the lease or round is abandoned.
Cached coupling generation is audited against the canonical generator every
64 salts and for every solved survivor.

Screening defaults to **off**. Its two structural models were trained for
GPU solves on an older recipe, and a small CPU comparison on the current
weak-45 / 1350-pair topology showed no clear benefit. CUDA kernel changes,
GPU deep lanes, and the GPU timing claims are outside this implementation.

## Build

```sh
cargo build --release --locked --features experimental --bin quip-cpu-funnel --bin quip-cpu-otsuka -j 2
```

The repository pins Rust 1.97.1. Launch through the existing Quip coordinator;
the binary implements its normal solver session protocol and has no signer.

## Example for a two-vCPU miner

Install the binary at the configured path, then use this backend table in
the operator's existing config. Keep the operator's other tables and keys.

From your node deployment directory, install the built binary:

```sh
mkdir -p data/cpu-funnel/bin
install -m 755 /path/to/quip-solver-cpu/target/release/quip-cpu-funnel data/cpu-funnel/bin/quip-cpu-funnel
```

```toml
[cpu]
binary = "/data/cpu-funnel/bin/quip-cpu-funnel"
num_cpus = 2
queue_depth = 2
funnel_screen = false
funnel_final_reads = 16
funnel_final_sweeps = 8192
funnel_tile_milli = 250
funnel_pair_milli = 250
funnel_weak_scale = 1.5
funnel_resamples = 512
funnel_houdayer_every = 0
funnel_polish_sweeps = 256
```

Restrict the miner container to two CPUs as well. Longer solves trade
latency for lower energy; this budget is an experimental starting point.
It does not establish that a CPU can beat current GPU miners.

For the `nodes.quip.network` Docker deployment, set `QUIP_MINER_CPUSET=0,1`
and `QUIP_MINER_MEM_LIMIT=1g` in its existing `.env`, then validate and
recreate the CPU service with `docker compose --profile cpu config --quiet`
and `docker compose --profile cpu up -d --no-deps --pull never cpu`.
The downloadable Linux x86_64 build requires glibc 2.34 or later and
the standard GNU runtime libraries. The measured coordinator image works.

## Optional screening

```toml
funnel_screen = true
funnel_batch = 128
funnel_keep = 4
funnel_probe_reads = 4
funnel_probe_sweeps = 1024
```

Each batch receives two structural filters and up to four short trial
solves. The best surviving trial gets a fresh longer solve. An already
qualifying trial is returned immediately. Every returned spin vector is
rescored with the canonical coefficients and family offset. Screening
statistics are advisory; a skipped candidate is not reported as solved.

The `funnel_*` backend knobs also have `QUIP_CPU_FUNNEL_*` environment
forms, such as `QUIP_CPU_FUNNEL_SCREEN=true`. Backend TOML overrides the
environment. Final read and sweep overrides default to absent, preserving
the incoming job budget. Other defaults are batch 128, keep 4, probe 4
reads / 1024 sweeps, weights 250 / 250, weak scale 1.5, resamples 512,
Houdayer at checkpoints, and polish 256 sweeps on the best four reads.
Weak bias uses integer milli coefficients: a scale of 1.5 maps +/-45
to +/-67 internally. Proof scoring always uses the original +/-45.

## Validation

The adaptation includes cached-generator comparisons across 64 nonces,
exact original-energy checks, polish monotonicity checks, and end-to-end
family/plain session conformance, including stale leases and deadlines.
The Otsuka tests check exact cluster energies, cancellation, and preservation
of stock trajectories when default population options are used.

Reproduce the [recorded measurements](docs/cpu-funnel/RESULTS.md) with Docker:

```sh
python3 tools/cpu-funnel/benchmark.py --suite smoke --cpus 0,1
python3 tools/cpu-funnel/benchmark.py --suite screening --cpus 0,1 --output screening-results
python3 tools/cpu-funnel/benchmark.py --suite stock --cpus 0,1 --output stock-results
```

The harness mounts source and binaries read-only, uses `--network none`,
and has no keystore. It pins the measured coordinator image by digest.
It needs two available CPUs and Docker access. The smoke suite checks the
protocol with small budgets; its output is not a performance measurement.

The SDK's generic CPU config parser may warn that `funnel_*` fields are
unknown. The custom sampler reads the original backend TOML and logs
`CPU Funnel configured` with the effective values. These warnings do not
prevent the custom settings from applying.

```sh
cargo test --release --locked --features experimental --lib families::funnel::tests -j 2
cargo test --release --locked --features experimental --test funnel_conformance --test otsuka_kernel -j 2
cargo clippy --locked --features experimental --lib --bin quip-cpu-funnel --test funnel_conformance --test otsuka_kernel -- -D warnings
cargo fmt --all --check
```

## Source and attribution

Base: [quip-solver-cpu](https://gitlab.com/quip.network/quip-solver-cpu),
revision `de51fd4fd31fef2f50ff3a343961796dbb5ca49f` (v0.3.6).
The CPU Otsuka kernel is upstream work; this patch adds its optional
population cadence and the CPU pipeline around it.

The imported `fast_dimer.rs`, `stage0.rs`, and `stage0v2.rs` come from
[Funnel revision 77bfd8210616174e2cd9325a77220f1e122adafd](https://github.com/ntllinh2511/quip-solver-cuda/commit/77bfd8210616174e2cd9325a77220f1e122adafd).
Their original explanatory comments and fitted models are retained.
See the original branch's [README](https://github.com/ntllinh2511/quip-solver-cuda/blob/funnel/docs/funnel/README.md)
and [experiments](https://github.com/ntllinh2511/quip-solver-cuda/blob/funnel/docs/funnel/EXPERIMENTS.md).

License: **AGPL-3.0-or-later**, the same as both source repositories.
See [LICENSE](LICENSE). Provide the corresponding source for a deployed
build, including this adaptation and its build instructions.
