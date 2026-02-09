use crate::libs::geometry::group::ElementGroupMask;
use crate::libs::geometry::ids::{AttributeId, PointId, HalfEdgeId};
use crate::libs::geometry::mesh::Geometry;
use crate::libs::geometry::topology::Topology;
use std::collections::{HashMap, HashSet};

pub mod select;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupDomain { Point, Vertex, Primitive, Edge }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupOp { Union, Intersect, Subtract }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromoteMode { All, BoundaryOnly }

#[inline] fn norm_edge(p0: PointId, p1: PointId) -> (PointId, PointId) { if (p0.index, p0.generation) <= (p1.index, p1.generation) { (p0, p1) } else { (p1, p0) } }

/// Houdini-style pattern: "0-10", "1,5,8", "*"
pub fn parse_pattern(pattern: &str, count: usize) -> ElementGroupMask {
    let mut mask = ElementGroupMask::new(count);
    if pattern.trim() == "*" { mask.invert(); return mask; }
    for part in pattern.split_whitespace() {
        for sub in part.split(',') {
            let s = sub.trim();
            if s.is_empty() { continue; }
            if let Some((a, b)) = s.split_once('-') {
                if let (Ok(start), Ok(end)) = (a.parse::<usize>(), b.parse::<usize>()) {
                    let start = start.min(count);
                    let end = (end + 1).min(count + 1);
                    for i in start..end { if i < count { mask.set(i, true); } }
                }
            } else if let Ok(i) = s.parse::<usize>() {
                if i < count { mask.set(i, true); }
            }
        }
    }
    mask
}

#[inline] fn mask_len(geo: &Geometry, d: GroupDomain) -> usize { match d { GroupDomain::Point => geo.points().len(), GroupDomain::Vertex => geo.vertices().len(), GroupDomain::Primitive => geo.primitives().len(), GroupDomain::Edge => geo.edges().len() } }

#[inline] fn get_group<'a>(geo: &'a Geometry, d: GroupDomain, name: &str) -> Option<&'a ElementGroupMask> { match d { GroupDomain::Point => geo.get_point_group(name), GroupDomain::Vertex => geo.get_vertex_group(name), GroupDomain::Primitive => geo.get_primitive_group(name), GroupDomain::Edge => geo.get_edge_group(name) } }
#[inline] fn ensure_group<'a>(geo: &'a mut Geometry, d: GroupDomain, name: &str) -> &'a mut ElementGroupMask { match d { GroupDomain::Point => geo.ensure_point_group(name), GroupDomain::Vertex => geo.ensure_vertex_group(name), GroupDomain::Primitive => geo.ensure_primitive_group(name), GroupDomain::Edge => geo.ensure_edge_group(name) } }

pub fn group_manage(geo: &mut Geometry, domain: GroupDomain, op: &str, from: &str, to: &str) -> bool {
    let from_id = AttributeId::from(from);
    let to_id = AttributeId::from(to);
    match (domain, op) {
        (GroupDomain::Point, "delete") => geo.point_groups.remove(&from_id).is_some(),
        (GroupDomain::Vertex, "delete") => geo.vertex_groups.remove(&from_id).is_some(),
        (GroupDomain::Primitive, "delete") => geo.primitive_groups.remove(&from_id).is_some(),
        (GroupDomain::Edge, "delete") => geo.edge_groups.as_mut().map(|m| m.remove(&from_id).is_some()).unwrap_or(false),
        (GroupDomain::Point, "rename") => geo.point_groups.remove(&from_id).map(|v| geo.point_groups.insert(to_id, v)).is_some(),
        (GroupDomain::Vertex, "rename") => geo.vertex_groups.remove(&from_id).map(|v| geo.vertex_groups.insert(to_id, v)).is_some(),
        (GroupDomain::Primitive, "rename") => geo.primitive_groups.remove(&from_id).map(|v| geo.primitive_groups.insert(to_id, v)).is_some(),
        (GroupDomain::Edge, "rename") => geo.edge_groups.as_mut().and_then(|m| m.remove(&from_id).map(|v| m.insert(to_id, v))).is_some(),
        (d, "copy") => get_group(geo, d, from).cloned().map(|m| { *ensure_group(geo, d, to) = m; true }).unwrap_or(false),
        _ => false,
    }
}

pub fn group_combine(geo: &mut Geometry, domain: GroupDomain, op: GroupOp, a: &str, b: &str, out: &str) -> bool {
    let (Some(ma), Some(mb)) = (get_group(geo, domain, a), get_group(geo, domain, b)) else { return false; };
    let mut r = ma.clone();
    match op { GroupOp::Union => r.union_with(mb), GroupOp::Intersect => r.intersect_with(mb), GroupOp::Subtract => r.difference_with(mb) }
    *ensure_group(geo, domain, out) = r;
    true
}

pub fn group_normalize(geo: &mut Geometry, domain: GroupDomain, drop_empty: bool) {
    let want = mask_len(geo, domain);
    let mut norm = |m: &mut ElementGroupMask| { m.resize(want, false); };
    match domain {
        GroupDomain::Point => { geo.point_groups.values_mut().for_each(&mut norm); if drop_empty { geo.point_groups.retain(|_,m| m.count_ones() != 0); } }
        GroupDomain::Vertex => { geo.vertex_groups.values_mut().for_each(&mut norm); if drop_empty { geo.vertex_groups.retain(|_,m| m.count_ones() != 0); } }
        GroupDomain::Primitive => { geo.primitive_groups.values_mut().for_each(&mut norm); if drop_empty { geo.primitive_groups.retain(|_,m| m.count_ones() != 0); } }
        GroupDomain::Edge => {
            if let Some(groups) = &mut geo.edge_groups {
                groups.values_mut().for_each(&mut norm);
                if drop_empty { groups.retain(|_,m| m.count_ones() != 0); }
            }
        }
    }
}

fn is_manifold_pair_edge(topo: &Topology, he: HalfEdgeId) -> bool {
    let p = topo.pair(he);
    if !p.is_valid() || topo.pair(p) != he { return false; }
    let mut seen = HashSet::with_capacity(4);
    let mut cur = he;
    for _ in 0..4 {
        if !cur.is_valid() || !seen.insert((cur.index, cur.generation)) { break; }
        cur = topo.next_equivalent(cur);
    }
    seen.len() == 2
}

fn ensure_manifold_edge_domain(geo: &mut Geometry) -> HashMap<(PointId, PointId), usize> {
    if !geo.edges().is_empty() { return geo.edges().iter().enumerate().filter_map(|(i,e)| Some((norm_edge(e.p0, e.p1), i))).collect(); }
    let topo = geo.get_topology();
    let mut map = HashMap::new();
    for (he_idx, he) in topo.half_edges.iter_enumerated() {
        let heid = HalfEdgeId::from(he_idx);
        if !is_manifold_pair_edge(&topo, heid) { continue; }
        let p0 = he.origin_point;
        let p1 = topo.dest_point(heid);
        if p0 == p1 { continue; }
        let k = norm_edge(p0, p1);
        if map.contains_key(&k) { continue; }
        let eid = geo.add_edge(k.0, k.1);
        let di = geo.edges().get_dense_index(eid.into()).unwrap_or(usize::MAX);
        if di != usize::MAX { map.insert(k, di); }
    }
    map
}

pub fn promote_mask(geo: &mut Geometry, from: GroupDomain, to: GroupDomain, input: &ElementGroupMask, mode: PromoteMode) -> ElementGroupMask {
    let topo = geo.get_topology();
    match (from, to) {
        (d, d2) if d == d2 => input.clone(),
        (GroupDomain::Point, GroupDomain::Vertex) => {
            let mut out = ElementGroupMask::new(geo.vertices().len());
            for (vi, v) in geo.vertices().iter().enumerate() {
                if let Some(pi) = geo.points().get_dense_index(v.point_id.into()) { if input.get(pi) { out.set(vi, true); } }
            }
            out
        }
        (GroupDomain::Vertex, GroupDomain::Point) => {
            let mut out = ElementGroupMask::new(geo.points().len());
            for i in input.iter_ones() { if let Some(v) = geo.vertices().values().get(i) { if let Some(pi) = geo.points().get_dense_index(v.point_id.into()) { out.set(pi, true); } } }
            out
        }
        (GroupDomain::Primitive, GroupDomain::Point) | (GroupDomain::Primitive, GroupDomain::Vertex) => {
            let mut out = ElementGroupMask::new(if to == GroupDomain::Point { geo.points().len() } else { geo.vertices().len() });
            for prim_i in input.iter_ones() {
                let Some(prim) = geo.primitives().values().get(prim_i) else { continue; };
                for &vid in prim.vertices() {
                    if let Some(v) = geo.vertices().get(vid.into()) {
                        if to == GroupDomain::Point {
                            if let Some(pi) = geo.points().get_dense_index(v.point_id.into()) { out.set(pi, true); }
                        } else if let Some(vi) = geo.vertices().get_dense_index(vid.into()) { out.set(vi, true); }
                    }
                }
            }
            out
        }
        (GroupDomain::Point, GroupDomain::Primitive) => {
            let mut out = ElementGroupMask::new(geo.primitives().len());
            for (pi, prim) in geo.primitives().iter().enumerate() {
                let mut hit = false;
                for &vid in prim.vertices() {
                    let Some(v) = geo.vertices().get(vid.into()) else { continue; };
                    let Some(pdi) = geo.points().get_dense_index(v.point_id.into()) else { continue; };
                    if input.get(pdi) { hit = true; break; }
                }
                if hit { out.set(pi, true); }
            }
            out
        }
        (GroupDomain::Primitive, GroupDomain::Edge) | (GroupDomain::Point, GroupDomain::Edge) => {
            let edge_map = ensure_manifold_edge_domain(geo);
            let mut out = ElementGroupMask::new(geo.edges().len());
            match from {
                GroupDomain::Point => {
                    for (ei, e) in geo.edges().iter().enumerate() {
                        let (p0, p1) = (e.p0, e.p1);
                        let p0i = geo.points().get_dense_index(p0.into()).unwrap_or(usize::MAX);
                        let p1i = geo.points().get_dense_index(p1.into()).unwrap_or(usize::MAX);
                        if (p0i != usize::MAX && input.get(p0i)) || (p1i != usize::MAX && input.get(p1i)) { out.set(ei, true); }
                    }
                }
                _ => {
                    if mode == PromoteMode::BoundaryOnly {
                        for (he_idx, he) in topo.half_edges.iter_enumerated() {
                            let heid = HalfEdgeId::from(he_idx);
                            if !is_manifold_pair_edge(&topo, heid) { continue; }
                            let Some(prim_a) = geo.primitives().get_dense_index(he.primitive_index.into()) else { continue; };
                            let pair = topo.pair(heid);
                            if !pair.is_valid() { continue; }
                            let Some(he_p) = topo.half_edges.get(pair.into()) else { continue; };
                            let Some(prim_b) = geo.primitives().get_dense_index(he_p.primitive_index.into()) else { continue; };
                            if input.get(prim_a) == input.get(prim_b) { continue; }
                            let k = norm_edge(he.origin_point, topo.dest_point(heid));
                            if let Some(&ei) = edge_map.get(&k) { out.set(ei, true); }
                        }
                    } else {
                        for prim_i in input.iter_ones() {
                            let Some(prim) = geo.primitives().values().get(prim_i) else { continue; };
                            let vids = prim.vertices();
                            let n = vids.len();
                            if n < 2 { continue; }
                            let mut pts = Vec::with_capacity(n);
                            for &vid in vids { if let Some(v) = geo.vertices().get(vid.into()) { pts.push(v.point_id); } }
                            for i in 0..n {
                                let k = norm_edge(pts[i], pts[(i + 1) % n]);
                                if let Some(&ei) = edge_map.get(&k) { out.set(ei, true); }
                            }
                        }
                    }
                }
            }
            out
        }
        (GroupDomain::Edge, GroupDomain::Point) => {
            let mut out = ElementGroupMask::new(geo.points().len());
            for ei in input.iter_ones() {
                let Some(e) = geo.edges().values().get(ei) else { continue; };
                for pid in [e.p0, e.p1] {
                    if let Some(pi) = geo.points().get_dense_index(pid.into()) { out.set(pi, true); }
                }
            }
            out
        }
        (GroupDomain::Edge, GroupDomain::Primitive) => {
            let mut he_map: HashMap<(PointId, PointId), HalfEdgeId> = HashMap::new();
            for (he_idx, he) in topo.half_edges.iter_enumerated() {
                let heid = HalfEdgeId::from(he_idx);
                let k = norm_edge(he.origin_point, topo.dest_point(heid));
                he_map.entry(k).or_insert(heid);
            }
            let mut out = ElementGroupMask::new(geo.primitives().len());
            for ei in input.iter_ones() {
                let Some(e) = geo.edges().values().get(ei) else { continue; };
                let Some(&he) = he_map.get(&norm_edge(e.p0, e.p1)) else { continue; };
                let Some(he0) = topo.half_edges.get(he.into()) else { continue; };
                if let Some(di) = geo.primitives().get_dense_index(he0.primitive_index.into()) { out.set(di, true); }
                let p = topo.pair(he);
                if p.is_valid() {
                    if let Some(he1) = topo.half_edges.get(p.into()) {
                        if let Some(di) = geo.primitives().get_dense_index(he1.primitive_index.into()) { out.set(di, true); }
                    }
                }
            }
            out
        }
        _ => ElementGroupMask::new(mask_len(geo, to)),
    }
}

pub fn promote_named_group(geo: &mut Geometry, from: GroupDomain, to: GroupDomain, src: &str, dst: &str, mode: PromoteMode) -> bool {
    let Some(m) = get_group(geo, from, src).cloned() else { return false; };
    let out = promote_mask(geo, from, to, &m, mode);
    *ensure_group(geo, to, dst) = out;
    true
}

