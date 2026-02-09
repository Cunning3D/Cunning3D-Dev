use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::mesh::{GeoPrimitive, Geometry, PolygonPrim};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use bevy::prelude::Vec3;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct RemeshNode;

impl NodeParameters for RemeshNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "target_edge_length",
                "Target Edge Length",
                "Construct",
                ParameterValue::Float(0.1),
                ParameterUIType::FloatSlider {
                    min: 0.001,
                    max: 10.0,
                },
            ),
            Parameter::new(
                "iterations",
                "Iterations",
                "Construct",
                ParameterValue::Int(5),
                ParameterUIType::IntSlider { min: 0, max: 50 },
            ),
            Parameter::new(
                "smooth_strength",
                "Smooth Strength",
                "Construct",
                ParameterValue::Float(0.5),
                ParameterUIType::FloatSlider { min: 0.0, max: 1.0 },
            ),
        ]
    }
}

#[inline]
fn tri_fan(out: &mut Vec<[usize; 3]>, poly_pts: &[usize]) {
    if poly_pts.len() < 3 {
        return;
    }
    let p0 = poly_pts[0];
    for i in 1..poly_pts.len() - 1 {
        out.push([p0, poly_pts[i], poly_pts[i + 1]]);
    }
}

fn build_triangles(geo: &Geometry) -> (Vec<Vec3>, Vec<[usize; 3]>) {
    let Some(pos) = geo
        .get_point_attribute(attrs::P)
        .and_then(|a| a.as_slice::<Vec3>())
    else {
        return (Vec::new(), Vec::new());
    };
    if pos.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut tris = Vec::new();
    for prim in geo.primitives().values() {
        let GeoPrimitive::Polygon(poly) = prim else {
            continue;
        };
        let mut pts = Vec::with_capacity(poly.vertices.len());
        let mut ok = true;
        for &vid in &poly.vertices {
            let Some(v) = geo.vertices().get(vid.into()) else {
                ok = false;
                break;
            };
            let Some(pidx) = geo.points().get_dense_index(v.point_id.into()) else {
                ok = false;
                break;
            };
            pts.push(pidx);
        }
        if ok {
            tri_fan(&mut tris, &pts);
        }
    }
    (pos.to_vec(), tris)
}

#[inline]
fn build_weld_rep(pos: &[Vec3], eps: f32) -> Vec<usize> {
    if pos.is_empty() || !(eps > 0.0) { return (0..pos.len()).collect(); }
    let inv = 1.0 / eps;
    let mut map: HashMap<(i64, i64, i64), usize> = HashMap::with_capacity(pos.len());
    let mut rep = vec![0usize; pos.len()];
    for (i, &p) in pos.iter().enumerate() {
        let k = (
            (p.x * inv).round() as i64,
            (p.y * inv).round() as i64,
            (p.z * inv).round() as i64,
        );
        rep[i] = *map.entry(k).or_insert(i);
    }
    rep
}

#[inline]
fn snap_welded(pos: &mut [Vec3], rep: &[usize]) {
    if pos.is_empty() || rep.len() != pos.len() { return; }
    let n = pos.len();
    let mut sum = vec![Vec3::ZERO; n];
    let mut cnt = vec![0u32; n];
    for i in 0..n {
        let r = rep[i].min(n - 1);
        sum[r] += pos[i];
        cnt[r] = cnt[r].saturating_add(1);
    }
    for i in 0..n {
        let r = rep[i].min(n - 1);
        let c = cnt[r].max(1) as f32;
        pos[i] = sum[r] / c;
    }
}

#[inline]
fn bbox_diag(pos: &[Vec3]) -> f32 {
    if pos.is_empty() { return 0.0; }
    let mut mn = Vec3::splat(f32::INFINITY);
    let mut mx = Vec3::splat(f32::NEG_INFINITY);
    for &p in pos {
        mn = mn.min(p);
        mx = mx.max(p);
    }
    (mx - mn).length()
}

#[inline]
fn orient_tris_consistent(pos: &[Vec3], tris: &mut [[usize; 3]], weld_eps: f32) {
    if pos.is_empty() || tris.is_empty() || !(weld_eps > 0.0) { return; }
    let rep = build_weld_rep(pos, weld_eps);
    #[derive(Clone, Copy)]
    struct EdgeInfo { tri: usize, u: usize, v: usize }
    let mut edge_map: HashMap<(usize, usize), Vec<EdgeInfo>> = HashMap::with_capacity(tris.len() * 3 / 2);
    let mut adj: Vec<Vec<(usize, u8)>> = vec![Vec::new(); tris.len()]; // parity: 0=same flip, 1=opposite flip
    for (ti, [a, b, c]) in tris.iter().copied().enumerate() {
        let (ra, rb, rc) = (rep[a], rep[b], rep[c]);
        let edges = [(ra, rb), (rb, rc), (rc, ra)];
        for (u, v) in edges {
            if u == v { continue; }
            let key = if u < v { (u, v) } else { (v, u) };
            if let Some(prevs) = edge_map.get(&key) {
                for prev in prevs {
                    let same_dir = prev.u == u && prev.v == v;
                    let parity = if same_dir { 1u8 } else { 0u8 };
                    adj[ti].push((prev.tri, parity));
                    adj[prev.tri].push((ti, parity));
                }
            }
            edge_map.entry(key).or_insert_with(Vec::new).push(EdgeInfo { tri: ti, u, v });
        }
    }
    let mut flip: Vec<Option<bool>> = vec![None; tris.len()];
    let mut comp: Vec<usize> = vec![usize::MAX; tris.len()];
    let mut q: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    let mut comp_idx: usize = 0;
    for s in 0..tris.len() {
        if flip[s].is_some() { continue; }
        let cur_comp = comp_idx;
        comp_idx = comp_idx.saturating_add(1);
        flip[s] = Some(false);
        comp[s] = cur_comp;
        q.push_back(s);
        while let Some(i) = q.pop_front() {
            let fi = flip[i].unwrap_or(false);
            for &(j, parity) in &adj[i] {
                let want = if parity == 0 { fi } else { !fi };
                match flip[j] {
                    None => { flip[j] = Some(want); comp[j] = cur_comp; q.push_back(j); }
                    Some(cur) => { let _ = cur == want; } // ignore conflicts (non-manifold) instead of panicking
                }
            }
        }
    }
    for (ti, f) in flip.into_iter().enumerate() {
        if f.unwrap_or(false) {
            let [a, b, c] = tris[ti];
            tris[ti] = [a, c, b];
        }
    }

    // If the mesh is split into multiple shells (UV seam / non-manifold), the above only enforces
    // *internal* edge consistency. We still need to pick a globally "outward" orientation per shell.
    let mut centroid = Vec3::ZERO;
    for &p in pos { centroid += p; }
    centroid /= pos.len().max(1) as f32;
    let mut score: Vec<f64> = vec![0.0; comp_idx.max(1)];
    for (ti, [a, b, c]) in tris.iter().copied().enumerate() {
        let ci = comp.get(ti).copied().unwrap_or(0);
        if ci == usize::MAX || ci >= score.len() { continue; }
        let pa = pos.get(a).copied().unwrap_or(Vec3::ZERO);
        let pb = pos.get(b).copied().unwrap_or(Vec3::ZERO);
        let pc = pos.get(c).copied().unwrap_or(Vec3::ZERO);
        let n = (pb - pa).cross(pc - pa);
        let area2 = n.length() as f64;
        if area2 <= 0.0 { continue; }
        let ctr = (pa + pb + pc) * (1.0 / 3.0);
        let dir = ctr - centroid;
        score[ci] += (n.dot(dir) as f64) * area2; // weight by area^2 for stability
    }
    // swap pass with correct temp (keep it branchless & clear)
    for (ti, t) in tris.iter_mut().enumerate() {
        let ci = comp.get(ti).copied().unwrap_or(0);
        if ci != usize::MAX && ci < score.len() && score[ci] < 0.0 {
            let [aa, bb, cc] = *t;
            *t = [aa, cc, bb];
        }
    }
}

fn remesh_triangles(
    mut positions: Vec<Vec3>,
    mut tris: Vec<[usize; 3]>,
    target: f32,
    iters: u32,
    smooth: f32,
    weld_eps: f32,
) -> (Vec<Vec3>, Vec<[usize; 3]>) {
    if positions.is_empty() || tris.is_empty() {
        return (positions, tris);
    }
    let target = target.max(1e-6);
    let mut rep = build_weld_rep(&positions, weld_eps);

    for _ in 0..iters {
        let mut edge_mid: HashMap<(usize, usize), usize> = HashMap::new();
        let mut new_tris = Vec::with_capacity(tris.len());
        let mut any_split = false;

        let mid = |a: usize,
                       b: usize,
                       pos: &mut Vec<Vec3>,
                       rep: &mut Vec<usize>,
                       map: &mut HashMap<(usize, usize), usize>|
         -> usize {
            let ra = rep.get(a).copied().unwrap_or(a);
            let rb = rep.get(b).copied().unwrap_or(b);
            let (i, j) = if ra < rb { (ra, rb) } else { (rb, ra) };
            if let Some(&k) = map.get(&(i, j)) {
                return k;
            }
            let p = (pos[i] + pos[j]) * 0.5;
            let k = pos.len();
            pos.push(p);
            rep.push(k);
            map.insert((i, j), k);
            k
        };

        for [a, b, c] in tris.into_iter() {
            let pa = positions[a];
            let pb = positions[b];
            let pc = positions[c];
            let l0 = (pa - pb).length();
            let l1 = (pb - pc).length();
            let l2 = (pc - pa).length();
            if l0.max(l1).max(l2) <= target {
                new_tris.push([a, b, c]);
                continue;
            }
            any_split = true;
            let ab = mid(a, b, &mut positions, &mut rep, &mut edge_mid);
            let bc = mid(b, c, &mut positions, &mut rep, &mut edge_mid);
            let ca = mid(c, a, &mut positions, &mut rep, &mut edge_mid);
            new_tris.extend_from_slice(&[[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
        }

        tris = new_tris;
        if !any_split {
            break;
        }

        if smooth > 0.0 {
            let mut nbrs: Vec<Vec<usize>> = vec![Vec::new(); positions.len()];
            for [a, b, c] in &tris {
                nbrs[*a].extend_from_slice(&[*b, *c]);
                nbrs[*b].extend_from_slice(&[*a, *c]);
                nbrs[*c].extend_from_slice(&[*a, *b]);
            }
            let old = positions.clone();
            for (i, n) in nbrs.iter_mut().enumerate() {
                if n.is_empty() {
                    continue;
                }
                n.sort_unstable();
                n.dedup();
                let mut avg = Vec3::ZERO;
                for &j in n.iter() {
                    avg += old[j];
                }
                avg /= n.len() as f32;
                positions[i] = old[i].lerp(avg, smooth);
            }
            // Keep UV-seam duplicates welded during relaxation so they can't "open" into cracks.
            snap_welded(&mut positions, &rep);
        }
    }

    (positions, tris)
}

fn triangles_to_geo(positions: Vec<Vec3>, tris: Vec<[usize; 3]>) -> Geometry {
    let mut geo = Geometry::new();
    if positions.is_empty() || tris.is_empty() {
        return geo;
    }
    let mut pids = Vec::with_capacity(positions.len());
    for _ in 0..positions.len() {
        pids.push(geo.add_point());
    }
    geo.insert_point_attribute(
        attrs::P,
        crate::libs::geometry::mesh::Attribute::new(positions),
    );
    for [a, b, c] in tris {
        let va = geo.add_vertex(pids[a]);
        let vb = geo.add_vertex(pids[b]);
        let vc = geo.add_vertex(pids[c]);
        geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim {
            vertices: vec![va, vb, vc],
        }));
    }
    geo.calculate_smooth_normals();
    geo
}

impl NodeOp for RemeshNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let Some(input0) = inputs.get(0) else {
            return Arc::new(Geometry::new());
        };
        let input = input0.materialize();

        let mut target = 0.1f32;
        let mut iters = 5u32;
        let mut smooth = 0.5f32;
        for p in params {
            match p.name.as_str() {
                "target_edge_length" => {
                    if let ParameterValue::Float(v) = p.value {
                        target = v;
                    }
                }
                "iterations" => {
                    if let ParameterValue::Int(v) = p.value {
                        iters = v as u32;
                    }
                }
                "smooth_strength" => {
                    if let ParameterValue::Float(v) = p.value {
                        smooth = v;
                    }
                }
                _ => {}
            }
        }

        let (pos, tris) = build_triangles(&input);
        if pos.is_empty() || tris.is_empty() {
            return Arc::new(Geometry::new());
        }
        // UV seams often split points with identical/near-identical positions.
        // If we don't weld them, smoothing/remesh can open cracks or even create holes along seams.
        let diag = bbox_diag(&pos);
        let weld_eps = (diag * 1e-6).max(target.abs() * 1e-4).max(1e-6);
        let (pos, mut tris) = remesh_triangles(pos, tris, target, iters, smooth.clamp(0.0, 1.0), weld_eps);
        // Slightly larger epsilon for orientation stitching across UV seams only.
        orient_tris_consistent(&pos, &mut tris, (weld_eps * 10.0).max(weld_eps));
        Arc::new(triangles_to_geo(pos, tris))
    }
}

register_node!("Remesh", "Modeling", RemeshNode);
