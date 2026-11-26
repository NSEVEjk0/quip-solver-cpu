//! Exact conditional ground states on induced trees.

use rand::{rngs::SmallRng, seq::SliceRandom, SeedableRng};

use super::graph::OtsukaGraph;

const NO_VERTEX: u32 = u32::MAX;
const PARTITIONS: usize = 16;
const PASSES: usize = 16;
const INITIAL_READS: usize = 8;
const CROSSOVERS: usize = 4;

#[derive(Clone, Copy)]
struct Vertex {
    node: u32,
    parent: u32,
    coupling: i32,
}

struct Tree {
    vertices: Vec<Vertex>,
    ends: Vec<usize>,
}

impl Tree {
    fn partition(graph: &OtsukaGraph, order: &[u32], stopped: &impl Fn() -> bool) -> Option<Self> {
        let mut vertices = Vec::with_capacity(order.len());
        let mut ends = Vec::new();
        let mut assigned = vec![false; graph.n];
        let mut found = vec![false; graph.n];
        let mut touches = vec![0_u8; graph.n];
        let mut parent = vec![0_u32; graph.n];
        let mut coupling = vec![0_i32; graph.n];
        let mut queue = Vec::<u32>::new();
        let mut touched = Vec::new();
        for (index, &root) in order.iter().enumerate() {
            if index.is_multiple_of(256) && stopped() {
                return None;
            }
            if assigned[root as usize] {
                continue;
            }
            let start = vertices.len();
            assigned[root as usize] = true;
            vertices.push(Vertex {
                node: root,
                parent: NO_VERTEX,
                coupling: 0,
            });
            let mut scan = start;
            let mut next = 0;
            loop {
                while scan < vertices.len() {
                    if scan.is_multiple_of(256) && stopped() {
                        return None;
                    }
                    for &link in graph.all.row(vertices[scan].node as usize) {
                        let v = link.node();
                        if assigned[v] || link.j() == 0 {
                            continue;
                        }
                        if !found[v] {
                            found[v] = true;
                            touched.push(v);
                            queue.push(v as u32);
                            parent[v] = scan as u32;
                            coupling[v] = link.j();
                        }
                        touches[v] = (touches[v] + 1).min(2);
                    }
                    scan += 1;
                }
                while let Some(&candidate) = queue.get(next) {
                    next += 1;
                    let v = candidate as usize;
                    if touches[v] == 1 {
                        assigned[v] = true;
                        vertices.push(Vertex {
                            node: candidate,
                            parent: parent[v],
                            coupling: coupling[v],
                        });
                        break;
                    }
                }
                if scan == vertices.len() {
                    break;
                }
            }
            ends.push(vertices.len());
            queue.clear();
            for v in touched.drain(..) {
                found[v] = false;
                touches[v] = 0;
            }
        }
        Some(Self { vertices, ends })
    }

    /// Solve each tree in order, updating the fields seen by later trees.
    fn descend(
        &self,
        graph: &OtsukaGraph,
        spins: &mut [i8],
        scratch: &mut Scratch,
        stopped: &impl Fn() -> bool,
    ) -> Option<bool> {
        let mut changed = false;
        let mut start = 0;
        for &end in &self.ends {
            if stopped() {
                return None;
            }
            for (offset, vertex) in self.vertices[start..end].iter().enumerate() {
                if offset.is_multiple_of(256) && stopped() {
                    return None;
                }
                let old = i64::from(spins[vertex.node as usize]);
                scratch.cost[start + offset] = [0, -2 * old * scratch.field[vertex.node as usize]];
            }
            // Flip-relative labels give the all-keep assignment cost zero.
            // For XOR along an internal edge, the both-flip cost corrects
            // the two single-flip field terms by four times its old energy.
            for at in (start + 1..end).rev() {
                let vertex = self.vertices[at];
                let parent = vertex.parent as usize;
                let old_edge = i64::from(vertex.coupling)
                    * i64::from(spins[vertex.node as usize])
                    * i64::from(spins[self.vertices[parent].node as usize]);
                let [keep, flip] = scratch.cost[at];
                let flip_with_parent = flip + 4 * old_edge;
                let choices = [flip < keep, flip_with_parent < keep];
                scratch.choice[at] = choices;
                scratch.cost[parent][0] += keep.min(flip);
                scratch.cost[parent][1] += keep.min(flip_with_parent);
            }
            let [keep, flip] = scratch.cost[start];
            if keep.min(flip) < 0 {
                changed = true;
                scratch.selected[start] = flip < keep;
                for at in start + 1..end {
                    let parent = self.vertices[at].parent as usize;
                    scratch.selected[at] =
                        scratch.choice[at][usize::from(scratch.selected[parent])];
                }
                for at in start..end {
                    if scratch.selected[at] {
                        let u = self.vertices[at].node as usize;
                        let old = i64::from(spins[u]);
                        spins[u] = -spins[u];
                        for &link in graph.all.row(u) {
                            scratch.field[link.node()] -= 2 * old * i64::from(link.j());
                        }
                    }
                }
            }
            start = end;
        }
        Some(changed)
    }
}

struct Scratch {
    field: Vec<i64>,
    cost: Vec<[i64; 2]>,
    choice: Vec<[bool; 2]>,
    selected: Vec<bool>,
}

impl Scratch {
    fn new(n: usize) -> Self {
        Self {
            field: vec![0; n],
            cost: vec![[0; 2]; n],
            choice: vec![[false; 2]; n],
            selected: vec![false; n],
        }
    }

    fn reset(&mut self, graph: &OtsukaGraph, spins: &[i8]) {
        for (u, field) in self.field.iter_mut().enumerate() {
            *field = graph
                .all
                .row(u)
                .iter()
                .map(|link| i64::from(link.j()) * i64::from(spins[link.node()]))
                .sum();
        }
    }
}

pub(crate) struct TreePolish<'a> {
    graph: &'a OtsukaGraph,
    partitions: Vec<Tree>,
    scratch: Scratch,
}

impl<'a> TreePolish<'a> {
    pub(crate) fn new(
        graph: &'a OtsukaGraph,
        seed: u64,
        stopped: &impl Fn() -> bool,
    ) -> Option<Self> {
        let mut rng = SmallRng::seed_from_u64(seed ^ 0x5452_4545_504f_4c31);
        let mut order: Vec<u32> = (0..graph.n as u32).collect();
        let mut partitions = Vec::with_capacity(PARTITIONS);
        for _ in 0..PARTITIONS {
            order.shuffle(&mut rng);
            partitions.push(Tree::partition(graph, &order, stopped)?);
        }
        Some(Self {
            graph,
            partitions,
            scratch: Scratch::new(graph.n),
        })
    }

    fn descend(&mut self, spins: &mut [i8], stopped: &impl Fn() -> bool) -> Option<()> {
        self.scratch.reset(self.graph, spins);
        for _ in 0..PASSES {
            let mut changed = false;
            for partition in &self.partitions {
                changed |= partition.descend(self.graph, spins, &mut self.scratch, stopped)?;
            }
            if !changed {
                break;
            }
        }
        Some(())
    }

    /// A disagreement component can copy its donor spins independently.
    /// Other disagreement components have no edges into it.
    fn combine(&mut self, a: &mut [i8], b: &[i8], stopped: &impl Fn() -> bool) -> Option<()> {
        let mut visited = vec![false; a.len()];
        let mut pending = Vec::new();
        for start in 0..a.len() {
            if start.is_multiple_of(256) && stopped() {
                return None;
            }
            if visited[start] || a[start] == b[start] {
                continue;
            }
            visited[start] = true;
            pending.push(start);
            let mut at = 0;
            let mut delta = 0;
            while at < pending.len() {
                if at.is_multiple_of(256) && stopped() {
                    return None;
                }
                let u = pending[at];
                for &link in self.graph.all.row(u) {
                    let v = link.node();
                    if a[v] == b[v] {
                        delta -= 2 * i64::from(a[u]) * i64::from(link.j()) * i64::from(a[v]);
                    } else if !visited[v] && link.j() != 0 {
                        visited[v] = true;
                        pending.push(v);
                    }
                }
                at += 1;
            }
            if delta < 0 {
                for u in pending.drain(..) {
                    a[u] = b[u];
                }
            } else {
                pending.clear();
            }
        }
        Some(())
    }

    pub(crate) fn improve(
        &mut self,
        reads: &mut [quip_solver_core::SamplerResult],
        stopped: &impl Fn() -> bool,
    ) -> Option<()> {
        let mut order: Vec<_> = (0..reads.len()).collect();
        order.sort_by_key(|&i| reads[i].energy_milli);
        for &index in order.iter().take(INITIAL_READS) {
            let read = &mut reads[index];
            self.descend(&mut read.spins, stopped)?;
            read.energy_milli = self.graph.energy(&read.spins);
        }
        order.sort_by_key(|&i| reads[i].energy_milli);
        let (&best, donors) = order.split_first()?;
        for &donor in donors.iter().take(CROSSOVERS) {
            let mut candidate = reads[best].spins.clone();
            self.combine(&mut candidate, &reads[donor].spins, stopped)?;
            self.descend(&mut candidate, stopped)?;
            let energy_milli = self.graph.energy(&candidate);
            if energy_milli < reads[best].energy_milli {
                reads[best] = quip_solver_core::SamplerResult {
                    spins: candidate,
                    energy_milli,
                };
            }
        }
        Some(())
    }
}
