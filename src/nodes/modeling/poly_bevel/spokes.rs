use crate::libs::geometry::ids::{HalfEdgeId, PointId};
use crate::libs::geometry::topology::Topology;

#[inline]
fn around_prev(topo: &Topology, he: HalfEdgeId) -> HalfEdgeId {
    let pair = topo.pair(he);
    if pair.is_valid() {
        topo.next(pair)
    } else {
        HalfEdgeId::INVALID
    }
}

#[inline]
fn around_next(topo: &Topology, he: HalfEdgeId) -> HalfEdgeId {
    let prev = topo.prev(he);
    let pair = topo.pair(prev);
    if pair.is_valid() {
        pair
    } else {
        HalfEdgeId::INVALID
    }
}

/// Collect outgoing half-edges around `p` in a complete fan (handles boundary vertices).
pub fn spoke_fan(topo: &Topology, p: PointId) -> Vec<HalfEdgeId> {
    let Some(&start) = topo.point_to_halfedge.get(&p) else {
        return Vec::new();
    };
    if !start.is_valid() {
        return Vec::new();
    }
    if topo
        .half_edges
        .get(start.into())
        .map(|he| he.origin_point != p)
        .unwrap_or(true)
    {
        return Vec::new();
    }
    let mut first = start;
    let mut it = 0usize;
    loop {
        let prev = around_prev(topo, first);
        if !prev.is_valid() || prev == start || it > 512 {
            break;
        }
        first = prev;
        it += 1;
    }
    let mut out = Vec::new();
    let mut curr = first;
    let mut it2 = 0usize;
    loop {
        if !curr.is_valid() || it2 > 1024 {
            break;
        }
        out.push(curr);
        let nxt = around_next(topo, curr);
        if !nxt.is_valid() || nxt == first {
            break;
        }
        curr = nxt;
        it2 += 1;
    }
    out
}

