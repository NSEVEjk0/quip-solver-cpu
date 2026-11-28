mod cut;

use super::{MotifDescentStats, OtsukaGraph, Workspace, FIXED};
use quip_solver_core::SamplerResult;
use rand::{rngs::SmallRng, seq::SliceRandom, SeedableRng};

/// Joint optimization to try after each single-component pass.
#[derive(Clone, Copy, Debug)]
pub enum CoupledMoves {
    /// Use single-component moves only.
    None,
    /// Try the best pair of coupled components.
    Pairs,
    /// Optimize the signed quotient using a roof-duality cut.
    Cut {
        /// Release every such tile in shuffled order. Zero keeps all pairings.
        release_period: usize,
    },
}

/// Budget and move selection for the experimental motif search.
#[derive(Clone, Copy, Debug)]
pub struct PairingOptions {
    /// Number of independent tile pairings to visit.
    pub rounds: usize,
    /// Favor pairings that limit the growth of large components.
    pub balance_sizes: bool,
    /// Joint optimization to try after each single-component pass.
    pub coupled_moves: CoupledMoves,
}

/// Counts and checked energy gain for the selected read.
#[derive(Debug, Default)]
pub struct PairingStats {
    /// Number of partitions evaluated.
    pub partitions: usize,
    /// Sum of component counts over the partitions.
    pub component_count_sum: usize,
    /// Minimum of the largest component sizes.
    pub smallest_max_component: usize,
    /// Sum of the largest component sizes.
    pub max_component_sum: usize,
    /// Largest component encountered.
    pub largest_component: usize,
    /// Strictly improving single-component flips.
    pub single_moves: usize,
    /// Strictly improving coupled pairs.
    pub joint_moves: usize,
    /// Strictly improving completed cut assignments.
    pub cut_moves: usize,
    /// Conditional quotients with a feasible assignment attaining the cut bound.
    pub certified_quotients: usize,
    /// Most free original spins in a certified quotient.
    pub largest_certified_region: usize,
    /// Most free motif components in a certified quotient.
    pub largest_certified_quotient: usize,
    /// Sum of the number of labeled original spins, excluding the anchor.
    pub cut_labeled_spins_sum: usize,
    /// Sum of the feasible completions' energy gaps to their cut bounds.
    pub cut_gap_sum: i64,
    /// Decrease in strong energy in milli-units.
    pub strong_gain_milli: i64,
    /// Decrease in the original energy in milli-units.
    pub gain_milli: i64,
}

/// Improve the best read using partitions of the original objective.
/// Returns `None` on cancellation or inconsistent input or output energies.
pub fn polish_pairings(
    graph: &OtsukaGraph,
    reads: &mut [SamplerResult],
    options: PairingOptions,
    seed: u64,
    stopped: &impl Fn() -> bool,
) -> Option<PairingStats> {
    if stopped()
        || reads.iter().any(|read| {
            read.spins.len() != graph.n
                || read.spins.iter().any(|&s| s != -1 && s != 1)
                || read.energy_milli != graph.energy(&read.spins)
        })
    {
        return None;
    }
    let mut stats = PairingStats::default();
    let Some(read) = reads.iter_mut().min_by_key(|r| r.energy_milli) else {
        return Some(stats);
    };
    let initial = read.energy_milli;
    let strong = graph.full_strong_energy(&read.spins);
    let mut workspace = Workspace::new(graph.n);
    let release_period = match options.coupled_moves {
        CoupledMoves::Cut { release_period } => release_period,
        _ => 0,
    };
    let mut balance = (options.balance_sizes || release_period > 0)
        .then(|| Balance::new(graph.n, graph.tiles.len()));
    let mut edges = Vec::new();
    let mut rng = SmallRng::seed_from_u64(seed ^ 0x5041_4952_5f53_5243);
    let mut moves = MotifDescentStats::default();
    for _ in 0..options.rounds {
        if stopped() {
            return None;
        }
        moves.max_component = 0;
        match &mut balance {
            Some(balance) => balance.partition(
                &mut workspace,
                graph,
                &read.spins,
                &mut rng,
                &mut moves,
                release_period,
            ),
            None => workspace.partition(graph, &read.spins, None, &mut rng, &mut moves),
        }
        let largest = moves.max_component;
        if stats.partitions == 0 {
            stats.smallest_max_component = largest;
        }
        stats.smallest_max_component = stats.smallest_max_component.min(largest);
        stats.max_component_sum += largest;
        stats.largest_component = stats.largest_component.max(largest);
        stats.component_count_sum += workspace.deltas.len();
        stats.partitions += 1;
        read.energy_milli += workspace.descend(graph, &mut read.spins, &mut rng, &mut moves);
        match options.coupled_moves {
            CoupledMoves::None => {}
            CoupledMoves::Pairs => {
                quotient(&workspace, graph, &read.spins, &mut edges);
                if let Some((a, b, expected)) = best_joint(&workspace, &edges) {
                    let first = workspace.flip(a, graph, &mut read.spins);
                    let second = workspace.flip(b, graph, &mut read.spins);
                    read.energy_milli += first + second;
                    if first + second != expected {
                        return None;
                    }
                    stats.joint_moves += 1;
                }
            }
            CoupledMoves::Cut { .. } => {
                quotient(&workspace, graph, &read.spins, &mut edges);
                improve_cut(&mut workspace, graph, read, &edges, &mut stats)?;
            }
        }
    }
    stats.single_moves = moves.accepted;
    stats.gain_milli = initial - read.energy_milli;
    stats.strong_gain_milli = strong - graph.full_strong_energy(&read.spins);
    (read.energy_milli == graph.energy(&read.spins)
        && (release_period > 0 || stats.strong_gain_milli == 0)
        && stats.gain_milli >= 0)
        .then_some(stats)
}

fn improve_cut(
    workspace: &mut Workspace,
    graph: &OtsukaGraph,
    read: &mut SamplerResult,
    bonds: &[(usize, usize, i64)],
    stats: &mut PairingStats,
) -> Option<()> {
    let sizes = || workspace.offsets.windows(2).map(|w| w[1] - w[0]);
    let (anchor, largest) = sizes().enumerate().max_by_key(|&(_, size)| size)?;
    let count = workspace.deltas.len();
    let cut = cut::roof_dual(count, anchor, bonds)?;
    stats.cut_gap_sum += cut.gap;
    stats.cut_labeled_spins_sum += sizes()
        .enumerate()
        .filter(|&(i, _)| i != anchor && cut.labeled[i])
        .map(|(_, size)| size)
        .sum::<usize>();
    if cut.gap == 0 {
        stats.certified_quotients += 1;
        stats.largest_certified_region = stats.largest_certified_region.max(graph.n - largest);
        stats.largest_certified_quotient = stats.largest_certified_quotient.max(count - 1);
    }
    if cut.delta < 0 {
        let actual = cut
            .flips
            .iter()
            .enumerate()
            .filter(|&(_, &flip)| flip)
            .map(|(i, _)| workspace.flip(i, graph, &mut read.spins))
            .sum::<i64>();
        read.energy_milli += actual;
        if actual != cut.delta {
            return None;
        }
        stats.cut_moves += 1;
    }
    Some(())
}

struct Balance {
    sizes: Vec<usize>,
    tile_order: Vec<usize>,
}

impl Balance {
    fn new(n: usize, tiles: usize) -> Self {
        Self {
            sizes: vec![1; n],
            tile_order: (0..tiles).collect(),
        }
    }

    fn join(sizes: &mut [usize], workspace: &mut Workspace, a: usize, b: usize) {
        let a = workspace.root(a);
        let b = workspace.root(b);
        if a != b {
            let (lo, hi) = (a.min(b), a.max(b));
            workspace.parents[hi] = lo;
            sizes[lo] += sizes[hi];
        }
    }

    fn cost(sizes: &[usize], [a, b, c, d]: [usize; 4]) -> u64 {
        // Half the increase in the sum of squared component sizes.
        let first = if a == b {
            0
        } else {
            sizes[a] as u64 * sizes[b] as u64
        };
        let merged = if a == b {
            sizes[a]
        } else {
            sizes[a] + sizes[b]
        };
        let c = if c == b { a } else { c };
        let d = if d == b { a } else { d };
        let size = |r| if r == a { merged } else { sizes[r] } as u64;
        first + if c == d { 0 } else { size(c) * size(d) }
    }

    fn partition(
        &mut self,
        workspace: &mut Workspace,
        graph: &OtsukaGraph,
        spins: &[i8],
        rng: &mut SmallRng,
        stats: &mut MotifDescentStats,
        release_period: usize,
    ) {
        self.sizes.fill(1);
        for (u, parent) in workspace.parents.iter_mut().enumerate() {
            *parent = u;
        }
        for &(a, b) in &graph.pairs {
            Self::join(&mut self.sizes, workspace, a as usize, b as usize);
        }
        self.tile_order.shuffle(rng);
        for (at, &index) in self.tile_order.iter().enumerate() {
            if release_period != 0 && (at + 1).is_multiple_of(release_period) {
                continue;
            }
            let tile = &graph.tiles[index];
            let mut by_sign = [[0; 4]; 2];
            let mut count = [0; 2];
            for (&node, &gauge) in tile.nodes.iter().zip(&tile.gauge) {
                let u = node as usize;
                let sign = usize::from(spins[u] == gauge);
                by_sign[sign][count[sign]] = u;
                count[sign] += 1;
            }
            let paired = count[0].min(count[1]);
            if paired > 0 {
                by_sign[0][..count[0]].shuffle(rng);
                by_sign[1][..count[1]].shuffle(rng);
            }
            if paired == 2 {
                let [a, b, c, d] = [by_sign[0][0], by_sign[1][0], by_sign[0][1], by_sign[1][1]]
                    .map(|u| workspace.root(u));
                if Self::cost(&self.sizes, [a, d, c, b]) < Self::cost(&self.sizes, [a, b, c, d]) {
                    by_sign[1].swap(0, 1);
                }
            }
            let [negative, positive] = &by_sign;
            for (&a, &b) in negative[..paired].iter().zip(&positive[..paired]) {
                Self::join(&mut self.sizes, workspace, a, b);
            }
            for (nodes, count) in by_sign.iter().zip(count) {
                if let Some((&first, rest)) = nodes[paired..count].split_first() {
                    for &u in rest {
                        Self::join(&mut self.sizes, workspace, first, u);
                    }
                }
            }
        }
        workspace.materialize(graph, spins, stats);
    }
}

fn quotient(
    workspace: &Workspace,
    graph: &OtsukaGraph,
    spins: &[i8],
    edges: &mut Vec<(usize, usize, i64)>,
) {
    edges.clear();
    for (u, &a) in workspace.labels.iter().enumerate() {
        if a == FIXED {
            continue;
        }
        for &edge in graph.all.row(u) {
            let v = edge.node();
            let b = workspace.labels[v];
            if u < v && a != b && b != FIXED {
                let signed = i64::from(spins[u]) * i64::from(edge.j()) * i64::from(spins[v]);
                edges.push((a.min(b), a.max(b), signed));
            }
        }
    }
    edges.sort_unstable_by_key(|&(a, b, _)| (a, b));
    let mut read = 0;
    let mut used = 0;
    while read < edges.len() {
        let (a, b, mut weight) = edges[read];
        read += 1;
        while read < edges.len() && (edges[read].0, edges[read].1) == (a, b) {
            weight += edges[read].2;
            read += 1;
        }
        edges[used] = (a, b, weight);
        used += 1;
    }
    edges.truncate(used);
}

fn best_joint(workspace: &Workspace, edges: &[(usize, usize, i64)]) -> Option<(usize, usize, i64)> {
    let mut best = None;
    let mut lowest = 0;
    for &(a, b, cross) in edges {
        let delta = workspace.deltas[a] + workspace.deltas[b] + 4 * cross;
        if delta < lowest {
            lowest = delta;
            best = Some((a, b, delta));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::families::otsuka::{Model, Motifs, Weights};

    #[test]
    fn joint_move_can_improve_when_all_single_moves_are_uphill() {
        let graph = OtsukaGraph::new(
            &Model {
                h_milli: &[0; 4],
                edges: &[(0, 1), (0, 2), (1, 3), (2, 3)],
                j_milli: &[-80, 50, 50, -900],
            },
            &Motifs {
                tiles: &[],
                gauges: &[],
                pairs: &[],
            },
            Weights::FULL,
        )
        .unwrap();
        let spins = vec![1; 4];
        let start = graph.energy(&spins);
        let mut reads = [SamplerResult {
            spins,
            energy_milli: start,
        }];
        let stats = polish_pairings(
            &graph,
            &mut reads,
            PairingOptions {
                rounds: 1,
                balance_sizes: true,
                coupled_moves: CoupledMoves::Pairs,
            },
            68,
            &|| false,
        )
        .unwrap();
        assert_eq!(stats.single_moves, 0);
        assert_eq!(stats.joint_moves, 1);
        assert_eq!(stats.gain_milli, 200);
        assert_eq!(reads[0].energy_milli, start - 200);
    }
}
