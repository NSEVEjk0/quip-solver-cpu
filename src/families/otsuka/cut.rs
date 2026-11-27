//! Conditional ground states on balanced induced neighborhoods.

use quip_solver_core::SamplerResult;
use rand::{rngs::SmallRng, seq::SliceRandom, SeedableRng};

use super::{graph::Link, OtsukaGraph};

const UNASSIGNED: usize = usize::MAX;
const MAX_BLOCK: usize = 128;
const PARTITIONS: usize = 16;
const POLISHED_READS: usize = 8;

struct Bond {
    u: usize,
    v: usize,
    magnitude: i64,
}

struct Block {
    nodes: Vec<usize>,
    gauge: Vec<i8>,
    bonds: Vec<Bond>,
    boundary: Vec<Vec<Link>>,
}

impl Block {
    fn partition(
        graph: &OtsukaGraph,
        order: &[usize],
        stopped: &impl Fn() -> bool,
    ) -> Option<Vec<Self>> {
        let mut blocks = Vec::new();
        let mut block_of = vec![UNASSIGNED; graph.n];
        let mut rejected_by = vec![UNASSIGNED; graph.n];
        let mut index_of = vec![0; graph.n];
        let mut assigned_gauge = vec![0; graph.n];

        for &root in order {
            if block_of[root] != UNASSIGNED {
                continue;
            }
            if stopped() {
                return None;
            }
            let id = blocks.len();
            let mut nodes = vec![root];
            block_of[root] = id;
            assigned_gauge[root] = 1;
            let mut next = 0;
            while next < nodes.len() && nodes.len() < MAX_BLOCK {
                let u = nodes[next];
                next += 1;
                for &link in graph.all.row(u) {
                    let v = link.node();
                    if link.j() == 0 || block_of[v] != UNASSIGNED || rejected_by[v] == id {
                        continue;
                    }
                    let gauge = -assigned_gauge[u] * link.j().signum() as i8;
                    let conflicts = graph.all.row(v).iter().any(|edge| {
                        let w = edge.node();
                        block_of[w] == id
                            && edge.j() != 0
                            && gauge * assigned_gauge[w] * edge.j().signum() as i8 != -1
                    });
                    if conflicts {
                        rejected_by[v] = id;
                        continue;
                    }
                    block_of[v] = id;
                    index_of[v] = nodes.len();
                    assigned_gauge[v] = gauge;
                    nodes.push(v);
                    if nodes.len() == MAX_BLOCK {
                        break;
                    }
                }
            }

            let mut bonds = Vec::new();
            let mut boundary = Vec::with_capacity(nodes.len());
            for (u, &node) in nodes.iter().enumerate() {
                let mut outside = Vec::new();
                for &link in graph.all.row(node) {
                    if link.j() == 0 {
                        continue;
                    }
                    let other = link.node();
                    if block_of[other] != id {
                        outside.push(link);
                    } else if u < index_of[other] {
                        bonds.push(Bond {
                            u,
                            v: index_of[other],
                            magnitude: i64::from(link.j()).abs(),
                        });
                    }
                }
                boundary.push(outside);
            }
            let gauge = nodes.iter().map(|&u| assigned_gauge[u]).collect();
            blocks.push(Self {
                nodes,
                gauge,
                bonds,
                boundary,
            });
        }
        Some(blocks)
    }

    fn descend(
        &self,
        spins: &mut [i8],
        cut: &mut Cut,
        stopped: &impl Fn() -> bool,
    ) -> Option<bool> {
        let n = self.nodes.len();
        let mut field = [0_i64; MAX_BLOCK];
        let mut before = [0_i8; MAX_BLOCK];
        for (i, (&u, outside)) in self.nodes.iter().zip(&self.boundary).enumerate() {
            field[i] = i64::from(self.gauge[i])
                * outside
                    .iter()
                    .map(|&link| i64::from(link.j()) * i64::from(spins[link.node()]))
                    .sum::<i64>();
            before[i] = self.gauge[i] * spins[u];
        }
        cut.minimize(&field[..n], &self.bonds, stopped)?;
        let mut after = [0_i8; MAX_BLOCK];
        let mut delta = 0;
        for i in 0..n {
            after[i] = if cut.level[i] == UNASSIGNED { -1 } else { 1 };
            delta += field[i] * i64::from(after[i] - before[i]);
        }
        for &Bond { u, v, magnitude } in &self.bonds {
            delta -= magnitude * i64::from(after[u] * after[v] - before[u] * before[v]);
        }
        if delta < 0 {
            for (i, &u) in self.nodes.iter().enumerate() {
                spins[u] = self.gauge[i] * after[i];
            }
        }
        Some(delta < 0)
    }
}

#[derive(Clone, Copy)]
struct Arc {
    to: usize,
    reverse: usize,
    capacity: i64,
}

struct Cut {
    adjacent: Vec<Vec<Arc>>,
    level: Vec<usize>,
    next: Vec<usize>,
    queue: Vec<usize>,
}

impl Cut {
    fn new() -> Self {
        let n = MAX_BLOCK + 2;
        Self {
            adjacent: vec![Vec::new(); n],
            level: vec![UNASSIGNED; n],
            next: vec![0; n],
            queue: Vec::with_capacity(n),
        }
    }

    fn add(&mut self, u: usize, v: usize, forward: i64, backward: i64) {
        let reverse = self.adjacent[u].len();
        let opposite = self.adjacent[v].len();
        self.adjacent[u].push(Arc {
            to: v,
            reverse: opposite,
            capacity: forward,
        });
        self.adjacent[v].push(Arc {
            to: u,
            reverse,
            capacity: backward,
        });
    }

    fn levels(&mut self, source: usize) {
        self.level.fill(UNASSIGNED);
        self.level[source] = 0;
        self.queue.clear();
        self.queue.push(source);
        let mut next = 0;
        while let Some(&u) = self.queue.get(next) {
            next += 1;
            for edge in &self.adjacent[u] {
                if edge.capacity > 0 && self.level[edge.to] == UNASSIGNED {
                    self.level[edge.to] = self.level[u] + 1;
                    self.queue.push(edge.to);
                }
            }
        }
    }

    fn send(&mut self, u: usize, sink: usize, capacity: i64) -> i64 {
        if u == sink {
            return capacity;
        }
        while let Some(&edge) = self.adjacent[u].get(self.next[u]) {
            if edge.capacity > 0 && self.level[edge.to] == self.level[u] + 1 {
                let delivered = self.send(edge.to, sink, capacity.min(edge.capacity));
                if delivered > 0 {
                    self.adjacent[u][self.next[u]].capacity -= delivered;
                    self.adjacent[edge.to][edge.reverse].capacity += delivered;
                    return delivered;
                }
            }
            self.next[u] += 1;
        }
        0
    }

    /// Minimize the gauged Ising energy. Source-reachable vertices are +1.
    fn minimize(
        &mut self,
        field: &[i64],
        bonds: &[Bond],
        stopped: &impl Fn() -> bool,
    ) -> Option<()> {
        let source = field.len();
        let sink = source + 1;
        for row in &mut self.adjacent[..=sink] {
            row.clear();
        }
        for (u, &h) in field.iter().enumerate() {
            if h < 0 {
                self.add(source, u, -2 * h, 0);
            } else if h > 0 {
                self.add(u, sink, 2 * h, 0);
            }
        }
        for &Bond { u, v, magnitude } in bonds {
            self.add(u, v, 2 * magnitude, 2 * magnitude);
        }
        loop {
            if stopped() {
                return None;
            }
            self.levels(source);
            if self.level[sink] == UNASSIGNED {
                return Some(());
            }
            self.next[..=sink].fill(0);
            while self.send(source, sink, i64::MAX) > 0 {}
        }
    }
}

pub(crate) fn polish(
    graph: &OtsukaGraph,
    reads: &mut [SamplerResult],
    seed: u64,
    stopped: &impl Fn() -> bool,
) -> Option<()> {
    let mut rng = SmallRng::seed_from_u64(seed ^ 0x4355_545f_504f_4c31);
    let mut order: Vec<usize> = (0..graph.n).collect();
    let mut selected: Vec<usize> = (0..reads.len()).collect();
    selected.sort_by_key(|&i| reads[i].energy_milli);
    selected.truncate(POLISHED_READS);
    let mut cut = Cut::new();
    let (mut cyclic_blocks, mut largest, mut accepted) = (0, 0, 0);
    for _ in 0..PARTITIONS {
        order.shuffle(&mut rng);
        let blocks = Block::partition(graph, &order, stopped)?;
        cyclic_blocks += blocks
            .iter()
            .filter(|b| b.bonds.len() >= b.nodes.len())
            .count();
        largest = largest.max(blocks.iter().map(|b| b.nodes.len()).max().unwrap_or(0));
        for &i in &selected {
            for block in &blocks {
                accepted += usize::from(block.descend(&mut reads[i].spins, &mut cut, stopped)?);
            }
        }
    }
    for i in selected {
        reads[i].energy_milli = graph.energy(&reads[i].spins);
    }
    tracing::debug!(
        cyclic_blocks,
        largest,
        accepted,
        "CPU conditional graph cuts"
    );
    Some(())
}

#[cfg(test)]
mod tests {
    use super::{Block, Cut};
    use crate::families::otsuka::{Model, Motifs, OtsukaGraph, Weights};
    use rand::{rngs::SmallRng, Rng, SeedableRng};

    #[test]
    fn every_induced_block_matches_exhaustive_conditional_optimization() {
        let n = 8;
        let edges: Vec<_> = (0..n)
            .flat_map(|u| (u + 1..n).map(move |v| (u, v)))
            .collect();
        let order: Vec<_> = (0..n as usize).collect();
        let mut rng = SmallRng::seed_from_u64(38);
        let mut cut = Cut::new();
        let mut used_cycle = false;
        for _ in 0..64 {
            let j: Vec<_> = edges.iter().map(|_| rng.gen_range(-3..=3)).collect();
            let graph = OtsukaGraph::new(
                &Model {
                    h_milli: &[0; 8],
                    edges: &edges,
                    j_milli: &j,
                },
                &Motifs {
                    tiles: &[],
                    gauges: &[],
                    pairs: &[],
                },
                Weights {
                    tile_milli: 250,
                    pair_milli: 250,
                },
            )
            .unwrap();
            for block in Block::partition(&graph, &order, &|| false).unwrap() {
                used_cycle |= block.bonds.len() >= block.nodes.len();
                let mut spins: Vec<i8> = (0..n).map(|_| if rng.gen() { 1 } else { -1 }).collect();
                let before = spins.clone();
                let best = (0..1_usize << block.nodes.len())
                    .map(|mask| {
                        for (i, &u) in block.nodes.iter().enumerate() {
                            spins[u] = if mask & (1 << i) == 0 { -1 } else { 1 };
                        }
                        graph.energy(&spins)
                    })
                    .min()
                    .unwrap();
                spins.clone_from(&before);
                let changed = block.descend(&mut spins, &mut cut, &|| false).unwrap();
                assert_eq!(graph.energy(&spins), best);
                assert_eq!(changed, best < graph.energy(&before));
                if !changed {
                    assert_eq!(spins, before);
                }
                for u in 0..n as usize {
                    if !block.nodes.contains(&u) {
                        assert_eq!(spins[u], before[u]);
                    }
                }
            }
        }
        assert!(used_cycle);
    }
}
