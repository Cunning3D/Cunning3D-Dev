//! Offset adjustment: port of Blender adjust_offsets / adjust_the_cycle_or_chain (3709–3967).
//! Iteratively adjusts boundary vertex positions for width consistency along connected edges.
use super::super::structures::BevelGraph;
use super::boundary::BoundVertLite;
use bevy::prelude::*;

const BEVEL_EPSILON: f32 = 1e-6;
const MAX_ADJUST_ITER: usize = 10;

/// Chain/cycle of boundary vertices for adjustment.
#[derive(Clone, Debug)]
pub struct AdjustChain {
    pub verts: Vec<usize>, // Indices into bnd_verts
    pub is_cycle: bool,
}

/// Blender adjust_the_cycle_or_chain (3709-3868): adjust widths along a chain or cycle.
/// Uses iterative solver to make offset widths consistent.
pub fn adjust_the_cycle_or_chain(
    bnd_verts: &mut [BoundVertLite],
    chain: &AdjustChain,
    edge_lengths: &[f32],
) {
    let n = chain.verts.len();
    if n < 2 {
        return;
    }

    // Try fast adjustment first
    if adjust_chain_fast(bnd_verts, chain, edge_lengths) {
        return;
    }

    // Fall back to iterative solver
    adjust_chain_iterative(bnd_verts, chain, edge_lengths);
}

/// Fast adjustment for simple cases (Blender adjust_the_cycle_or_chain_fast 3490-3571).
fn adjust_chain_fast(
    bnd_verts: &mut [BoundVertLite],
    chain: &AdjustChain,
    edge_lengths: &[f32],
) -> bool {
    let n = chain.verts.len();
    if n < 2 {
        return true;
    }

    // Collect current sin ratios
    let mut ratios: Vec<f32> = chain
        .verts
        .iter()
        .map(|&vi| bnd_verts.get(vi).map(|v| v.sinratio).unwrap_or(1.0))
        .collect();

    // Check if all ratios are close to 1 (simple case)
    let all_unity = ratios.iter().all(|&r| (r - 1.0).abs() < 0.01);
    if all_unity {
        return true;
    }

    // For cycles, try to balance
    if chain.is_cycle {
        let total: f32 = ratios.iter().sum();
        let avg = total / n as f32;
        if (avg - 1.0).abs() < BEVEL_EPSILON {
            // Can balance exactly
            for (i, &vi) in chain.verts.iter().enumerate() {
                if let Some(v) = bnd_verts.get_mut(vi) {
                    v.sinratio = avg;
                    ratios[i] = avg;
                }
            }
            return true;
        }
    }

    false
}

/// Iterative adjustment solver (Blender 3709+).
fn adjust_chain_iterative(
    bnd_verts: &mut [BoundVertLite],
    chain: &AdjustChain,
    _edge_lengths: &[f32],
) {
    let n = chain.verts.len();
    if n < 3 {
        return;
    }

    // Blender uses matrix solve for exact, we use simple relaxation
    for _iter in 0..MAX_ADJUST_ITER {
        let mut max_delta: f32 = 0.0;

        for i in 1..(n - 1) {
            let vi = chain.verts[i];
            let vi_prev = chain.verts[i - 1];
            let vi_next = chain.verts[i + 1];

            let (r_prev, r_next, r_curr) = {
                let prev = bnd_verts.get(vi_prev).map(|v| v.sinratio).unwrap_or(1.0);
                let next = bnd_verts.get(vi_next).map(|v| v.sinratio).unwrap_or(1.0);
                let curr = bnd_verts.get(vi).map(|v| v.sinratio).unwrap_or(1.0);
                (prev, next, curr)
            };

            // Target: smooth interpolation
            let target = (r_prev + r_next) * 0.5;
            let delta = (target - r_curr) * 0.5; // Relaxation factor

            if let Some(v) = bnd_verts.get_mut(vi) {
                v.sinratio += delta;
                max_delta = max_delta.max(delta.abs());
            }
        }

        // For cycles, also adjust first/last
        if chain.is_cycle && n >= 3 {
            let vi_first = chain.verts[0];
            let vi_last = chain.verts[n - 1];
            let vi_second = chain.verts[1];
            let vi_second_last = chain.verts[n - 2];

            let r_first = bnd_verts.get(vi_first).map(|v| v.sinratio).unwrap_or(1.0);
            let r_last = bnd_verts.get(vi_last).map(|v| v.sinratio).unwrap_or(1.0);
            let r_second = bnd_verts.get(vi_second).map(|v| v.sinratio).unwrap_or(1.0);
            let r_second_last = bnd_verts
                .get(vi_second_last)
                .map(|v| v.sinratio)
                .unwrap_or(1.0);

            // Wrap around
            let target_first = (r_last + r_second) * 0.5;
            let target_last = (r_second_last + r_first) * 0.5;

            if let Some(v) = bnd_verts.get_mut(vi_first) {
                let delta = (target_first - r_first) * 0.5;
                v.sinratio += delta;
                max_delta = max_delta.max(delta.abs());
            }
            if let Some(v) = bnd_verts.get_mut(vi_last) {
                let delta = (target_last - r_last) * 0.5;
                v.sinratio += delta;
                max_delta = max_delta.max(delta.abs());
            }
        }

        if max_delta < BEVEL_EPSILON {
            break;
        }
    }
}

/// Find adjustment chains in the boundary (Blender 3871-3955).
/// Chains are sequences of boundary verts connected via paired edges.
pub fn find_adjust_chains(bnd_verts: &[BoundVertLite], graph: &BevelGraph) -> Vec<AdjustChain> {
    let mut chains: Vec<AdjustChain> = Vec::new();
    let mut visited = vec![false; bnd_verts.len()];

    for start_idx in 0..bnd_verts.len() {
        if visited[start_idx] {
            continue;
        }

        let v = &bnd_verts[start_idx];
        // Only start from verts that have edge-on (eon) set
        if v.efirst.is_none() {
            continue;
        }

        // Trace chain in forward direction
        let mut chain_verts: Vec<usize> = vec![start_idx];
        visited[start_idx] = true;

        let mut curr_idx = start_idx;
        let mut is_cycle = false;

        // Forward trace
        loop {
            let curr = &bnd_verts[curr_idx];
            if let Some(e_idx) = curr.efirst {
                // Find paired edge's boundary vert
                if let Some(edge) = graph.edges.get(e_idx) {
                    let pair_idx = edge.pair_index;
                    if let Some(pair_edge) = graph.edges.get(pair_idx) {
                        if let Some(next_bv_idx) = pair_edge.left_v {
                            if next_bv_idx < bnd_verts.len() {
                                if next_bv_idx == start_idx {
                                    is_cycle = true;
                                    break;
                                }
                                if !visited[next_bv_idx] {
                                    visited[next_bv_idx] = true;
                                    chain_verts.push(next_bv_idx);
                                    curr_idx = next_bv_idx;
                                    continue;
                                }
                            }
                        }
                    }
                }
            }
            break;
        }

        if chain_verts.len() >= 2 {
            chains.push(AdjustChain {
                verts: chain_verts,
                is_cycle,
            });
        }
    }

    chains
}

/// Apply adjust_offsets to all chains (Blender adjust_offsets main loop).
pub fn adjust_all_offsets(
    bnd_verts: &mut [BoundVertLite],
    graph: &BevelGraph,
    edge_lengths: &[f32],
) {
    let chains = find_adjust_chains(bnd_verts, graph);
    for chain in &chains {
        adjust_the_cycle_or_chain(bnd_verts, chain, edge_lengths);
    }
}

/// Adjust bevel edge offsets using boundary connectivity (Blender adjust_offsets 3871+).
pub fn adjust_offsets(graph: &mut BevelGraph, max_iter: usize) -> bool {
    if max_iter == 0 || graph.bound_verts.is_empty() {
        return false;
    }
    let mut changed = false;
    for _ in 0..max_iter {
        let mut iter_changed = false;
        // 1) Enforce loop_slide ratio constraints at boundary vertices that lie on an intermediate edge.
        //    Blender: uses bv->sinratio to relate off1_r/off2_l when bv->eon is set.
        for bv in &graph.bound_verts {
            if bv.eon.is_none() {
                continue;
            }
            let (Some(e1), Some(e2)) = (bv.efirst, bv.elast) else {
                continue;
            };
            if e1 >= graph.edges.len() || e2 >= graph.edges.len() {
                continue;
            }
            let sr = bv.sinratio;
            if !sr.is_finite() || sr.abs() < 1e-8 {
                continue;
            }
            let (x, y) = {
                let a = &graph.edges[e1];
                let b = &graph.edges[e2];
                (a.offset_r, b.offset_l)
            };
            let x2 = (x + y / sr) * 0.5;
            let y2 = x2 * sr;
            if (x2 - x).abs() > BEVEL_EPSILON || (y2 - y).abs() > BEVEL_EPSILON {
                iter_changed = true;
            }
            graph.edges[e1].offset_r = x2;
            graph.edges[e2].offset_l = y2;
        }

        // 2) Enforce pair mirror constraints: (e.l == pair.r) and (e.r == pair.l) by averaging once per pair.
        let n = graph.edges.len();
        for ei in 0..n {
            let pair = graph.edges[ei].pair_index;
            if pair >= n || ei >= pair {
                continue;
            }
            let (l, r) = {
                let e = &graph.edges[ei];
                let p = &graph.edges[pair];
                (
                    (e.offset_l + p.offset_r) * 0.5,
                    (e.offset_r + p.offset_l) * 0.5,
                )
            };
            let (e, p) = {
                let (lo, hi) = graph.edges.split_at_mut(pair);
                (&mut lo[ei], &mut hi[0])
            };
            if (e.offset_l - l).abs() > BEVEL_EPSILON || (e.offset_r - r).abs() > BEVEL_EPSILON {
                iter_changed = true;
            }
            e.offset_l = l;
            e.offset_r = r;
            p.offset_l = r;
            p.offset_r = l;
        }
        if !iter_changed {
            break;
        }
        changed |= iter_changed;
    }
    if changed {
        graph.sync_edge_soa();
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_chain() {
        let mut bnd_verts: Vec<BoundVertLite> = vec![];
        let chain = AdjustChain {
            verts: vec![],
            is_cycle: false,
        };
        adjust_the_cycle_or_chain(&mut bnd_verts, &chain, &[]);
        // Should not crash
    }
}
