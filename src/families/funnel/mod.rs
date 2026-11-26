//! CPU population-solver orchestration and canonical result handling.
//!
//! Each worker processes a bounded lease-local candidate set and polishes
//! against original coefficients. Legacy structural screen modules retain
//! their AGPL source; provenance is recorded in ACKNOWLEDGMENTS.md.
//! CPU-native execution and candidate-preservation paths are independently
//! configurable. No lease or candidate archive outlives its round.

mod fast_dimer;
#[expect(
    clippy::allow_attributes,
    reason = "retain the imported screen's original lint annotations"
)]
mod stage0;
#[expect(
    clippy::allow_attributes,
    reason = "retain the imported screen's original lint annotations"
)]
mod stage0v2;

#[cfg(feature = "tree-polish")]
use super::otsuka::TreePolish;
use super::otsuka::{
    anneal_population_observed, AnnealError, Chain, Model, Motifs, OtsukaGraph, PopulationOptions,
    Schedule, SweepCache, Weights,
};
use super::otsuka_sampler::{parse_pairs, section, PAIRS_SECTION};
#[cfg(feature = "pair-worm-polish")]
use crate::postprocess::{
    greedy::{DescentError, Ising},
    pair_worm::{PairWorm, WormOptions},
};
use crate::sampler_core::{
    build_beta_schedule, effective_sweeps_per_beta, sample_ising_cancellable,
};
use crate::{OtsukaSampler, CPU_OTSUKA_IDENTITY};
use quip_solver_core::families::{Declaration, FamilyClaim, FamilyContext};
use quip_solver_core::ising_generators::family::SaltFault;
use quip_solver_core::ising_generators::ice::twisted_dimer::local_gauges;
use quip_solver_core::ising_generators::ice::TWISTED_DIMER_V1;
use quip_solver_core::ising_generators::instance::Instance;
use quip_solver_core::{
    BackendIdentity, CancelToken, IsingGraph, Lease, LeaseSink, SampleError, SampleParams, Sampler,
    SamplerResult, StreamResult, TopologyView, WarmStart, WarmStreamJob,
};
use rand::rngs::SmallRng;
use rand::SeedableRng;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Identity of the CPU-only staged solver. Hardware remains declared as CPU.
pub const CPU_FUNNEL_IDENTITY: BackendIdentity = CPU_OTSUKA_IDENTITY;
const CLAIMS: &[FamilyClaim] = &[FamilyClaim {
    revisions: &[TWISTED_DIMER_V1],
    only_params: &[],
}];

/// Bounded CPU budgets, in backend TOML or `QUIP_CPU_FUNNEL_*` variables.
#[derive(Clone, Debug)]
pub struct FunnelConfig {
    /// Enable the two structural screens and short trial selection.
    pub screen: bool,
    /// Maximum salts considered before a trial group.
    pub batch: usize,
    /// Candidates receiving a short trial after both structural screens.
    pub keep: usize,
    /// Replicas of each short trial.
    pub probe_reads: usize,
    /// Sweeps of each short trial.
    pub probe_sweeps: usize,
    /// Override final replicas; absent uses the job's read budget.
    pub final_reads: Option<usize>,
    /// Override final sweeps; absent uses the job's sweep budget.
    pub final_sweeps: Option<usize>,
    /// Tile strength assigned to auxiliary cluster bonds, in milli.
    pub tile_milli: i32,
    /// Dimer strength assigned to auxiliary cluster bonds, in milli.
    pub pair_milli: i32,
    /// Heuristic multiplier of weak couplings during annealing only.
    /// Internal i32 milli values truncate once; proof scoring is unscaled.
    pub weak_scale: f64,
    /// Population resampling checkpoints.
    pub resamples: usize,
    /// Houdayer cadence; zero means checkpoints only.
    pub houdayer_every: usize,
    /// Cold cluster continuation sweeps on the four best reads.
    pub polish_sweeps: usize,
    /// Metropolis cadence during the population phase. Zero uses clusters alone.
    pub single_every: usize,
    /// Enable conditional tree descent on the completed candidate batch.
    #[cfg(feature = "tree-polish")]
    pub tree_polish: bool,
    /// Finish the best reads with downhill, pair-aware collective moves.
    #[cfg(feature = "pair-worm-polish")]
    pub pair_worm_polish: bool,
    /// Share temperature-local probabilities across replicas.
    pub shared_cache: bool,
    /// Positive half-delta capacity of the shared probability table.
    pub cache_slots: usize,
    /// Copy parent local fields at population resampling checkpoints.
    pub copy_resampling: bool,
    /// Skip identity children and snapshot only required parent rows.
    pub selective_resampling: bool,
    /// Inline the monotone union-find path.
    pub inline_union: bool,
    /// Fuse full-field and residual-delta adjacency updates.
    pub fused_updates: bool,
    /// Preserve original-energy record candidates outside replica ancestry.
    pub archive: bool,
    /// Use exact motif-domain energy conversion for archive evaluation.
    pub motif_score: bool,
    /// Optional final beta measured in units of the biased weak coupling.
    pub weak_beta_end: Option<f64>,
    /// Inverse temperature of original-objective cluster polishing.
    pub polish_beta: f64,
}

impl Default for FunnelConfig {
    fn default() -> Self {
        Self {
            screen: false,
            batch: 128,
            keep: 4,
            probe_reads: 4,
            probe_sweeps: 1024,
            final_reads: None,
            final_sweeps: None,
            tile_milli: 250,
            pair_milli: 250,
            weak_scale: 1.5,
            resamples: 512,
            houdayer_every: 0,
            polish_sweeps: 256,
            single_every: 1,
            #[cfg(feature = "tree-polish")]
            tree_polish: false,
            #[cfg(feature = "pair-worm-polish")]
            pair_worm_polish: false,
            shared_cache: false,
            cache_slots: 8192,
            copy_resampling: false,
            selective_resampling: false,
            inline_union: false,
            fused_updates: false,
            archive: false,
            motif_score: true,
            weak_beta_end: None,
            polish_beta: 20.0,
        }
    }
}

impl FunnelConfig {
    fn read(table: &toml::Table, key: &str, default: usize, max: usize) -> Result<usize, String> {
        let Some(value) = table.get(key) else {
            return Ok(default);
        };
        let number = value.as_integer().and_then(|n| usize::try_from(n).ok());
        number
            .filter(|n| (1..=max).contains(n))
            .ok_or_else(|| format!("{key} must be in 1..={max}"))
    }

    fn apply(&mut self, table: &toml::Table) -> Result<(), String> {
        if let Some(v) = table.get("funnel_screen") {
            self.screen = v.as_bool().ok_or("funnel_screen must be boolean")?;
        }
        for (key, value) in [
            ("funnel_shared_cache", &mut self.shared_cache),
            ("funnel_copy_resampling", &mut self.copy_resampling),
            (
                "funnel_selective_resampling",
                &mut self.selective_resampling,
            ),
            ("funnel_inline_union", &mut self.inline_union),
            ("funnel_fused_updates", &mut self.fused_updates),
            ("funnel_archive", &mut self.archive),
            ("funnel_motif_score", &mut self.motif_score),
        ] {
            if let Some(v) = table.get(key) {
                *value = v
                    .as_bool()
                    .ok_or_else(|| format!("{key} must be boolean"))?;
            }
        }
        self.cache_slots = Self::read(table, "funnel_cache_slots", self.cache_slots, 65_536)?;
        if let Some(v) = table.get("funnel_weak_beta_end") {
            self.weak_beta_end = Some(
                v.as_float()
                    .or_else(|| v.as_integer().map(|n| n as f64))
                    .filter(|n| n.is_finite() && (0.1..=8.0).contains(n))
                    .ok_or("funnel_weak_beta_end must be in 0.1..=8")?,
            );
        }
        if let Some(v) = table.get("funnel_polish_beta") {
            self.polish_beta = v
                .as_float()
                .or_else(|| v.as_integer().map(|n| n as f64))
                .filter(|n| n.is_finite() && (1.0..=256.0).contains(n))
                .ok_or("funnel_polish_beta must be in 1..=256")?;
        }
        self.batch = Self::read(table, "funnel_batch", self.batch, 4096)?;
        self.keep = Self::read(table, "funnel_keep", self.keep, 64)?.min(self.batch);
        self.probe_reads = Self::read(table, "funnel_probe_reads", self.probe_reads, 128)?;
        self.probe_sweeps = Self::read(table, "funnel_probe_sweeps", self.probe_sweeps, 262_144)?;
        if table.contains_key("funnel_final_reads") {
            self.final_reads = Some(Self::read(table, "funnel_final_reads", 16, 128)?);
        }
        if table.contains_key("funnel_final_sweeps") {
            self.final_sweeps = Some(Self::read(table, "funnel_final_sweeps", 16_384, 262_144)?);
        }
        self.tile_milli = i32::try_from(Self::read(
            table,
            "funnel_tile_milli",
            self.tile_milli as usize,
            1000,
        )?)
        .map_err(|e| e.to_string())?;
        self.pair_milli = i32::try_from(Self::read(
            table,
            "funnel_pair_milli",
            self.pair_milli as usize,
            1000,
        )?)
        .map_err(|e| e.to_string())?;
        self.resamples = Self::read(table, "funnel_resamples", self.resamples, 4096)?;
        if let Some(v) = table.get("funnel_houdayer_every") {
            self.houdayer_every = v
                .as_integer()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n <= 262_144)
                .ok_or("invalid funnel_houdayer_every")?;
        }
        if let Some(v) = table.get("funnel_polish_sweeps") {
            self.polish_sweeps = v
                .as_integer()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n <= 8192)
                .ok_or("invalid funnel_polish_sweeps")?;
        }
        if let Some(v) = table.get("funnel_single_every") {
            self.single_every = v
                .as_integer()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n <= 262_144)
                .ok_or("invalid funnel_single_every")?;
        }
        #[cfg(feature = "tree-polish")]
        if let Some(v) = table.get("funnel_tree_polish") {
            self.tree_polish = v.as_bool().ok_or("funnel_tree_polish must be a boolean")?;
        }
        #[cfg(feature = "pair-worm-polish")]
        if let Some(v) = table.get("funnel_pair_worm_polish") {
            self.pair_worm_polish = v
                .as_bool()
                .ok_or("funnel_pair_worm_polish must be a boolean")?;
        }
        if let Some(v) = table.get("funnel_weak_scale") {
            self.weak_scale = v
                .as_float()
                .or_else(|| v.as_integer().map(|n| n as f64))
                .filter(|n| n.is_finite() && (1.0..=1.5).contains(n))
                .ok_or("funnel_weak_scale must be in 1..=1.5")?;
        }
        Ok(())
    }

    fn from_environment() -> Self {
        Self::from_prefixed_environment("QUIP_CPU_FUNNEL", Self::default())
    }

    fn from_prefixed_environment(prefix: &str, mut config: Self) -> Self {
        let mut table = toml::Table::new();
        for key in [
            "screen",
            "batch",
            "keep",
            "probe_reads",
            "probe_sweeps",
            "final_reads",
            "final_sweeps",
            "tile_milli",
            "pair_milli",
            "weak_scale",
            "resamples",
            "houdayer_every",
            "polish_sweeps",
            "single_every",
            #[cfg(feature = "tree-polish")]
            "tree_polish",
            #[cfg(feature = "pair-worm-polish")]
            "pair_worm_polish",
            "shared_cache",
            "cache_slots",
            "copy_resampling",
            "selective_resampling",
            "inline_union",
            "fused_updates",
            "archive",
            "motif_score",
            "weak_beta_end",
            "polish_beta",
        ] {
            let Ok(raw) = std::env::var(format!("{prefix}_{}", key.to_uppercase())) else {
                continue;
            };
            let value = match key {
                #[cfg(feature = "tree-polish")]
                "tree_polish" => raw.parse::<bool>().ok().map(toml::Value::Boolean),
                #[cfg(feature = "pair-worm-polish")]
                "pair_worm_polish" => raw.parse::<bool>().ok().map(toml::Value::Boolean),
                "screen"
                | "shared_cache"
                | "copy_resampling"
                | "selective_resampling"
                | "inline_union"
                | "fused_updates"
                | "archive"
                | "motif_score" => raw.parse::<bool>().ok().map(toml::Value::Boolean),
                "weak_scale" | "weak_beta_end" | "polish_beta" => {
                    raw.parse::<f64>().ok().map(toml::Value::Float)
                }
                _ => raw.parse::<i64>().ok().map(toml::Value::Integer),
            };
            if let Some(value) = value {
                table.insert(format!("funnel_{key}"), value);
            }
        }
        if let Err(error) = config.apply(&table) {
            tracing::error!(%error, "invalid CPU Funnel environment; using defaults");
            return Self::default();
        }
        config
    }
}

struct Slots {
    active: Mutex<usize>,
    changed: Condvar,
}

struct Slot<'a>(&'a Slots);
impl Drop for Slot<'_> {
    fn drop(&mut self) {
        *self.0.active.lock().unwrap_or_else(PoisonError::into_inner) -= 1;
        self.0.changed.notify_all();
    }
}

/// CPU sampler with bounded concurrent lease workers.
pub struct FunnelSampler {
    stock: OtsukaSampler,
    num_cpus: Arc<AtomicUsize>,
    config: Mutex<FunnelConfig>,
    slots: Slots,
}

impl Default for FunnelSampler {
    fn default() -> Self {
        Self {
            stock: OtsukaSampler::new(),
            num_cpus: Arc::new(AtomicUsize::new(2)),
            config: Mutex::new(FunnelConfig::from_environment()),
            slots: Slots {
                active: Mutex::new(0),
                changed: Condvar::new(),
            },
        }
    }
}

impl FunnelSampler {
    /// Create the solver; backend TOML overrides its environment defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A CPU-native configuration with independent execution optimizations.
    #[must_use]
    pub fn native() -> Self {
        let sampler = Self::default();
        let mut config = sampler
            .config
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *config = FunnelConfig::from_prefixed_environment(
            "QUIP_CPU_NATIVE",
            FunnelConfig {
                shared_cache: true,
                copy_resampling: true,
                inline_union: true,
                fused_updates: true,
                archive: true,
                ..FunnelConfig::default()
            },
        );
        drop(config);
        sampler
    }

    fn acquire(&self, sink: &LeaseSink) -> Option<Slot<'_>> {
        let mut active = self
            .slots
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        loop {
            if sink.is_stopped() {
                return None;
            }
            if *active < self.stream_width() {
                *active += 1;
                return Some(Slot(&self.slots));
            }
            active = self
                .slots
                .changed
                .wait_timeout(active, Duration::from_millis(5))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// Exact original energy, with the family offset. Useful to independently
/// check every returned spin vector, including weak-biased anneals.
#[must_use]
pub fn instance_energy(instance: &Instance, spins: &[i8]) -> i64 {
    instance
        .h()
        .iter()
        .zip(spins)
        .map(|(&h, &s)| i64::from(h) * i64::from(s))
        .sum::<i64>()
        + instance
            .edges()
            .iter()
            .zip(instance.j())
            .map(|(&(u, v), &j)| {
                i64::from(j) * i64::from(spins[u as usize]) * i64::from(spins[v as usize])
            })
            .sum::<i64>()
        + instance.offset_milli()
}

fn graph_of(instance: &Instance) -> IsingGraph {
    IsingGraph::new(
        instance
            .h()
            .iter()
            .map(|&v| f64::from(v) / 1000.0)
            .collect(),
        instance
            .j()
            .iter()
            .map(|&v| f64::from(v) / 1000.0)
            .collect(),
        instance
            .edges()
            .iter()
            .map(|&(u, v)| (u as usize, v as usize))
            .collect(),
    )
}

/// Prepare a CPU graph from a canonical instance and explicit search weights.
/// `bias` scales weak couplings in the internal search model only.
///
/// # Errors
/// Returns capacity when the metadata or weights do not fit the kernel.
pub fn prepared(
    instance: &Instance,
    config: &FunnelConfig,
    bias: bool,
) -> Result<OtsukaGraph, SampleError> {
    let (tiles, gauges) = local_gauges(instance).ok_or(SampleError::Capacity)?;
    let pairs =
        parse_pairs(section(instance.metadata(), PAIRS_SECTION).ok_or(SampleError::Capacity)?)
            .ok_or(SampleError::Capacity)?;
    let j: Vec<i32> = instance
        .j()
        .iter()
        .map(|&v| {
            if bias && v.abs() < 1000 {
                ((f64::from(v) * config.weak_scale) as i32).clamp(-999, 999)
            } else {
                v
            }
        })
        .collect();
    OtsukaGraph::new(
        &Model {
            h_milli: instance.h(),
            edges: instance.edges(),
            j_milli: &j,
        },
        &Motifs {
            tiles: &tiles,
            gauges: &gauges,
            pairs: &pairs,
        },
        Weights {
            tile_milli: config.tile_milli,
            pair_milli: config.pair_milli,
        },
    )
    .map_err(|_| SampleError::Capacity)
}

fn greedy(instance: &Instance, spins: &mut [i8], stopped: &impl Fn() -> bool) -> bool {
    let n = spins.len();
    let mut adj = vec![Vec::new(); n];
    for (&(u, v), &j) in instance.edges().iter().zip(instance.j()) {
        adj[u as usize].push((v as usize, i64::from(j)));
        adj[v as usize].push((u as usize, i64::from(j)));
    }
    let mut fields: Vec<i64> = (0..n)
        .map(|u| {
            i64::from(instance.h()[u])
                + adj[u]
                    .iter()
                    .map(|&(v, j)| j * i64::from(spins[v]))
                    .sum::<i64>()
        })
        .collect();
    loop {
        let mut changed = false;
        for u in 0..n {
            if u.is_multiple_of(256) && stopped() {
                return false;
            }
            let old = i64::from(spins[u]);
            if -2 * old * fields[u] < 0 {
                spins[u] = -spins[u];
                for &(v, j) in &adj[u] {
                    fields[v] -= 2 * old * j;
                }
                changed = true;
            }
        }
        if !changed {
            return true;
        }
    }
}

/// Run the tuned CPU kernel on a canonical instance. Returns `None` on
/// cancellation. Energies exclude the offset, as `Sampler::sample_family`
/// requires; callers managing a lease add it before pushing the salt.
///
/// # Errors
/// Returns capacity when the motifs or budget do not fit this kernel.
pub fn tuned_reads(
    instance: &Instance,
    params: &SampleParams,
    config: &FunnelConfig,
    stopped: impl Fn() -> bool,
) -> Result<Option<Vec<SamplerResult>>, SampleError> {
    if stopped() {
        return Ok(None);
    }
    if params.num_reads == 0 || params.num_reads > 128 {
        return Err(SampleError::Capacity);
    }
    let biased = prepared(instance, config, true)?;
    let truth = prepared(instance, config, false)?;
    let graph = graph_of(instance);
    let params = SampleParams {
        beta_range: Some(params.beta_range.unwrap_or_else(|| {
            let weak = instance
                .j()
                .iter()
                .map(|v| v.abs())
                .filter(|&v| v > 0 && v < 1000)
                .max();
            match (config.weak_beta_end, weak) {
                (Some(target), Some(weak)) => {
                    let biased_weak = ((f64::from(weak) * config.weak_scale) as i32).min(999);
                    (
                        1.0,
                        (target * 1000.0 / f64::from(biased_weak)).clamp(1.0, 128.0),
                    )
                }
                _ => (1.0, 12.0),
            }
        })),
        ..params.clone()
    };
    let per_beta = effective_sweeps_per_beta(params.num_sweeps, params.sweeps_per_beta);
    let betas: Vec<f64> = build_beta_schedule(&graph, &params)
        .into_iter()
        .flat_map(|beta| std::iter::repeat_n(beta, per_beta))
        .collect();
    let schedule = Schedule::new(&betas, config.single_every).map_err(|_| SampleError::Capacity)?;
    let weak_levels = config
        .motif_score
        .then(|| uniform_weak_levels(instance, config))
        .flatten();
    let mut archive: Option<SamplerResult> = None;
    let mut last_scored: Option<SamplerResult> = None;
    let outcome = match anneal_population_observed(
        &biased,
        &schedule,
        params.num_reads,
        params.seed,
        PopulationOptions {
            resamples: config.resamples,
            houdayer_every: config.houdayer_every,
            shared_cache: config.shared_cache,
            cache_slots: config.cache_slots,
            copy_resampling: config.copy_resampling,
            selective_resampling: config.selective_resampling,
            inline_union: config.inline_union,
            fused_updates: config.fused_updates,
        },
        &stopped,
        |biased_energy, spins| {
            if !config.archive {
                return;
            }
            let energy_milli = if let Some(previous) = &last_scored {
                if previous.spins == spins {
                    previous.energy_milli
                } else {
                    original_record_energy(&biased, &truth, weak_levels, biased_energy, spins)
                }
            } else {
                original_record_energy(&biased, &truth, weak_levels, biased_energy, spins)
            };
            if last_scored.as_ref().is_none_or(|r| r.spins != spins) {
                last_scored = Some(SamplerResult {
                    spins: spins.to_vec(),
                    energy_milli,
                });
            }
            if archive
                .as_ref()
                .is_none_or(|r| energy_milli < r.energy_milli)
            {
                archive = Some(SamplerResult {
                    spins: spins.to_vec(),
                    energy_milli,
                });
            }
        },
    ) {
        Ok(outcome) => outcome,
        Err(AnnealError::Cancelled) => return Ok(None),
    };
    tracing::debug!(metrics=?outcome.metrics, "CPU native population metrics");
    let mut reads: Vec<SamplerResult> = outcome
        .samples
        .into_iter()
        .map(|sample| {
            let energy_milli = truth.energy(&sample.spins);
            SamplerResult {
                spins: sample.spins,
                energy_milli,
            }
        })
        .collect();
    if config.polish_sweeps == 0 {
        if let Some(record) = archive {
            retain_record(&mut reads, record, params.num_reads);
        }
        #[cfg(feature = "tree-polish")]
        if config.tree_polish && !polish_trees(&truth, &mut reads, params.seed, &stopped) {
            return Ok(None);
        }
        #[cfg(feature = "pair-worm-polish")]
        if config.pair_worm_polish
            && !polish_pair_worm(instance, &mut reads, params.seed, &stopped)?
        {
            return Ok(None);
        }
        return Ok(Some(reads));
    }
    for read in &mut reads {
        if !greedy(instance, &mut read.spins, &stopped) {
            return Ok(None);
        }
        read.energy_milli = truth.energy(&read.spins);
    }
    let mut order: Vec<usize> = (0..reads.len()).collect();
    order.sort_unstable_by_key(|&i| reads[i].energy_milli);
    let mut rng = SmallRng::seed_from_u64(params.seed ^ 0x713c_0145_504f_4c49);
    let mut polish_cache = SweepCache::new(config.cache_slots);
    for i in order.into_iter().take(4) {
        let read = &mut reads[i];
        let mut chain =
            Chain::new(&truth, read.spins.clone()).map_err(|_| SampleError::Capacity)?;
        let mut energy = read.energy_milli;
        for _ in 0..config.polish_sweeps {
            if stopped() {
                return Ok(None);
            }
            energy += chain.cluster_sweep_optimized(
                config.polish_beta,
                config.shared_cache.then_some(&mut polish_cache),
                config.inline_union,
                config.fused_updates,
                &mut rng,
            );
            energy += if config.shared_cache {
                chain.single_sweep_cached(config.polish_beta, &mut polish_cache, &mut rng)
            } else {
                chain.single_sweep(config.polish_beta, &mut rng)
            };
            if energy < read.energy_milli {
                read.spins.copy_from_slice(chain.spins());
                read.energy_milli = energy;
            }
        }
        if !greedy(instance, &mut read.spins, &stopped) {
            return Ok(None);
        }
        read.energy_milli = truth.energy(&read.spins);
    }
    if let Some(mut record) = archive {
        if !reads.iter().any(|r| r.spins == record.spins) {
            if !greedy(instance, &mut record.spins, &stopped) {
                return Ok(None);
            }
            record.energy_milli = truth.energy(&record.spins);
            let mut chain =
                Chain::new(&truth, record.spins.clone()).map_err(|_| SampleError::Capacity)?;
            let mut rng = SmallRng::seed_from_u64(params.seed ^ 0x4350_555f_4152_4348);
            let mut energy = record.energy_milli;
            let mut cache = SweepCache::new(config.cache_slots);
            for _ in 0..config.polish_sweeps {
                if stopped() {
                    return Ok(None);
                }
                energy += chain.cluster_sweep_optimized(
                    config.polish_beta,
                    config.shared_cache.then_some(&mut cache),
                    config.inline_union,
                    config.fused_updates,
                    &mut rng,
                );
                energy += if config.shared_cache {
                    chain.single_sweep_cached(config.polish_beta, &mut cache, &mut rng)
                } else {
                    chain.single_sweep(config.polish_beta, &mut rng)
                };
                if energy < record.energy_milli {
                    record.spins.copy_from_slice(chain.spins());
                    record.energy_milli = energy;
                }
            }
            if !greedy(instance, &mut record.spins, &stopped) {
                return Ok(None);
            }
            record.energy_milli = truth.energy(&record.spins);
        }
        retain_record(&mut reads, record, params.num_reads);
    }
    #[cfg(feature = "tree-polish")]
    if config.tree_polish && !polish_trees(&truth, &mut reads, params.seed, &stopped) {
        return Ok(None);
    }
    #[cfg(feature = "pair-worm-polish")]
    if config.pair_worm_polish && !polish_pair_worm(instance, &mut reads, params.seed, &stopped)? {
        return Ok(None);
    }
    Ok(Some(reads))
}

#[cfg(feature = "pair-worm-polish")]
fn polish_pair_worm(
    instance: &Instance,
    reads: &mut [SamplerResult],
    seed: u64,
    stopped: &impl Fn() -> bool,
) -> Result<bool, SampleError> {
    let (tiles, _) = local_gauges(instance).ok_or(SampleError::Capacity)?;
    let pairs =
        parse_pairs(section(instance.metadata(), PAIRS_SECTION).ok_or(SampleError::Capacity)?)
            .ok_or(SampleError::Capacity)?;
    let h: Vec<_> = instance.h().iter().map(|&value| i64::from(value)).collect();
    let j: Vec<_> = instance.j().iter().map(|&value| i64::from(value)).collect();
    let ising = Ising::new(&h, instance.edges(), &j).map_err(|_| SampleError::Capacity)?;
    let worm = PairWorm::new(&ising, &tiles, &pairs).map_err(|_| SampleError::Capacity)?;
    let mut order: Vec<_> = (0..reads.len()).collect();
    order.sort_by_key(|&index| reads[index].energy_milli);
    let settings = WormOptions {
        max_sweeps: 8,
        max_len: 64,
        ..WormOptions::default()
    };
    for index in order.into_iter().take(8) {
        let read = &mut reads[index];
        let local_seed = seed ^ 0xa076_1d64_78bd_642f_u64.wrapping_mul(index as u64 + 1);
        match worm.descend_cancellable(&ising, &mut read.spins, &settings, local_seed, stopped) {
            Ok(_) => read.energy_milli = ising.energy(&read.spins),
            Err(DescentError::Cancelled) => return Ok(false),
            Err(_) => return Err(SampleError::Capacity),
        }
    }
    Ok(true)
}

#[cfg(feature = "tree-polish")]
fn polish_trees(
    graph: &OtsukaGraph,
    reads: &mut [SamplerResult],
    seed: u64,
    stopped: &impl Fn() -> bool,
) -> bool {
    TreePolish::new(graph, seed, stopped)
        .and_then(|mut polish| polish.improve(reads, stopped))
        .is_some()
}

fn retain_record(reads: &mut Vec<SamplerResult>, record: SamplerResult, limit: usize) {
    if reads.iter().any(|r| r.spins == record.spins) {
        return;
    }
    reads.push(record);
    reads.sort_by_key(|r| r.energy_milli);
    reads.truncate(limit);
}

fn uniform_weak_levels(instance: &Instance, config: &FunnelConfig) -> Option<(i64, i64)> {
    let weak = instance
        .j()
        .iter()
        .map(|j| j.abs())
        .filter(|&j| j < 1000)
        .max()?;
    if instance
        .j()
        .iter()
        .any(|j| j.abs() < 1000 && j.abs() != weak)
    {
        return None;
    }
    Some((
        i64::from(weak),
        i64::from(((f64::from(weak) * config.weak_scale) as i32).min(999)),
    ))
}

fn original_record_energy(
    biased: &OtsukaGraph,
    truth: &OtsukaGraph,
    levels: Option<(i64, i64)>,
    energy: i64,
    spins: &[i8],
) -> i64 {
    let Some((original_weak, search_weak)) = levels else {
        return truth.energy(spins);
    };
    if search_weak == 0 {
        return energy;
    }
    let strong = biased.full_strong_energy(spins);
    let weak_energy = energy - strong;
    if weak_energy % search_weak != 0 {
        return truth.energy(spins);
    }
    strong + (weak_energy / search_weak) * original_weak
}

fn offset_reads(instance: &Instance, reads: &mut [SamplerResult]) {
    for read in reads {
        read.energy_milli = instance_energy(instance, &read.spins);
    }
}

fn lowest(reads: &[SamplerResult]) -> i64 {
    reads
        .iter()
        .map(|r| r.energy_milli)
        .min()
        .unwrap_or(i64::MAX)
}

fn seed_for(nonce: [u8; 32], phase: u64) -> u64 {
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&nonce[..8]);
    u64::from_le_bytes(bytes) ^ phase
}

fn canonical_matches(fast: &fast_dimer::FastDimer, instance: &Instance, nonce: [u8; 32]) -> bool {
    fast.couplings(nonce).is_some_and(|(j, tiles)| {
        j == instance.j()
            && fast.edges() == instance.edges()
            && local_gauges(instance).is_some_and(|(canonical, _)| canonical == tiles)
    })
}

struct Candidate {
    index: u64,
    instance: Instance,
    reads: Vec<SamplerResult>,
}

impl FunnelSampler {
    fn work_family(
        &self,
        lease: &Lease,
        params: &SampleParams,
        sink: &LeaseSink,
        config: &FunnelConfig,
    ) -> Result<(), SampleError> {
        let family = lease.family().ok_or(SampleError::Capacity)?;
        let fast = fast_dimer::FastDimer::new(
            family.family_id(),
            family.revision(),
            family.params(),
            family.topology(),
        )
        .ok_or(SampleError::Capacity)?;
        let edges: Arc<[(usize, usize)]> = fast
            .edges()
            .iter()
            .map(|&(u, v)| (u as usize, v as usize))
            .collect();
        let lean = stage0::Topo::shared(fast.nodes(), &edges);
        let cycles = stage0v2::Topo2::shared(fast.nodes(), &edges);
        let started = Instant::now();
        let mut batch_start = 0;
        while batch_start < lease.salt_count() && !sink.is_stopped() {
            let end = (batch_start + config.batch as u64).min(lease.salt_count());
            let mut scores = Vec::new();
            for index in batch_start..end {
                if sink.is_stopped() {
                    return Ok(());
                }
                let nonce = lease.nonce(index);
                let Some((j, tiles)) = fast.couplings(nonce) else {
                    match family.generate(nonce) {
                        Err(SaltFault::Skip(_)) => {
                            let _ = sink.skip(index);
                            continue;
                        }
                        Err(SaltFault::Defect(error)) => {
                            return Err(SampleError::DeviceFault(format!(
                                "canonical family generation failed: {error:?}"
                            )))
                        }
                        Ok(_) => {
                            return Err(SampleError::DeviceFault(
                                "cached generator declined a canonical instance".into(),
                            ))
                        }
                    }
                };
                if index.is_multiple_of(64) {
                    let instance = family
                        .generate(nonce)
                        .map_err(|e| SampleError::DeviceFault(format!("generator audit: {e:?}")))?;
                    if !canonical_matches(&fast, &instance, nonce) {
                        return Err(SampleError::DeviceFault(
                            "cached generator audit mismatch".into(),
                        ));
                    }
                }
                let score = if config.screen {
                    stage0::lean_score(&lean, &j, &tiles).ok_or(SampleError::Capacity)?
                } else {
                    0.0
                };
                scores.push((index, score));
            }
            scores.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            let pre = if config.screen {
                (scores.len() / 5).max(config.keep).min(scores.len())
            } else {
                scores.len()
            };
            for &(index, _) in scores.iter().skip(pre) {
                if sink.screen(index, i64::MAX).is_err() {
                    return Ok(());
                }
            }
            scores.truncate(pre);
            if config.screen {
                for (index, score) in &mut scores {
                    if sink.is_stopped() {
                        return Ok(());
                    }
                    let (j, tiles) = fast
                        .couplings(lease.nonce(*index))
                        .ok_or(SampleError::Capacity)?;
                    *score = stage0v2::score(&cycles, &j, &tiles).ok_or(SampleError::Capacity)?;
                }
                scores.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
                for &(index, _) in scores.iter().skip(config.keep) {
                    if sink.screen(index, i64::MAX).is_err() {
                        return Ok(());
                    }
                }
                scores.truncate(config.keep);
            }
            let mut candidates = Vec::new();
            for (index, _) in scores {
                if sink.is_stopped() {
                    return Ok(());
                }
                let nonce = lease.nonce(index);
                let instance = family
                    .generate(nonce)
                    .map_err(|e| SampleError::DeviceFault(format!("canonical survivor: {e:?}")))?;
                if !canonical_matches(&fast, &instance, nonce) {
                    return Err(SampleError::DeviceFault(
                        "cached survivor differs from canonical generator".into(),
                    ));
                }
                let mut trial = params.clone();
                trial.seed = seed_for(nonce, 0x5354_4147_455f_3031);
                if config.screen {
                    trial.num_reads = config.probe_reads;
                    trial.num_sweeps = config.probe_sweeps;
                } else {
                    trial.num_reads = config.final_reads.unwrap_or(params.num_reads);
                    trial.num_sweeps = config.final_sweeps.unwrap_or(params.num_sweeps);
                }
                let trial_config = FunnelConfig {
                    polish_sweeps: if config.screen {
                        0
                    } else {
                        config.polish_sweeps
                    },
                    ..config.clone()
                };
                let Some(mut reads) =
                    tuned_reads(&instance, &trial, &trial_config, || sink.is_stopped())?
                else {
                    return Ok(());
                };
                offset_reads(&instance, &mut reads);
                let best = lowest(&reads);
                tracing::info!(
                    salt_index = index,
                    best_energy_milli = best,
                    reads = trial.num_reads,
                    sweeps = trial.num_sweeps,
                    phase = if config.screen { "probe" } else { "direct" },
                    "CPU Funnel solve"
                );
                if !config.screen
                    || sink
                        .target_energy_milli()
                        .is_some_and(|target| best < target)
                {
                    if sink.push(index, reads).is_err() {
                        return Ok(());
                    }
                } else {
                    candidates.push(Candidate {
                        index,
                        instance,
                        reads,
                    });
                }
            }
            candidates.sort_by_key(|c| lowest(&c.reads));
            if !candidates.is_empty() {
                let mut selected = candidates.remove(0);
                for candidate in candidates {
                    if sink.push(candidate.index, candidate.reads).is_err() {
                        return Ok(());
                    }
                }
                let nonce = lease.nonce(selected.index);
                let final_params = SampleParams {
                    num_reads: config.final_reads.unwrap_or(params.num_reads),
                    num_sweeps: config.final_sweeps.unwrap_or(params.num_sweeps),
                    seed: seed_for(nonce, 0x5354_4147_455f_3032),
                    ..params.clone()
                };
                let Some(mut deep) =
                    tuned_reads(&selected.instance, &final_params, config, || {
                        sink.is_stopped()
                    })?
                else {
                    return Ok(());
                };
                offset_reads(&selected.instance, &mut deep);
                if lowest(&deep) < lowest(&selected.reads) {
                    selected.reads = deep;
                }
                tracing::info!(
                    salt_index = selected.index,
                    best_energy_milli = lowest(&selected.reads),
                    reads = final_params.num_reads,
                    sweeps = final_params.num_sweeps,
                    elapsed_s = started.elapsed().as_secs_f64(),
                    "CPU Funnel deep solve"
                );
                if sink.push(selected.index, selected.reads).is_err() {
                    return Ok(());
                }
            }
            batch_start = end;
        }
        Ok(())
    }

    fn work_other(
        &self,
        lease: &Lease,
        topology: &TopologyView,
        params: &SampleParams,
        sink: &LeaseSink,
    ) -> Result<(), SampleError> {
        let cancel = CancelToken::default();
        let done = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                while !done.load(Ordering::Relaxed) {
                    if sink.is_stopped() {
                        cancel.cancel_through(1);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            });
            let result = (|| {
                for index in 0..lease.salt_count() {
                    if sink.is_stopped() {
                        break;
                    }
                    let (graph, offset) = if let Some(family) = lease.family() {
                        match family.generate(lease.nonce(index)) {
                            Ok(instance) => (graph_of(&instance), instance.offset_milli()),
                            Err(SaltFault::Skip(_)) => {
                                let _ = sink.skip(index);
                                continue;
                            }
                            Err(SaltFault::Defect(error)) => {
                                return Err(SampleError::DeviceFault(format!(
                                    "family generation: {error:?}"
                                )))
                            }
                        }
                    } else {
                        let (h, j) = topology
                            .draw(lease.nonce(index))
                            .map_err(|_| SampleError::Capacity)?;
                        (
                            IsingGraph::new(
                                h.iter().map(|&v| f64::from(v) / 1000.0).collect(),
                                j.iter().map(|&v| f64::from(v) / 1000.0).collect(),
                                topology.edges.clone(),
                            ),
                            0,
                        )
                    };
                    crate::check_sample_size(graph.h.len(), graph.edges.len())?;
                    let Ok(mut reads) = sample_ising_cancellable(
                        &graph,
                        params,
                        crate::Algorithm::Sa,
                        Some((&cancel, Some(1))),
                    ) else {
                        break;
                    };
                    for read in &mut reads {
                        read.energy_milli += offset;
                    }
                    if sink.push(index, reads).is_err() {
                        break;
                    }
                }
                Ok(())
            })();
            done.store(true, Ordering::Relaxed);
            result
        })
    }
}

impl Sampler for FunnelSampler {
    fn sample(
        &self,
        graph: &IsingGraph,
        params: &SampleParams,
    ) -> Result<Vec<SamplerResult>, SampleError> {
        self.stock.sample(graph, params)
    }
    fn accepts_family_metadata() -> bool {
        true
    }
    fn generates_locally() -> bool {
        true
    }
    fn declaration() -> Declaration {
        Declaration {
            solver_id: "quip-cpu-funnel/1",
            generated: CLAIMS,
            accepted: CLAIMS,
        }
    }
    fn stream_width(&self) -> usize {
        crate::core_budget(&self.num_cpus)
    }
    fn declared_stream_width() -> u32 {
        0
    }
    fn max_reads(&self) -> u32 {
        128
    }
    fn apply_config(&self, backend_toml: &str) {
        match backend_toml.parse::<toml::Table>() {
            Ok(table) => {
                let mut common = toml::Table::new();
                if let Some(value) = table.get("num_cpus") {
                    common.insert("num_cpus".into(), value.clone());
                }
                let common = toml::to_string(&common).unwrap_or_default();
                self.stock.apply_config(&common);
                if let Err(error) = crate::apply_num_cpus(&self.num_cpus, &common) {
                    crate::reject_num_cpus(error);
                }
                let mut stored = self.config.lock().unwrap_or_else(PoisonError::into_inner);
                let mut updated = stored.clone();
                match updated.apply(&table) {
                    Ok(()) => {
                        *stored = updated;
                        tracing::info!(config=?*stored, "CPU population solver configured");
                    }
                    Err(error) => {
                        tracing::error!(%error, "CPU Funnel config rejected; retaining previous settings")
                    }
                }
            }
            Err(error) => tracing::error!(%error, "CPU Funnel TOML rejected"),
        }
    }
    fn sample_family(
        &self,
        graph: &IsingGraph,
        params: &SampleParams,
        warm: Option<&WarmStart>,
        family: &FamilyContext,
    ) -> Result<Vec<SamplerResult>, SampleError> {
        if family.family_id() != TWISTED_DIMER_V1.family_id
            || family.revision() != TWISTED_DIMER_V1.revision
        {
            return self.stock.sample_family(graph, params, warm, family);
        }
        let config = self
            .config
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(tuned_reads(family.instance(), params, &config, || false)?.unwrap_or_default())
    }
    fn sample_lease(
        &self,
        lease: &Lease,
        topology: &TopologyView,
        params: &SampleParams,
        sink: &LeaseSink,
    ) -> Result<(), SampleError> {
        let Some(_slot) = self.acquire(sink) else {
            return Ok(());
        };
        let config = self
            .config
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if lease.family().is_some_and(|family| {
            family.family_id() == TWISTED_DIMER_V1.family_id
                && family.revision() == TWISTED_DIMER_V1.revision
        }) {
            self.work_family(lease, params, sink, &config)
        } else {
            self.work_other(lease, topology, params, sink)
        }
    }
    fn sample_stream_warm(
        &self,
        jobs: tokio::sync::mpsc::Receiver<WarmStreamJob>,
        out: tokio::sync::mpsc::Sender<StreamResult>,
        cancel: CancelToken,
    ) {
        self.stock.sample_stream_warm(jobs, out, cancel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quip_solver_conformance::families::{export_instances, reference_revision};
    use quip_solver_core::ising_generators::family::BaseTopology;

    fn reference() -> (BaseTopology, Vec<u8>, Vec<Instance>) {
        let revision =
            reference_revision(TWISTED_DIMER_V1.family_id, TWISTED_DIMER_V1.revision).unwrap();
        let stored = quip_solver_conformance::topology(revision.topology).unwrap();
        let topology = BaseTopology::new(stored.nodes, &stored.edges).unwrap();
        let nonces: Vec<_> = (0..64_u8).map(|n| [n; 32]).collect();
        for params in revision.params {
            if let Ok(instances) = export_instances(revision.contract, &params, &topology, &nonces)
            {
                let instances: Vec<_> = instances.into_iter().filter_map(Result::ok).collect();
                if instances.len() == nonces.len() {
                    return (topology, params.to_vec(), instances);
                }
            }
        }
        unreachable!("the reference has a feasible parameter set")
    }

    #[test]
    fn cached_generator_matches_64_canonical_nonces() {
        let (topology, params, instances) = reference();
        let fast =
            fast_dimer::FastDimer::new(TWISTED_DIMER_V1.family_id, 1, &params, &topology).unwrap();
        for (nonce, instance) in (0..64_u8).zip(instances) {
            assert!(
                canonical_matches(&fast, &instance, [nonce; 32]),
                "nonce {nonce}"
            );
        }
    }

    #[test]
    fn biased_anneals_report_original_energies_and_polish_never_raises_them() {
        let (_, _, instances) = reference();
        let config = FunnelConfig {
            polish_sweeps: 0,
            ..FunnelConfig::default()
        };
        for instance in instances.iter().take(8) {
            let params = SampleParams {
                num_reads: 8,
                num_sweeps: 128,
                seed: 731,
                ..SampleParams::default()
            };
            let before = tuned_reads(instance, &params, &config, || false)
                .unwrap()
                .unwrap();
            let config = FunnelConfig {
                polish_sweeps: 32,
                ..config.clone()
            };
            let after = tuned_reads(instance, &params, &config, || false)
                .unwrap()
                .unwrap();
            assert_eq!(before.len(), after.len());
            for (raw, polished) in before.iter().zip(&after) {
                assert!(polished.energy_milli <= raw.energy_milli);
                assert_eq!(
                    instance_energy(instance, &polished.spins),
                    polished.energy_milli + instance.offset_milli()
                );
            }
        }
    }

    #[test]
    fn native_execution_preserves_pipeline_reads_and_archive_never_loses_the_best() {
        let (_, _, instances) = reference();
        for instance in instances.iter().take(4) {
            let params = SampleParams {
                num_reads: 4,
                num_sweeps: 96,
                seed: 3311,
                ..SampleParams::default()
            };
            let baseline = FunnelConfig {
                polish_sweeps: 16,
                ..FunnelConfig::default()
            };
            let native = FunnelConfig {
                shared_cache: true,
                copy_resampling: true,
                inline_union: true,
                fused_updates: true,
                ..baseline.clone()
            };
            let a = tuned_reads(instance, &params, &baseline, || false)
                .unwrap()
                .unwrap();
            let b = tuned_reads(instance, &params, &native, || false)
                .unwrap()
                .unwrap();
            assert_eq!(a.len(), b.len());
            for (left, right) in a.iter().zip(&b) {
                assert_eq!(left.spins, right.spins);
                assert_eq!(left.energy_milli, right.energy_milli);
            }
            let archive = FunnelConfig {
                archive: true,
                ..native
            };
            let c = tuned_reads(instance, &params, &archive, || false)
                .unwrap()
                .unwrap();
            let scalar_archive = FunnelConfig {
                motif_score: false,
                ..archive.clone()
            };
            let scalar = tuned_reads(instance, &params, &scalar_archive, || false)
                .unwrap()
                .unwrap();
            for (left, right) in c.iter().zip(&scalar) {
                assert_eq!(left.spins, right.spins);
                assert_eq!(left.energy_milli, right.energy_milli);
            }
            assert_eq!(c.len(), params.num_reads);
            assert!(lowest(&c) <= lowest(&b));
            for read in c {
                assert_eq!(
                    read.energy_milli + instance.offset_milli(),
                    instance_energy(instance, &read.spins)
                );
            }
        }
    }

    #[cfg(feature = "pair-worm-polish")]
    #[test]
    fn pair_worm_polish_keeps_candidates_and_lowers_original_energy() {
        let (_, _, instances) = reference();
        let instance = &instances[0];
        for polish_sweeps in [0, 4] {
            let config = FunnelConfig {
                archive: true,
                polish_sweeps,
                ..FunnelConfig::default()
            };
            let mut polished = config.clone();
            polished
                .apply(
                    &"funnel_pair_worm_polish = true"
                        .parse::<toml::Table>()
                        .unwrap(),
                )
                .unwrap();
            let params = SampleParams {
                num_reads: 4,
                num_sweeps: 32,
                seed: 5281,
                ..SampleParams::default()
            };
            let before = tuned_reads(instance, &params, &config, || false)
                .unwrap()
                .unwrap();
            let after = tuned_reads(instance, &params, &polished, || false)
                .unwrap()
                .unwrap();
            assert_eq!(after.len(), params.num_reads);
            assert_eq!(before.len(), after.len());
            for (original, polished) in before.iter().zip(&after) {
                assert!(polished.energy_milli <= original.energy_milli);
                assert_eq!(
                    polished.energy_milli + instance.offset_milli(),
                    instance_energy(instance, &polished.spins)
                );
            }
        }
    }

    #[cfg(feature = "tree-polish")]
    #[test]
    fn configured_tree_polish_preserves_candidate_count_and_canonical_energy() {
        let (_, _, instances) = reference();
        let config = FunnelConfig {
            polish_sweeps: 0,
            archive: true,
            ..FunnelConfig::default()
        };
        let mut polished = config.clone();
        let settings = "funnel_tree_polish = true".parse::<toml::Table>().unwrap();
        polished.apply(&settings).unwrap();
        let params = SampleParams {
            num_reads: 4,
            num_sweeps: 16,
            seed: 5309,
            ..SampleParams::default()
        };
        let instance = &instances[0];
        let baseline = tuned_reads(instance, &params, &config, || false)
            .unwrap()
            .unwrap();
        let candidate = tuned_reads(instance, &params, &polished, || false)
            .unwrap()
            .unwrap();
        assert_eq!(baseline.len(), candidate.len());
        assert_eq!(candidate.len(), params.num_reads);
        assert!(lowest(&candidate) <= lowest(&baseline));
        for read in candidate {
            assert_eq!(
                read.energy_milli + instance.offset_milli(),
                instance_energy(instance, &read.spins)
            );
        }
    }

    #[test]
    fn motif_original_energy_conversion_matches_full_canonical_rescoring() {
        use rand::Rng;
        let (_, _, instances) = reference();
        let mut rng = SmallRng::seed_from_u64(87142);
        for instance in instances.iter().take(4) {
            for scale in [1.0, 1.25, 1.5] {
                let config = FunnelConfig {
                    weak_scale: scale,
                    ..FunnelConfig::default()
                };
                let biased = prepared(instance, &config, true).unwrap();
                let truth = prepared(instance, &config, false).unwrap();
                let levels = uniform_weak_levels(instance, &config);
                assert!(levels.is_some());
                for _ in 0..32 {
                    let spins: Vec<i8> = (0..instance.h().len())
                        .map(|_| if rng.gen::<bool>() { 1 } else { -1 })
                        .collect();
                    let actual = original_record_energy(
                        &biased,
                        &truth,
                        levels,
                        biased.energy(&spins),
                        &spins,
                    );
                    assert_eq!(
                        actual + instance.offset_milli(),
                        instance_energy(instance, &spins)
                    );
                }
            }
        }
    }
}
