type Bond = (usize, usize, i64);

pub(super) struct Cut {
    pub(super) flips: Vec<bool>,
    pub(super) labeled: Vec<bool>,
    pub(super) delta: i64,
    pub(super) gap: i64,
}

/// Return a feasible completed assignment and its gap to the cut bound.
/// Variables and their complements are separate vertices. Every consistent
/// cut has the same cost as the signed objective plus a fixed constant.
pub(super) fn roof_dual(count: usize, anchor: usize, bonds: &[Bond]) -> Option<Cut> {
    if anchor >= count {
        return None;
    }
    let source = 2 * count;
    let sink = source + 1;
    let mut graph = Flow::new(sink + 1);
    let mut magnitude = 0;
    let mut incumbent_cost = 0;
    for &(a, b, weight) in bonds {
        debug_assert!(a < b && b < count);
        if weight < 0 {
            graph.add(a, b, -weight, -weight);
            graph.add(a + count, b + count, -weight, -weight);
        } else if weight > 0 {
            graph.add(a, b + count, weight, weight);
            graph.add(b, a + count, weight, weight);
            incumbent_cost += 2 * weight;
        }
        magnitude += weight.abs();
    }
    // The anchor chooses one representative of the global spin inversion.
    let pin = 2 * magnitude + 1;
    graph.add(source, anchor, pin, 0);
    graph.add(anchor + count, sink, pin, 0);
    let (bound, reachable) = graph.minimum_cut(source, sink);
    let cut_cost = bonds
        .iter()
        .map(|&(a, b, weight)| {
            if weight < 0 {
                -weight
                    * (i64::from(reachable[a] != reachable[b])
                        + i64::from(reachable[a + count] != reachable[b + count]))
            } else {
                weight
                    * (i64::from(reachable[a] != reachable[b + count])
                        + i64::from(reachable[b] != reachable[a + count]))
            }
        })
        .sum::<i64>();
    let mut flips = Vec::with_capacity(count);
    let mut labeled = Vec::with_capacity(count);
    for at in 0..count {
        flips.push(!reachable[at] && reachable[at + count]);
        labeled.push(reachable[at] != reachable[at + count]);
    }
    let delta = bonds
        .iter()
        .filter(|&&(a, b, _)| flips[a] != flips[b])
        .map(|&(_, _, weight)| -2 * weight)
        .sum::<i64>();
    let gap = incumbent_cost + delta - bound;
    (!flips[anchor] && labeled[anchor] && gap >= 0 && cut_cost == bound).then_some(Cut {
        flips,
        labeled,
        delta,
        gap,
    })
}

#[derive(Clone, Copy)]
struct Arc {
    to: usize,
    reverse: usize,
    capacity: i64,
}

struct Flow {
    links: Vec<Vec<Arc>>,
}

impl Flow {
    fn new(nodes: usize) -> Self {
        Self {
            links: vec![Vec::new(); nodes],
        }
    }

    fn add(&mut self, a: usize, b: usize, ab: i64, ba: i64) {
        let a_slot = self.links[a].len();
        let b_slot = self.links[b].len();
        self.links[a].push(Arc {
            to: b,
            reverse: b_slot,
            capacity: ab,
        });
        self.links[b].push(Arc {
            to: a,
            reverse: a_slot,
            capacity: ba,
        });
    }

    fn minimum_cut(&mut self, source: usize, sink: usize) -> (i64, Vec<bool>) {
        let nodes = self.links.len();
        let mut level = vec![usize::MAX; nodes];
        let mut queue = Vec::with_capacity(nodes);
        let mut cursor = vec![0; nodes];
        let mut path: Vec<(usize, usize)> = Vec::new();
        let mut flow = 0;
        loop {
            level.fill(usize::MAX);
            level[source] = 0;
            queue.clear();
            queue.push(source);
            let mut at = 0;
            while at < queue.len() {
                let node = queue[at];
                for arc in &self.links[node] {
                    if arc.capacity > 0 && level[arc.to] == usize::MAX {
                        level[arc.to] = level[node] + 1;
                        queue.push(arc.to);
                    }
                }
                at += 1;
            }
            if level[sink] == usize::MAX {
                return (flow, level.into_iter().map(|v| v != usize::MAX).collect());
            }
            cursor.fill(0);
            path.clear();
            let mut u = source;
            loop {
                if u == sink {
                    let sent = path
                        .iter()
                        .map(|&(v, edge)| self.links[v][edge].capacity)
                        .min()
                        .expect("source and sink are distinct");
                    for &(v, edge) in &path {
                        let arc = self.links[v][edge];
                        self.links[v][edge].capacity -= sent;
                        self.links[arc.to][arc.reverse].capacity += sent;
                    }
                    flow += sent;
                    path.clear();
                    u = source;
                    continue;
                }
                while let Some(arc) = self.links[u].get(cursor[u]) {
                    if arc.capacity > 0 && level[arc.to] == level[u] + 1 {
                        break;
                    }
                    cursor[u] += 1;
                }
                if let Some(arc) = self.links[u].get(cursor[u]) {
                    path.push((u, cursor[u]));
                    u = arc.to;
                } else if let Some((parent, _)) = path.pop() {
                    cursor[parent] += 1;
                    u = parent;
                } else {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::SmallRng, Rng, SeedableRng};

    #[test]
    fn cut_bounds_exact_enumeration_and_certifies_integral_answers() {
        let mut rng = SmallRng::seed_from_u64(0x4355_54);
        let n = 6;
        let mut certificates = 0;
        for _ in 0..128 {
            let mut bonds = Vec::new();
            for a in 0..n {
                for b in (a + 1)..n {
                    let weight = rng.gen_range(-6..=6) * 45;
                    if weight != 0 {
                        bonds.push((a, b, weight));
                    }
                }
            }
            let cutoff = roof_dual(n, 0, &bonds).unwrap();
            let exact = (0..(1 << n))
                .step_by(2)
                .map(|mask| {
                    bonds
                        .iter()
                        .filter(|&&(a, b, _)| (mask >> a) & 1 != (mask >> b) & 1)
                        .map(|&(_, _, w)| -2 * w)
                        .sum::<i64>()
                })
                .min()
                .unwrap();
            let obtained = bonds
                .iter()
                .filter(|&&(a, b, _)| cutoff.flips[a] != cutoff.flips[b])
                .map(|&(_, _, w)| -2 * w)
                .sum::<i64>();
            assert_eq!(cutoff.delta, obtained);
            assert!(cutoff.delta - cutoff.gap <= exact && exact <= cutoff.delta);
            if cutoff.gap == 0 {
                assert_eq!(cutoff.delta, exact);
                certificates += 1;
            }
        }
        assert!(certificates > 0);
    }
}
