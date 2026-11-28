use std::{
    cmp::Reverse,
    collections::{BinaryHeap, VecDeque},
};

type Bond = (usize, usize, i64);
type EliminationOrder = Vec<(usize, Vec<usize>)>;
const MAX_WIDTH: usize = 10;

pub(super) struct Exact {
    pub(super) flips: Vec<bool>,
    pub(super) delta: i64,
    pub(super) width: usize,
    pub(super) fixed: usize,
}

struct Step {
    node: usize,
    later: Vec<usize>,
    value: Vec<i64>,
    choice: Vec<bool>,
}

fn sign(flip: bool) -> i64 {
    if flip {
        -1
    } else {
        1
    }
}

fn dominating_spins(
    weighted: &[Vec<(usize, i64)>],
    anchor: usize,
) -> (Vec<Option<bool>>, Vec<i64>) {
    let n = weighted.len();
    let mut fixed = vec![None; n];
    let mut field = vec![0_i64; n];
    let mut radius: Vec<i64> = weighted
        .iter()
        .map(|row| row.iter().map(|&(_, w)| w.abs()).sum())
        .collect();
    let mut candidates: VecDeque<_> = (0..n).filter(|&u| u != anchor).collect();
    candidates.push_front(anchor);
    while let Some(node) = candidates.pop_front() {
        if fixed[node].is_some() || (node != anchor && field[node].abs() < radius[node]) {
            continue;
        }
        // Once the field dominates the entire free neighborhood, this
        // sign minimizes the local term for every neighboring assignment.
        let flipped = node != anchor && field[node] > 0;
        fixed[node] = Some(flipped);
        for &(other, w) in &weighted[node] {
            if fixed[other].is_none() {
                field[other] += sign(flipped) * w;
                radius[other] -= w.abs();
                candidates.push_back(other);
            }
        }
    }
    (fixed, field)
}

fn elimination_order(
    weighted: &[Vec<(usize, i64)>],
    fixed: &[Option<bool>],
) -> Result<Option<EliminationOrder>, ()> {
    let mut adjacent: Vec<Vec<usize>> = weighted
        .iter()
        .map(|row| {
            row.iter()
                .filter_map(|&(j, _)| fixed[j].is_none().then_some(j))
                .collect()
        })
        .collect();
    let mut removed: Vec<_> = fixed.iter().map(Option::is_some).collect();
    let mut queue: BinaryHeap<_> = adjacent
        .iter()
        .enumerate()
        .filter_map(|(u, row)| (!removed[u]).then_some(Reverse((row.len(), u))))
        .collect();
    let mut order = Vec::new();
    while let Some(Reverse((degree, u))) = queue.pop() {
        if removed[u] || adjacent[u].len() != degree {
            continue;
        }
        if degree > MAX_WIDTH {
            return Ok(None);
        }
        removed[u] = true;
        let row = std::mem::take(&mut adjacent[u]);
        for &v in &row {
            let old = adjacent[v].iter().position(|&w| w == u).ok_or(())?;
            adjacent[v].swap_remove(old);
            for &w in &row {
                if w != v && !adjacent[v].contains(&w) {
                    adjacent[v].push(w);
                }
            }
            queue.push(Reverse((adjacent[v].len(), v)));
        }
        order.push((u, row));
    }
    Ok(Some(order))
}

/// Minimize the full anchored signed quotient by dominance and variable
/// elimination. Return `Ok(None)` when the elimination order exceeds its cap.
pub(super) fn minimize(count: usize, anchor: usize, bonds: &[Bond]) -> Result<Option<Exact>, ()> {
    if anchor >= count {
        return Err(());
    }
    let mut weighted = vec![Vec::new(); count];
    for &(a, b, w) in bonds {
        if w != 0 {
            weighted[a].push((b, w));
            weighted[b].push((a, w));
        }
    }
    let (fixed, field) = dominating_spins(&weighted, anchor);
    let Some(order) = elimination_order(&weighted, &fixed)? else {
        return Ok(None);
    };
    let mut rank = vec![usize::MAX; count];
    for (i, &(node, _)) in order.iter().enumerate() {
        rank[node] = i;
    }
    let mut inbox = vec![Vec::<usize>::new(); order.len()];
    let mut steps = Vec::<Step>::with_capacity(order.len());
    let mut optimum = bonds
        .iter()
        .filter_map(|&(a, b, w)| Some(w * sign(fixed[a]?) * sign(fixed[b]?)))
        .sum::<i64>();
    let mut width = 0;
    for (at, (node, mut later)) in order.into_iter().enumerate() {
        later.sort_unstable_by_key(|&u| rank[u]);
        width = width.max(later.len());
        let terms: Vec<_> = later
            .iter()
            .enumerate()
            .filter_map(|(bit, &u)| {
                weighted[node]
                    .iter()
                    .find(|&&(v, _)| v == u)
                    .map(|&(_, w)| (bit, w))
            })
            .collect();
        let factors = inbox[at]
            .iter()
            .map(|&i| {
                let step = &steps[i];
                if step.later.first() != Some(&node) {
                    return Err(());
                }
                let projection = step.later[1..]
                    .iter()
                    .map(|&u| later.iter().position(|&v| v == u).ok_or(()))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((&step.value, projection))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let size = 1 << later.len();
        let mut value = Vec::with_capacity(size);
        let mut choice = Vec::with_capacity(size);
        for assignment in 0..size {
            let field = field[node]
                + terms
                    .iter()
                    .map(|&(bit, w)| sign(assignment & (1 << bit) != 0) * w)
                    .sum::<i64>();
            let (mut keep, mut flip) = (field, -field);
            for (factor, projection) in &factors {
                let selected = projection.iter().enumerate().fold(0, |ind, (bit, &src)| {
                    ind | (((assignment >> src) & 1) << (bit + 1))
                });
                keep += factor[selected];
                flip += factor[selected | 1];
            }
            value.push(keep.min(flip));
            choice.push(flip < keep);
        }
        if let Some(&receiver) = later.first() {
            inbox[rank[receiver]].push(at);
        } else {
            optimum += value[0];
        }
        steps.push(Step {
            node,
            later,
            value,
            choice,
        });
    }
    let mut flips: Vec<_> = fixed.iter().map(|s| s.unwrap_or(false)).collect();
    for step in steps.iter().rev() {
        let assignment = step.later.iter().enumerate().fold(0, |mask, (bit, &node)| {
            mask | (usize::from(flips[node]) << bit)
        });
        flips[step.node] = step.choice[assignment];
    }
    let achieved = bonds
        .iter()
        .map(|&(a, b, w)| w * sign(flips[a]) * sign(flips[b]))
        .sum::<i64>();
    let incumbent = bonds.iter().map(|&(_, _, w)| w).sum::<i64>();
    if achieved != optimum || achieved > incumbent {
        return Err(());
    }
    Ok(Some(Exact {
        flips,
        delta: achieved - incumbent,
        width,
        fixed: fixed.iter().filter(|s| s.is_some()).count(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::SmallRng, Rng, SeedableRng};

    #[test]
    fn exact_quotient_matches_exhaustive_signed_assignments() {
        let n = 7;
        let anchor = 3;
        let mut rng = SmallRng::seed_from_u64(0x4d4f_5449);
        for _ in 0..128 {
            let mut bonds = Vec::new();
            for a in 0..n {
                for b in (a + 1)..n {
                    let w = rng.gen_range(-8..=8) * 45;
                    bonds.push((a, b, w));
                }
            }
            let expected = (0..(1 << n))
                .filter(|mask| mask & (1 << anchor) == 0)
                .map(|mask| {
                    bonds
                        .iter()
                        .filter(|&&(a, b, _)| (mask >> a) & 1 != (mask >> b) & 1)
                        .map(|&(_, _, w)| -2 * w)
                        .sum::<i64>()
                })
                .min()
                .unwrap();
            let solved = minimize(n, anchor, &bonds).unwrap().unwrap();
            assert_eq!(solved.delta, expected);
            assert!(!solved.flips[anchor]);
        }
    }
}
