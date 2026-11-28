//! Offline ranking signals for canonical twisted-ice-dimer instances.

use clap::Parser;
use quip_solver_conformance::families::{export_instances, reference_revision};
use quip_solver_core::ising_generators::family::BaseTopology;
use quip_solver_core::ising_generators::ice::TWISTED_DIMER_V1;
use quip_solver_core::ising_generators::instance::Instance;
use quip_solver_core::SampleParams;
use quip_solver_cpu::families::native::{instance_energy, tuned_reads, NativeConfig};
use quip_solver_cpu::postprocess::greedy::{Greedy, Ising};
use rand::{rngs::SmallRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::{error::Error, path::PathBuf, time::Instant};

const SWEEPS: usize = 24;
const HYPERPLANES: usize = 12;
const LOCAL_STARTS: usize = 3;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    seed: u64,
    #[arg(
        long,
        default_value = "tools/cpu-native/inputs/current-topology.spec.json"
    )]
    topology: PathBuf,
    #[arg(long, default_value = "tools/cpu-native/inputs/current-family.params")]
    params: PathBuf,
    #[arg(long)]
    save_starts: Option<PathBuf>,
}

#[derive(Deserialize)]
struct Topology {
    nodes: Vec<u32>,
    edges: Vec<[u32; 2]>,
}

type Edge = (u32, i32);

fn adjacency(n: usize, edges: &[(u32, u32)], j: &[i32]) -> Vec<Vec<Edge>> {
    let mut rows = vec![Vec::new(); n];
    for (&(u, v), &weight) in edges.iter().zip(j) {
        rows[u as usize].push((v, weight));
        rows[v as usize].push((u, weight));
    }
    for row in &mut rows {
        row.sort_unstable_by_key(|&(v, _)| v);
    }
    rows
}

fn weak(j: i32) -> bool {
    j != 0 && j.unsigned_abs() < 1000
}

#[derive(Serialize)]
struct Structure {
    weak_abs_field_score: i64,
    frustrated_weak_triangles: u64,
    weak_triangles: u64,
    seconds: f64,
}

fn structure(rows: &[Vec<Edge>]) -> Structure {
    let started = Instant::now();
    let mut weak_abs_field_score = 0;
    let mut weak_triangles = 0;
    let mut frustrated_weak_triangles = 0;
    for (u, row) in rows.iter().enumerate() {
        weak_abs_field_score -= row
            .iter()
            .filter_map(|&(_, j)| weak(j).then_some(i64::from(j)))
            .sum::<i64>()
            .abs();
        for &(v, j_uv) in row {
            if v as usize <= u || !weak(j_uv) {
                continue;
            }
            for &(w, j_vw) in &rows[v as usize] {
                if w <= v || !weak(j_vw) {
                    continue;
                }
                if let Ok(closing) = row.binary_search_by_key(&w, |&(node, _)| node) {
                    let j_wu = row[closing].1;
                    if weak(j_wu) {
                        weak_triangles += 1;
                        // A triangle with positive coupling-sign product
                        // cannot satisfy all three preferred spin products.
                        frustrated_weak_triangles += u64::from(j_uv * j_vw * j_wu > 0);
                    }
                }
            }
        }
    }
    Structure {
        weak_abs_field_score,
        frustrated_weak_triangles,
        weak_triangles,
        seconds: started.elapsed().as_secs_f64(),
    }
}

fn weight(j: i32, weak_scale: f64) -> f64 {
    f64::from(j) * if weak(j) { weak_scale } else { 1.0 }
}

fn dot<const D: usize>(a: &[f64; D], b: &[f64; D]) -> f64 {
    a.iter().zip(b).map(|(u, v)| u * v).sum()
}

fn vector_energy<const D: usize>(vectors: &[[f64; D]], rows: &[Vec<Edge>], weak_scale: f64) -> f64 {
    rows.iter()
        .enumerate()
        .map(|(u, row)| {
            row.iter()
                .filter(|&&(v, _)| v as usize > u)
                .map(|&(v, j)| weight(j, weak_scale) * dot(&vectors[u], &vectors[v as usize]))
                .sum::<f64>()
        })
        .sum()
}

fn gaussian<const D: usize>(rng: &mut SmallRng) -> [f64; D] {
    let mut values = [0.0; D];
    for chunk in values.chunks_mut(2) {
        let radius = (-2.0 * (1.0 - rng.gen::<f64>()).ln()).sqrt();
        let angle = std::f64::consts::TAU * rng.gen::<f64>();
        let (sin, cos) = angle.sin_cos();
        chunk[0] = radius * cos;
        if chunk.len() == 2 {
            chunk[1] = radius * sin;
        }
    }
    values
}

fn initialize<const D: usize>(n: usize, rng: &mut SmallRng) -> Vec<[f64; D]> {
    (0..n)
        .map(|_| {
            let mut node = gaussian::<D>(rng);
            let norm = dot(&node, &node).sqrt();
            if norm > 0.0 {
                for x in &mut node {
                    *x /= norm;
                }
            } else {
                node[0] = 1.0;
            }
            node
        })
        .collect()
}

fn sweep<const D: usize>(vectors: &mut [[f64; D]], rows: &[Vec<Edge>], weak_scale: f64) {
    for (u, row) in rows.iter().enumerate() {
        let mut field = [0.0; D];
        for &(v, j) in row {
            let j = weight(j, weak_scale);
            for (f, x) in field.iter_mut().zip(vectors[v as usize]) {
                *f += j * x;
            }
        }
        let norm = dot(&field, &field).sqrt();
        if norm > 0.0 {
            vectors[u] = field.map(|value| -value / norm);
        }
    }
}

#[derive(Serialize)]
struct Relaxation {
    rank: usize,
    sweeps: usize,
    weak_scale: f64,
    weighted_before_milli: f64,
    weighted_after_milli: f64,
    relaxed_milli: f64,
    rounded_best_milli: i64,
    descended_best_milli: i64,
    descended_mean_milli: f64,
    seconds: f64,
}

type RankedStart = (i64, Vec<i8>);

fn relax<const D: usize>(
    instance: &Instance,
    rows: &[Vec<Edge>],
    greedy: &Greedy,
    weak_scale: f64,
    seed: u64,
) -> Result<(Relaxation, Vec<RankedStart>), Box<dyn Error>> {
    let started = Instant::now();
    let mut rng = SmallRng::seed_from_u64(seed);
    let mut vectors = initialize::<D>(rows.len(), &mut rng);
    let before = vector_energy(&vectors, rows, weak_scale);
    for _ in 0..SWEEPS {
        sweep(&mut vectors, rows, weak_scale);
    }
    let after = vector_energy(&vectors, rows, weak_scale);
    if !after.is_finite() || after > before + 1.0e-5 {
        return Err("vector coordinate descent failed its objective check".into());
    }
    let relaxed = if weak_scale == 1.0 {
        after
    } else {
        vector_energy(&vectors, rows, 1.0)
    };
    let mut starts = Vec::new();
    for _ in 0..HYPERPLANES {
        let plane = gaussian::<D>(&mut rng);
        // Use one representative of each globally inverted pair.
        let orientation = if dot(&vectors[0], &plane) < 0.0 {
            -1
        } else {
            1
        };
        let spins: Vec<i8> = vectors
            .iter()
            .map(|u| {
                if dot(u, &plane) < 0.0 {
                    -orientation
                } else {
                    orientation
                }
            })
            .collect();
        if !starts.iter().any(|(_, old)| *old == spins) {
            starts.push((instance_energy(instance, &spins), spins));
        }
    }
    starts.sort_by_key(|(energy, _)| *energy);
    let rounded = starts.first().ok_or("no rounded candidates")?.0;
    starts.truncate(LOCAL_STARTS);
    for (energy, spins) in &mut starts {
        greedy.descend(spins, None)?;
        let canonical = instance_energy(instance, spins);
        if canonical > *energy
            || greedy.ising().energy(spins) + instance.offset_milli() != canonical
        {
            return Err("rounded candidate failed canonical rescoring".into());
        }
        *energy = canonical;
    }
    starts.sort_by_key(|(energy, _)| *energy);
    let best = starts.first().ok_or("no descended candidates")?.0;
    let mean = starts.iter().map(|(energy, _)| *energy as f64).sum::<f64>() / starts.len() as f64;
    let seconds = started.elapsed().as_secs_f64();
    Ok((
        Relaxation {
            rank: D,
            sweeps: SWEEPS,
            weak_scale,
            weighted_before_milli: before,
            weighted_after_milli: after,
            relaxed_milli: relaxed + instance.offset_milli() as f64,
            rounded_best_milli: rounded,
            descended_best_milli: best,
            descended_mean_milli: mean,
            seconds,
        },
        starts,
    ))
}

#[derive(Serialize)]
struct Probe {
    reads: usize,
    sweeps: usize,
    best_milli: i64,
    mean_milli: f64,
    seconds: f64,
}

fn probe(instance: &Instance, sweeps: usize, seed: u64) -> Result<Probe, Box<dyn Error>> {
    let started = Instant::now();
    let params = SampleParams {
        num_reads: 4,
        num_sweeps: sweeps,
        beta_range: Some((1.0, 12.0)),
        seed,
        ..SampleParams::default()
    };
    let config = NativeConfig {
        shared_cache: true,
        copy_resampling: true,
        inline_union: true,
        fused_updates: true,
        archive: true,
        polish_sweeps: 0,
        resamples: (sweeps / 16).max(1),
        ..NativeConfig::default()
    };
    let reads = tuned_reads(instance, &params, &config, || false)
        .map_err(|error| format!("probe: {error:?}"))?
        .ok_or("unexpected probe cancellation")?;
    let energies: Vec<_> = reads
        .iter()
        .map(|read| {
            let canonical = instance_energy(instance, &read.spins);
            if canonical != read.energy_milli + instance.offset_milli() {
                return Err("probe candidate energy failed canonical verification");
            }
            Ok(canonical)
        })
        .collect::<Result<_, _>>()?;
    Ok(Probe {
        reads: params.num_reads,
        sweeps,
        best_milli: energies.iter().min().copied().ok_or("empty probe")?,
        mean_milli: energies.iter().sum::<i64>() as f64 / energies.len() as f64,
        seconds: started.elapsed().as_secs_f64(),
    })
}

#[expect(
    clippy::print_stdout,
    reason = "emit one JSON record for each instance"
)]
fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let topology: Topology = serde_json::from_slice(&std::fs::read(&args.topology)?)?;
    let edges: Vec<_> = topology.edges.into_iter().map(|[u, v]| (u, v)).collect();
    let topology = BaseTopology::new(topology.nodes, &edges).map_err(|e| format!("{e:?}"))?;
    let params = std::fs::read(&args.params)?;
    let revision = reference_revision(TWISTED_DIMER_V1.family_id, 1).ok_or("missing revision")?;
    let nonce = blake3::hash(&args.seed.to_le_bytes());
    let generation = Instant::now();
    let instances = export_instances(revision.contract, &params, &topology, &[*nonce.as_bytes()])?;
    let instance = instances
        .into_iter()
        .next()
        .ok_or("missing instance")?
        .map_err(|e| format!("salt generation: {e:?}"))?;
    let generation_seconds = generation.elapsed().as_secs_f64();
    if instance.h().is_empty() || instance.h().iter().any(|&h| h != 0) {
        return Err("ranking study requires a nonempty zero-field graph".into());
    }

    let preparation = Instant::now();
    let rows = adjacency(instance.h().len(), instance.edges(), instance.j());
    let adjacency_seconds = preparation.elapsed().as_secs_f64();
    let structural = structure(&rows);
    let preparation = Instant::now();
    let ising = Ising::new(
        &vec![0; rows.len()],
        instance.edges(),
        &instance
            .j()
            .iter()
            .map(|&j| i64::from(j))
            .collect::<Vec<_>>(),
    )?;
    let greedy = Greedy::new(ising, &[], true)?;
    let greedy_seconds = preparation.elapsed().as_secs_f64();

    let seed = args.seed ^ 0x424d_5f52_414e_4b31;
    let (rank2, _) = relax::<2>(&instance, &rows, &greedy, 1.0, seed)?;
    let (rank4, starts) = relax::<4>(&instance, &rows, &greedy, 1.0, seed)?;
    let (rank4_weak3, _) = relax::<4>(&instance, &rows, &greedy, 3.0, seed)?;
    let short = probe(&instance, 64, args.seed ^ 0x4350_5542_454e_4348)?;
    let medium = probe(&instance, 256, args.seed ^ 0x4350_5542_454e_4348)?;

    if let Some(path) = &args.save_starts {
        serde_json::to_writer(
            std::fs::File::create_new(path)?,
            &serde_json::json!({
                "instance_digest": instance.digest(),
                "reads": starts.into_iter().map(|(energy, spins)| {
                    (energy - instance.offset_milli(), spins)
                }).collect::<Vec<_>>(),
            }),
        )?;
    }

    println!(
        "{}",
        serde_json::json!({
            "seed": args.seed, "nonce": nonce.to_hex().to_string(),
            "generation_seconds": generation_seconds,
            "adjacency_seconds": adjacency_seconds, "greedy_setup_seconds": greedy_seconds,
            "structure": structural, "rank2": rank2, "rank4": rank4, "rank4_weak3": rank4_weak3,
            "probe4x64": short, "probe4x256": medium,
        })
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weak_triangle_sign_and_rank_four_sweep_local_descent() {
        let rows = adjacency(4, &[(0, 1), (1, 2), (0, 2), (2, 3)], &[45, 45, 45, -75]);
        let s = structure(&rows);
        assert_eq!(s.frustrated_weak_triangles, 1);
        assert_eq!(s.weak_triangles, 1);
        let rows = adjacency(4, &[(0, 1), (1, 2), (0, 2), (2, 3)], &[-45, 45, 45, -75]);
        assert_eq!(structure(&rows).frustrated_weak_triangles, 0);
        let mut rng = SmallRng::seed_from_u64(438);
        let mut v = initialize::<4>(rows.len(), &mut rng);
        for _ in 0..SWEEPS {
            let before = vector_energy(&v, &rows, 1.0);
            sweep(&mut v, &rows, 1.0);
            assert!(vector_energy(&v, &rows, 1.0) <= before + 1.0e-8);
            assert!(v.iter().all(|u| (dot(u, u) - 1.0).abs() < 1.0e-12));
        }
    }
}
