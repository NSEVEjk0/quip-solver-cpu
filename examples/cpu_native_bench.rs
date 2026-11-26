//! Canonical current-recipe CPU benchmark with independent optimization flags.
use clap::{Parser, ValueEnum};
use quip_solver_conformance::families::{export_instances, reference_revision};
use quip_solver_core::ising_generators::family::BaseTopology;
use quip_solver_core::ising_generators::ice::TWISTED_DIMER_V1;
use quip_solver_core::SampleParams;
use quip_solver_cpu::families::funnel::{instance_energy, prepared, tuned_reads, FunnelConfig};
use quip_solver_cpu::families::otsuka::{anneal_population_observed, PopulationOptions, Schedule};
use serde::Deserialize;
use std::error::Error;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Mode {
    Kernel,
    Pipeline,
}
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Preset {
    Baseline,
    Cache,
    Copy,
    Union,
    Fusion,
    Core,
    Archive,
}

#[derive(Parser)]
struct Args {
    #[arg(
        long,
        default_value = "tools/cpu-funnel/inputs/current-topology.spec.json"
    )]
    topology: PathBuf,
    #[arg(long, default_value = "tools/cpu-funnel/inputs/current-family.params")]
    params: PathBuf,
    #[arg(long, value_enum, default_value = "kernel")]
    mode: Mode,
    #[arg(long, value_enum, default_value = "baseline")]
    preset: Preset,
    #[arg(long, default_value_t = 1001)]
    seed: u64,
    #[arg(long, default_value_t = 16)]
    reads: usize,
    #[arg(long, default_value_t = 1024)]
    sweeps: usize,
    #[arg(long, default_value_t = 12.0)]
    beta_max: f64,
    #[arg(long)]
    weak_theta: Option<f64>,
    #[arg(long, default_value_t = 20.0)]
    polish_beta: f64,
    #[arg(long, default_value_t = 256)]
    polish_sweeps: usize,
    #[arg(long, default_value_t = 1)]
    single_every: usize,
    #[arg(long, default_value_t = 0.0)]
    beta_hold: f64,
    #[arg(long, default_value_t = 250)]
    tile_milli: i32,
    #[arg(long, default_value_t = 250)]
    pair_milli: i32,
    #[arg(long, default_value_t = 1.5)]
    weak_scale: f64,
    #[arg(long, default_value_t = 512)]
    resamples: usize,
    #[arg(long, default_value_t = 0)]
    houdayer_every: usize,
    #[arg(long)]
    independent_ancestry: bool,
    #[arg(long)]
    strong_exchanges: bool,
    #[arg(long)]
    slot_archive: bool,
    #[arg(long, default_value_t = 8192)]
    cache_slots: usize,
    #[arg(long)]
    disable_cache: bool,
    #[arg(long)]
    disable_motif_score: bool,
    #[arg(long)]
    selective_resampling: bool,
}

#[derive(Deserialize)]
struct Topology {
    nodes: Vec<u32>,
    edges: Vec<[u32; 2]>,
}

#[expect(clippy::print_stdout, reason = "benchmark emits one JSON result")]
fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    if !(0.0..1.0).contains(&args.beta_hold) {
        return Err("beta hold must be in [0, 1)".into());
    }
    if args.beta_hold != 0.0 && matches!(args.mode, Mode::Kernel) {
        return Err("beta hold requires pipeline mode".into());
    }
    let spec: Topology = serde_json::from_slice(&std::fs::read(&args.topology)?)?;
    let edges: Vec<_> = spec.edges.into_iter().map(|[u, v]| (u, v)).collect();
    let topology = BaseTopology::new(spec.nodes, &edges).map_err(|e| format!("topology: {e:?}"))?;
    let params = std::fs::read(&args.params)?;
    let revision =
        reference_revision(TWISTED_DIMER_V1.family_id, 1).ok_or("family revision missing")?;
    let nonce = *blake3::hash(&args.seed.to_le_bytes()).as_bytes();
    let instances = export_instances(revision.contract, &params, &topology, &[nonce])?;
    let instance = instances
        .into_iter()
        .next()
        .ok_or("no instance")?
        .map_err(|e| format!("salt generation: {e:?}"))?;
    let all = matches!(args.preset, Preset::Core | Preset::Archive);
    let config = FunnelConfig {
        shared_cache: !args.disable_cache && (all || matches!(args.preset, Preset::Cache)),
        copy_resampling: all || matches!(args.preset, Preset::Copy),
        selective_resampling: args.selective_resampling,
        inline_union: all || matches!(args.preset, Preset::Union),
        fused_updates: all || matches!(args.preset, Preset::Fusion),
        archive: matches!(args.preset, Preset::Archive),
        weak_beta_end: args.weak_theta,
        polish_beta: args.polish_beta,
        polish_sweeps: args.polish_sweeps,
        single_every: args.single_every,
        beta_hold: args.beta_hold,
        tile_milli: args.tile_milli,
        pair_milli: args.pair_milli,
        weak_scale: args.weak_scale,
        resamples: args.resamples,
        houdayer_every: args.houdayer_every,
        independent_ancestry: args.independent_ancestry,
        strong_exchanges: args.strong_exchanges,
        slot_archive: args.slot_archive,
        #[cfg(feature = "tree-polish")]
        tree_polish: true,
        #[cfg(feature = "pair-worm-polish")]
        pair_worm_polish: true,
        cache_slots: args.cache_slots,
        motif_score: !args.disable_motif_score,
        ..FunnelConfig::default()
    };
    let sample_params = SampleParams {
        num_reads: args.reads,
        num_sweeps: args.sweeps,
        seed: args.seed ^ 0x4350_5542_454e_4348,
        beta_range: args.weak_theta.is_none().then_some((1.0, args.beta_max)),
        ..SampleParams::default()
    };
    let effective_beta_max = if let Some(theta) = args.weak_theta {
        instance
            .j()
            .iter()
            .map(|v| v.abs())
            .filter(|&v| v > 0 && v < 1000)
            .max()
            .map(|weak| {
                (theta * 1000.0
                    / f64::from(((f64::from(weak) * config.weak_scale) as i32).min(999)))
                .clamp(1.0, 128.0)
            })
            .unwrap_or(12.0)
    } else {
        args.beta_max
    };
    let mut fingerprint = blake3::Hasher::new();
    let (seconds, energies, metrics) = match args.mode {
        Mode::Kernel => {
            let graph = prepared(&instance, &config, true).map_err(|e| format!("graph: {e:?}"))?;
            let betas: Vec<f64> = (0..args.sweeps)
                .map(|i| {
                    let x = if args.sweeps > 1 {
                        i as f64 / (args.sweeps - 1) as f64
                    } else {
                        1.0
                    };
                    effective_beta_max.powf(x)
                })
                .collect();
            let schedule = Schedule::new(&betas, args.single_every)?;
            let started = Instant::now();
            let outcome = anneal_population_observed(
                &graph,
                &schedule,
                args.reads,
                sample_params.seed,
                PopulationOptions {
                    resamples: config.resamples,
                    houdayer_every: config.houdayer_every,
                    independent_ancestry: args.independent_ancestry,
                    strong_exchanges: args.strong_exchanges,
                    shared_cache: config.shared_cache,
                    cache_slots: config.cache_slots,
                    copy_resampling: config.copy_resampling,
                    selective_resampling: config.selective_resampling,
                    inline_union: config.inline_union,
                    fused_updates: config.fused_updates,
                },
                || false,
                |_, _| {},
            )?;
            let seconds = started.elapsed().as_secs_f64();
            let energies: Vec<_> = outcome
                .samples
                .iter()
                .map(|r| {
                    let original = instance_energy(&instance, &r.spins);
                    fingerprint.update(&original.to_le_bytes());
                    fingerprint.update(&r.spins.iter().map(|&s| s as u8).collect::<Vec<_>>());
                    original
                })
                .collect();
            let m = outcome.metrics;
            let c = m.cache;
            (
                seconds,
                energies,
                serde_json::json!({
                    "replica_sweeps":m.replica_sweeps,"resamples":m.resamples,
                    "field_rebuilds":m.field_rebuilds,"field_copies":m.field_copies,
                    "field_copies_skipped":m.field_copies_skipped,"parent_snapshots":m.parent_snapshots,
                    "local_field_bytes":m.local_field_bytes,"snapshot_bytes":m.snapshot_bytes,
                    "coefficient_visits_avoided":m.coefficient_visits_avoided,"cache_bytes":m.cache_bytes,
                    "boundary_updated_nodes":m.boundary_updated_nodes,
                    "boundary_field_updates":m.boundary_field_updates,
                    "interior_field_updates_avoided":m.interior_field_updates_avoided,
                    "heat_hits":c.heat_hits,"heat_misses":c.heat_misses,
                    "metropolis_hits":c.metropolis_hits,"metropolis_misses":c.metropolis_misses,
                    "fallbacks":c.fallbacks,"temperatures":c.temperatures,
                    "epoch_wraps":c.epoch_wraps,
                    "exchange_proposals": m.exchange_proposals,
                    "exchange_partial": m.exchange_partial,
                    "exchange_rejected": m.exchange_rejected,
                }),
            )
        }
        Mode::Pipeline => {
            let started = Instant::now();
            let reads = tuned_reads(&instance, &sample_params, &config, || false)
                .map_err(|e| format!("pipeline: {e:?}"))?
                .ok_or("unexpected cancellation")?;
            let seconds = started.elapsed().as_secs_f64();
            let energies: Vec<_> = reads
                .iter()
                .map(|r| {
                    let original = instance_energy(&instance, &r.spins);
                    if r.energy_milli + instance.offset_milli() != original {
                        return Err::<i64, Box<dyn Error>>(
                            "Original-energy rescoring mismatch".into(),
                        );
                    }
                    fingerprint.update(&original.to_le_bytes());
                    fingerprint.update(&r.spins.iter().map(|&s| s as u8).collect::<Vec<_>>());
                    Ok(original)
                })
                .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
            (seconds, energies, serde_json::Value::Null)
        }
    };
    let best = energies.iter().min().copied().ok_or("no reads")?;
    let mut row = serde_json::json!({"mode":format!("{:?}",args.mode),"preset":format!("{:?}",args.preset),
        "seed":args.seed,"nonce":blake3::Hash::from(nonce).to_hex().to_string(),
        "reads":args.reads,"sweeps":args.sweeps,"beta_max":effective_beta_max,
        "weak_theta":args.weak_theta,"polish_beta":args.polish_beta,"cache_slots":args.cache_slots,
        "cache_disabled":args.disable_cache,"motif_score_disabled":args.disable_motif_score,
        "selective_resampling":args.selective_resampling,
        "seconds":seconds,"best_energy_milli":best,
        "energies_milli":energies,"trajectory_digest":fingerprint.finalize().to_hex().to_string(),"metrics":metrics});
    let build = serde_json::json!({
        "compact_local_fields":cfg!(feature="compact-local-fields"),
        "compact_cache_stamps":cfg!(feature="compact-cache-stamps"),
        "cluster_boundary_updates":cfg!(feature="cluster-boundary-updates"),
        "packed_component_spins":cfg!(feature="packed-component-spins"),
        "row_grouped_residual_deltas":cfg!(feature="row-grouped-residual-deltas"),
        "masked_residual_deltas":cfg!(feature="masked-residual-deltas"),
        "table_tile_bonds":cfg!(feature="table-tile-bonds"),
        "shallow_union_roots":cfg!(feature="shallow-union-roots"),
        "linked_component_lists":cfg!(feature="linked-component-lists"),
        "packed_residual_fields":cfg!(feature="packed-residual-fields"),
        "tree_polish":cfg!(feature="tree-polish"),
        "cyclic_polish":cfg!(feature="cyclic-polish"),
        "pair_worm_polish":cfg!(feature="pair-worm-polish"),
        "interleaved_probability_cache":cfg!(feature="interleaved-probability-cache"),
    });
    let schedule = serde_json::json!({
        "polish_sweeps":args.polish_sweeps,
        "single_every":args.single_every,
        "beta_hold":args.beta_hold,"weak_scale":args.weak_scale,
        "tile_milli":args.tile_milli,"pair_milli":args.pair_milli,"resamples":args.resamples,
        "houdayer_every":args.houdayer_every,"independent_ancestry":args.independent_ancestry,
        "slot_archive":args.slot_archive,
        "strong_exchanges":args.strong_exchanges,
    });
    let row_fields = row
        .as_object_mut()
        .ok_or("benchmark output must be an object")?;
    for fields in [build, schedule] {
        if let serde_json::Value::Object(fields) = fields {
            row_fields.extend(fields);
        }
    }
    println!("{row}");
    Ok(())
}
