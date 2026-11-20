//! CPU adaptation of BudFox's Funnel branch (ntllinh2511/quip-solver-cuda,
//! revision 77bfd821). The three screen modules retain their AGPL source.
//!
//! Each worker screens a bounded batch, trials a few survivors, deep-solves
//! the best trial, and polishes with the original coefficients. Generation
//! is checked against the canonical family generator before a survivor runs.
//! A lease and its candidate pool never outlive their round.

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

use super::otsuka::{
    anneal_population_configured, AnnealError, Chain, Model, Motifs, OtsukaGraph,
    PopulationOptions, Schedule, Weights,
};
use super::otsuka_sampler::{parse_pairs, section, PAIRS_SECTION};
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
        let mut config = Self::default();
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
        ] {
            let Ok(raw) = std::env::var(format!("QUIP_CPU_FUNNEL_{}", key.to_uppercase())) else {
                continue;
            };
            let value = match key {
                "screen" => raw.parse::<bool>().ok().map(toml::Value::Boolean),
                "weak_scale" => raw.parse::<f64>().ok().map(toml::Value::Float),
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

fn prepared(
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
                (f64::from(v) * config.weak_scale) as i32
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
        beta_range: Some(params.beta_range.unwrap_or((1.0, 12.0))),
        ..params.clone()
    };
    let per_beta = effective_sweeps_per_beta(params.num_sweeps, params.sweeps_per_beta);
    let betas: Vec<f64> = build_beta_schedule(&graph, &params)
        .into_iter()
        .flat_map(|beta| std::iter::repeat_n(beta, per_beta))
        .collect();
    let schedule = Schedule::new(&betas, 1).map_err(|_| SampleError::Capacity)?;
    let samples = match anneal_population_configured(
        &biased,
        &schedule,
        params.num_reads,
        params.seed,
        PopulationOptions {
            resamples: config.resamples,
            houdayer_every: config.houdayer_every,
        },
        &stopped,
    ) {
        Ok(samples) => samples,
        Err(AnnealError::Cancelled) => return Ok(None),
    };
    let mut reads: Vec<SamplerResult> = samples
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
    for i in order.into_iter().take(4) {
        let read = &mut reads[i];
        let mut chain =
            Chain::new(&truth, read.spins.clone()).map_err(|_| SampleError::Capacity)?;
        let mut energy = read.energy_milli;
        for _ in 0..config.polish_sweeps {
            if stopped() {
                return Ok(None);
            }
            energy += chain.cluster_sweep(20.0, &mut rng);
            energy += chain.single_sweep(20.0, &mut rng);
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
    Ok(Some(reads))
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
        self.stock.apply_config(backend_toml);
        if let Err(error) = crate::apply_num_cpus(&self.num_cpus, backend_toml) {
            crate::reject_num_cpus(error);
        }
        match backend_toml.parse::<toml::Table>() {
            Ok(table) => {
                let mut stored = self.config.lock().unwrap_or_else(PoisonError::into_inner);
                let mut updated = stored.clone();
                match updated.apply(&table) {
                    Ok(()) => {
                        *stored = updated;
                        tracing::info!(config=?*stored, "CPU Funnel configured");
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
}
