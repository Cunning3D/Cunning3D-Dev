use std::collections::{HashMap, HashSet};
use bevy::prelude::Vec2;
use crate::mesh::{Attribute, Geometry, GeoPrimitive};
use crate::libs::geometry::group::ElementGroupMask;
use crate::libs::geometry::ids::{PointId, VertexId};

#[derive(Default)]
struct Dsu { parent: Vec<usize>, rank: Vec<u8> }
impl Dsu {
    #[inline] fn new(n: usize) -> Self { Self { parent: (0..n).collect(), rank: vec![0; n] } }
    #[inline] fn find(&mut self, x: usize) -> usize { let p=self.parent[x]; if p==x { x } else { let r=self.find(p); self.parent[x]=r; r } }
    #[inline] fn union(&mut self, a: usize, b: usize) { let mut ra=self.find(a); let mut rb=self.find(b); if ra==rb { return; } let (rka,rkb)=(self.rank[ra], self.rank[rb]); if rka<rkb { std::mem::swap(&mut ra,&mut rb); } self.parent[rb]=ra; if rka==rkb { self.rank[ra]=rka.saturating_add(1); } }
}

#[inline] fn norm_pt(p: PointId) -> (u32, u32) { (p.index, p.generation) }
#[inline] fn norm_edge(a: PointId, b: PointId) -> ((u32, u32), (u32, u32)) { let ka=norm_pt(a); let kb=norm_pt(b); if ka <= kb { (ka, kb) } else { (kb, ka) } }
#[inline] fn approx2(a: Vec2, b: Vec2, tol: f32) -> bool { (a - b).abs().max_element() <= tol }
#[inline] fn get_point_of_vertex(geo: &Geometry, vid: VertexId) -> Option<PointId> { geo.vertices().get(vid.into()).map(|v| v.point_id) }
#[inline] fn get_uv(geo: &Geometry, uv_attr: &Attribute, vid: VertexId) -> Option<Vec2> { let di = geo.vertices().get_dense_index(vid.into())?; uv_attr.as_slice::<Vec2>().and_then(|s| s.get(di).copied()).or_else(|| uv_attr.as_paged::<Vec2>().and_then(|pb| pb.get(di))) }

fn primitive_edges(geo: &Geometry, prim_dense: usize) -> Vec<(PointId, PointId, VertexId, VertexId, bool)> {
    let Some(prim_id) = geo.primitives().get_id_from_dense(prim_dense) else { return Vec::new(); };
    let Some(prim) = geo.primitives().get(prim_id) else { return Vec::new(); };
    let vids = prim.vertices();
    if vids.len() < 2 { return Vec::new(); }
    let closed = matches!(prim, GeoPrimitive::Polygon(_)) || matches!(prim, GeoPrimitive::Polyline(p) if p.closed) || matches!(prim, GeoPrimitive::BezierCurve(p) if p.closed);
    let limit = if closed { vids.len() } else { vids.len() - 1 };
    let mut out = Vec::with_capacity(limit);
    for i in 0..limit {
        let v0 = vids[i];
        let v1 = vids[(i + 1) % vids.len()];
        let (Some(p0), Some(p1)) = (get_point_of_vertex(geo, v0), get_point_of_vertex(geo, v1)) else { continue; };
        if p0 == p1 { continue; }
        out.push((p0, p1, v0, v1, closed));
    }
    out
}

pub fn build_seam_edge_set(geo: &Geometry, seam_group: &str) -> HashSet<((u32, u32), (u32, u32))> {
    if seam_group.is_empty() { return HashSet::new(); }
    let Some(mask) = geo.get_edge_group(seam_group) else { return HashSet::new(); };
    let mut set = HashSet::with_capacity(mask.len().min(4096));
    for ei in mask.iter_ones() {
        let Some(eid) = geo.edges().get_id_from_dense(ei) else { continue; };
        let Some(e) = geo.edges().get(eid) else { continue; };
        set.insert(norm_edge(e.p0, e.p1));
    }
    set
}

pub fn connectivity_primitives(geo: &Geometry, selection: &ElementGroupMask, seam_edges: &HashSet<((u32, u32), (u32, u32))>, uv_attr: Option<&Attribute>, uv_tol: f32) -> Vec<i32> {
    let n = geo.primitives().len();
    let mut dsu = Dsu::new(n);
    let mut enabled = vec![false; n];
    for i in selection.iter_ones() { if i < n { enabled[i] = true; } }
    if enabled.iter().all(|v| !*v) { return vec![-1; n]; }

    if seam_edges.is_empty() && uv_attr.is_none() {
        let mut first_by_point: HashMap<(u32, u32), usize> = HashMap::new();
        for pi in 0..n {
            if !enabled[pi] { continue; }
            let Some(pid) = geo.primitives().get_id_from_dense(pi) else { continue; };
            let Some(p) = geo.primitives().get(pid) else { continue; };
            for &vid in p.vertices() {
                let Some(pt) = get_point_of_vertex(geo, vid.into()) else { continue; };
                let key = norm_pt(pt);
                if let Some(&first) = first_by_point.get(&key) { dsu.union(first, pi); } else { first_by_point.insert(key, pi); }
            }
        }
    } else {
        #[derive(Clone, Copy)]
        struct EdgeSeen { prim: usize, uv_a: Option<Vec2>, uv_b: Option<Vec2> }
        let mut seen: HashMap<((u32, u32), (u32, u32)), EdgeSeen> = HashMap::new();
        for pi in 0..n {
            if !enabled[pi] { continue; }
            for (p0, p1, v0, v1, _) in primitive_edges(geo, pi) {
                let key = norm_edge(p0, p1);
                if seam_edges.contains(&key) { continue; }
                let (ka, _) = key;
                let (uv_a, uv_b) = if let Some(ua) = uv_attr {
                    let u0 = get_uv(geo, ua, v0);
                    let u1 = get_uv(geo, ua, v1);
                    let (a, b) = if norm_pt(p0) == ka { (u0, u1) } else { (u1, u0) };
                    (a, b)
                } else { (None, None) };
                if let Some(prev) = seen.get(&key).copied() {
                    let ok = match (prev.uv_a, prev.uv_b, uv_a, uv_b) {
                        (Some(pa), Some(pb), Some(ua), Some(ub)) => approx2(pa, ua, uv_tol) && approx2(pb, ub, uv_tol),
                        _ => true,
                    };
                    if ok { dsu.union(prev.prim, pi); }
                } else { seen.insert(key, EdgeSeen { prim: pi, uv_a, uv_b }); }
            }
        }
    }

    let mut root_to_id: HashMap<usize, i32> = HashMap::new();
    let mut next = 0i32;
    let mut out = vec![-1i32; n];
    for i in 0..n {
        if !enabled[i] { continue; }
        let r = dsu.find(i);
        let id = *root_to_id.entry(r).or_insert_with(|| { let v = next; next += 1; v });
        out[i] = id;
    }
    out
}

pub fn connectivity_points(geo: &Geometry, selection: &ElementGroupMask, seam_edges: &HashSet<((u32, u32), (u32, u32))>, uv_attr: Option<&Attribute>, uv_tol: f32) -> Vec<i32> {
    let pn = geo.points().len();
    if pn == 0 { return Vec::new(); }

    if uv_attr.is_none() {
        let mut dsu = Dsu::new(pn);
        for prim_dense in selection.iter_ones() {
            for (p0, p1, _, _, _) in primitive_edges(geo, prim_dense) {
                let key = norm_edge(p0, p1);
                if seam_edges.contains(&key) { continue; }
                let (Some(i0), Some(i1)) = (geo.points().get_dense_index(p0.into()), geo.points().get_dense_index(p1.into())) else { continue; };
                dsu.union(i0, i1);
            }
        }
        let mut root_to_id: HashMap<usize, i32> = HashMap::new();
        let mut next = 0i32;
        let mut out = vec![0i32; pn];
        for i in 0..pn {
            let r = dsu.find(i);
            out[i] = *root_to_id.entry(r).or_insert_with(|| { let v = next; next += 1; v });
        }
        return out;
    }

    // UV connectivity for Point output is vertex-based; robustly assign each point the minimum component id of its incident vertices.
    let ua = uv_attr.unwrap();
    let vn = geo.vertices().len();
    let mut dsu = Dsu::new(vn);
    let mut first_by_pt_uv: HashMap<((u32, u32), (i32, i32)), usize> = HashMap::new();
    for vdi in 0..vn {
        let Some(vid) = geo.vertices().get_id_from_dense(vdi) else { continue; };
        let Some(pid) = get_point_of_vertex(geo, vid.into()) else { continue; };
        let uv = match get_uv(geo, ua, vid.into()) { Some(v) => v, None => continue };
        let q = if uv_tol > 0.0 { ((uv.x / uv_tol).round() as i32, (uv.y / uv_tol).round() as i32) } else { (uv.x.to_bits() as i32, uv.y.to_bits() as i32) };
        let key = (norm_pt(pid), q);
        if let Some(&first) = first_by_pt_uv.get(&key) { dsu.union(first, vdi); } else { first_by_pt_uv.insert(key, vdi); }
    }
    for prim_dense in selection.iter_ones() {
        for (p0, p1, v0, v1, _) in primitive_edges(geo, prim_dense) {
            let key = norm_edge(p0, p1);
            if seam_edges.contains(&key) { continue; }
            let (Some(i0), Some(i1)) = (geo.vertices().get_dense_index(v0.into()), geo.vertices().get_dense_index(v1.into())) else { continue; };
            dsu.union(i0, i1);
        }
    }
    let mut root_to_id: HashMap<usize, i32> = HashMap::new();
    let mut next = 0i32;
    let mut v_class = vec![0i32; vn];
    for i in 0..vn { let r = dsu.find(i); v_class[i] = *root_to_id.entry(r).or_insert_with(|| { let v = next; next += 1; v }); }

    let mut out = vec![i32::MAX; pn];
    for vdi in 0..vn {
        let Some(vid) = geo.vertices().get_id_from_dense(vdi) else { continue; };
        let Some(pid) = get_point_of_vertex(geo, vid.into()) else { continue; };
        let Some(pi) = geo.points().get_dense_index(pid.into()) else { continue; };
        out[pi] = out[pi].min(v_class[vdi]);
    }
    for v in out.iter_mut() { if *v == i32::MAX { *v = 0; } }
    out
}

