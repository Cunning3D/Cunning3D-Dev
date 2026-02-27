use bevy::prelude::{Vec2, Vec3};
use rustc_hash::{FxHashMap, FxHashSet};
use std::fmt;

use crate::libs::geometry::ids::VertexId;
use crate::mesh::{GeoPrimitive, Geometry};

#[derive(Clone, Copy, Debug)]
pub struct AutoUvSmartProjectOptions {
    pub max_angle_deg: f32,
    /// Chart padding in normalized UV units (0..1).
    pub padding: f32,
}

impl Default for AutoUvSmartProjectOptions {
    fn default() -> Self {
        Self {
            max_angle_deg: 66.0,
            padding: 0.02,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum AutoUvWeighting {
    Uniform,
    /// Cotangent Laplacian weights (clamped to non-negative for solver stability).
    Cotangent,
}

#[derive(Clone, Copy, Debug)]
pub struct AutoUvSmartFlattenOptions {
    pub max_angle_deg: f32,
    /// Chart padding in normalized UV units (0..1).
    pub padding: f32,
    pub weighting: AutoUvWeighting,
    pub solver_tol: f32,
    pub max_solver_iters: u32,
}

impl Default for AutoUvSmartFlattenOptions {
    fn default() -> Self {
        Self {
            max_angle_deg: 66.0,
            padding: 0.02,
            weighting: AutoUvWeighting::Cotangent,
            solver_tol: 1e-6,
            max_solver_iters: 2048,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AutoUvSmartProjectResult {
    pub uvs: Vec<Vec2>,
    pub points: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub charts: usize,
}

#[derive(Debug, Clone)]
pub enum AutoUvError {
    MissingPositions,
    NonFinitePositions,
    NoValidTriangles,
}

impl fmt::Display for AutoUvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPositions => write!(f, "missing @P (point positions)"),
            Self::NonFinitePositions => write!(f, "non-finite point positions detected"),
            Self::NoValidTriangles => write!(f, "no valid polygon triangles to unwrap"),
        }
    }
}

impl std::error::Error for AutoUvError {}

#[derive(Clone, Copy)]
struct Tri {
    pts: [usize; 3],
    verts: [usize; 3],
    normal: Vec3,
    area: f32,
    centroid: Vec3,
    prim_dense: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct EdgeKey(u32, u32);
impl EdgeKey {
    #[inline]
    fn new(a: usize, b: usize) -> Self {
        let (a, b) = (a as u32, b as u32);
        if a <= b {
            Self(a, b)
        } else {
            Self(b, a)
        }
    }
}

#[derive(Default)]
struct Dsu {
    parent: Vec<usize>,
    rank: Vec<u8>,
}
impl Dsu {
    #[inline]
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    #[inline]
    fn find(&mut self, x: usize) -> usize {
        let p = self.parent[x];
        if p == x {
            x
        } else {
            let r = self.find(p);
            self.parent[x] = r;
            r
        }
    }

    #[inline]
    fn union(&mut self, a: usize, b: usize) {
        let mut ra = self.find(a);
        let mut rb = self.find(b);
        if ra == rb {
            return;
        }
        let (rka, rkb) = (self.rank[ra], self.rank[rb]);
        if rka < rkb {
            std::mem::swap(&mut ra, &mut rb);
        }
        self.parent[rb] = ra;
        if rka == rkb {
            self.rank[ra] = rka.saturating_add(1);
        }
    }
}

#[inline]
fn vid_to_point_dense(geo: &Geometry, vid: VertexId) -> Option<usize> {
    let v = geo.vertices().get(vid.into())?;
    geo.points().get_dense_index(v.point_id.into())
}

#[inline]
fn vid_to_vertex_dense(geo: &Geometry, vid: VertexId) -> Option<usize> {
    geo.vertices().get_dense_index(vid.into())
}

fn pack_shelves(charts: &mut [PackChart], pad_world: f32) -> (f32, f32) {
    let pad = if pad_world.is_finite() {
        pad_world.max(0.0)
    } else {
        0.0
    };

    charts.sort_by(|a, b| b.size.y.total_cmp(&a.size.y));

    let mut total_area = 0.0f32;
    let mut max_w = 0.0f32;
    for c in charts.iter() {
        let w = (c.size.x + pad).max(0.0);
        let h = (c.size.y + pad).max(0.0);
        total_area += w * h;
        max_w = max_w.max(w);
    }

    let mut row_w = total_area.sqrt();
    if !row_w.is_finite() || row_w <= 0.0 {
        row_w = max_w.max(1.0);
    }
    row_w = row_w.max(max_w) * 1.15;

    let (mut x, mut y, mut row_h) = (0.0f32, 0.0f32, 0.0f32);
    let mut out_w = 0.0f32;
    for c in charts.iter_mut() {
        let w = c.size.x + pad;
        let h = c.size.y + pad;
        if x > 0.0 && x + w > row_w {
            y += row_h;
            x = 0.0;
            row_h = 0.0;
        }
        c.place = Vec2::new(x, y);
        c.rotated = false;
        x += w;
        row_h = row_h.max(h);
        out_w = out_w.max(x);
    }
    let out_h = y + row_h;
    (out_w.max(1e-6), out_h.max(1e-6))
}

#[derive(Clone, Copy, Debug)]
struct SkyNode {
    x: f32,
    y: f32,
    w: f32,
}

fn skyline_pack_with_width(charts: &mut [PackChart], pad_world: f32, width: f32) -> (f32, f32) {
    let pad = if pad_world.is_finite() {
        pad_world.max(0.0)
    } else {
        0.0
    };
    let width = width.max(1e-6);

    // Sort large-first to reduce fragmentation.
    charts.sort_by(|a, b| {
        let aa = a.size.x.max(a.size.y);
        let bb = b.size.x.max(b.size.y);
        bb.total_cmp(&aa)
    });

    for c in charts.iter_mut() {
        c.place = Vec2::ZERO;
        c.rotated = false;
    }

    let mut skyline: Vec<SkyNode> = vec![SkyNode {
        x: 0.0,
        y: 0.0,
        w: width,
    }];
    let mut out_h = 0.0f32;

    let mut find_pos = |rw: f32, rh: f32, skyline: &[SkyNode]| -> Option<(f32, f32, usize)> {
        let mut best: Option<(f32, f32, usize, f32)> = None; // (x,y,i,score)
        for (i, node) in skyline.iter().enumerate() {
            let x = node.x;
            if x + rw > width + 1e-6 {
                continue;
            }
            let mut y = node.y;
            let mut remain = rw;
            let mut j = i;
            while remain > 1e-6 {
                let n = skyline.get(j)?;
                y = y.max(n.y);
                let take = (n.w).min(remain);
                remain -= take;
                j += 1;
                if j > skyline.len() + 2 {
                    return None;
                }
            }
            let score = y + rh;
            match best {
                None => best = Some((x, y, i, score)),
                Some((bx, by, bi, bs)) => {
                    if score < bs - 1e-6
                        || (score - bs).abs() <= 1e-6
                            && (y < by - 1e-6 || (y - by).abs() <= 1e-6 && x < bx - 1e-6)
                    {
                        best = Some((x, y, i, score));
                    } else {
                        let _ = bi;
                    }
                }
            }
        }
        best.map(|(x, y, i, _)| (x, y, i))
    };

    for c in charts.iter_mut() {
        let (w0, h0) = (c.size.x + pad, c.size.y + pad);
        let (w1, h1) = (c.size.y + pad, c.size.x + pad);

        let p0 = find_pos(w0, h0, &skyline);
        let p1 = find_pos(w1, h1, &skyline);

        let (x, y, i, rw, rh, rot) = match (p0, p1) {
            (Some((x0, y0, i0)), Some((x1, y1, i1))) => {
                let s0 = y0 + h0;
                let s1 = y1 + h1;
                if s1 < s0 - 1e-6 {
                    (x1, y1, i1, w1, h1, true)
                } else if s0 < s1 - 1e-6 {
                    (x0, y0, i0, w0, h0, false)
                } else if y1 < y0 - 1e-6 {
                    (x1, y1, i1, w1, h1, true)
                } else {
                    (x0, y0, i0, w0, h0, false)
                }
            }
            (Some((x0, y0, i0)), None) => (x0, y0, i0, w0, h0, false),
            (None, Some((x1, y1, i1))) => (x1, y1, i1, w1, h1, true),
            (None, None) => {
                // If width is too small for this chart, push it to the end by expanding height.
                c.place = Vec2::new(0.0, out_h);
                c.rotated = false;
                out_h += h0.max(h1);
                continue;
            }
        };

        c.place = Vec2::new(x, y);
        c.rotated = rot;
        out_h = out_h.max(y + rh);

        // Update skyline: insert new node then remove overlaps.
        skyline.insert(
            i,
            SkyNode {
                x,
                y: y + rh,
                w: rw,
            },
        );

        let mut idx = i + 1;
        while idx < skyline.len() {
            let prev = skyline[idx - 1];
            let cur = skyline[idx];
            let overlap = (prev.x + prev.w) - cur.x;
            if overlap > 1e-6 {
                if overlap >= cur.w - 1e-6 {
                    skyline.remove(idx);
                    continue;
                } else {
                    skyline[idx].x += overlap;
                    skyline[idx].w -= overlap;
                }
            }
            if skyline[idx].w <= 1e-6 {
                skyline.remove(idx);
                continue;
            }
            idx += 1;
        }

        // Merge same-height neighbors.
        let mut m = 0usize;
        while m + 1 < skyline.len() {
            if (skyline[m].y - skyline[m + 1].y).abs() <= 1e-6 {
                skyline[m].w += skyline[m + 1].w;
                skyline.remove(m + 1);
            } else {
                m += 1;
            }
        }
    }

    (width, out_h.max(1e-6))
}

fn pack_skyline(charts: &mut [PackChart], pad_world: f32) -> (f32, f32) {
    let pad = if pad_world.is_finite() {
        pad_world.max(0.0)
    } else {
        0.0
    };

    let mut total_area = 0.0f32;
    let mut min_width_lb = 0.0f32;
    for c in charts.iter() {
        let w = (c.size.x + pad).max(1e-6);
        let h = (c.size.y + pad).max(1e-6);
        total_area += w * h;
        min_width_lb = min_width_lb.max(w.min(h));
    }
    if !total_area.is_finite() || total_area <= 0.0 {
        return skyline_pack_with_width(charts, pad_world, min_width_lb.max(1.0));
    }

    let base = total_area.sqrt().max(1e-6);
    let cands = [
        min_width_lb.max(base * 1.00),
        min_width_lb.max(base * 1.15),
        min_width_lb.max(base * 1.35),
    ];

    let mut best_dim = f32::INFINITY;
    let mut best_w = cands[0];
    let mut best_h = f32::INFINITY;
    let mut best_layout: Option<Vec<(Vec2, bool)>> = None;

    for &w in cands.iter() {
        let mut tmp = charts.to_vec();
        let (ow, oh) = skyline_pack_with_width(&mut tmp, pad_world, w);
        let dim = ow.max(oh);
        if dim < best_dim - 1e-6 || ((dim - best_dim).abs() <= 1e-6 && oh < best_h - 1e-6) {
            best_dim = dim;
            best_w = ow;
            best_h = oh;
            let mut layout = vec![(Vec2::ZERO, false); tmp.len()];
            for c in tmp.iter() {
                layout[c.id] = (c.place, c.rotated);
            }
            best_layout = Some(layout);
        }
    }

    // Re-pack once more using best_w to write placements into `charts`.
    let (_ow, _oh) = skyline_pack_with_width(charts, pad_world, best_w);
    if let Some(layout) = best_layout {
        for c in charts.iter_mut() {
            let (p, r) = layout[c.id];
            c.place = p;
            c.rotated = r;
        }
        (best_w.max(1e-6), best_h.max(1e-6))
    } else {
        (best_w.max(1e-6), best_h.max(1e-6))
    }
}

#[derive(Clone, Copy)]
struct PackChart {
    id: usize,
    min: Vec2,
    size: Vec2,
    place: Vec2,
    rotated: bool,
}

/// Pure-Rust "Smart UV Project" style unwrap:
/// - triangulate polygons
/// - chart by normal angle threshold
/// - per-chart planar projection (best-fit normal)
/// - shelf-pack charts into 0..1 atlas
pub fn auto_uv_smart_project(
    geo: &Geometry,
    opts: AutoUvSmartProjectOptions,
) -> Result<AutoUvSmartProjectResult, AutoUvError> {
    let positions = geo
        .get_point_position_attribute()
        .ok_or(AutoUvError::MissingPositions)?;
    if positions.iter().any(|p| !p.is_finite()) {
        return Err(AutoUvError::NonFinitePositions);
    }

    let mut tris: Vec<Tri> = Vec::new();

    for (prim_dense, prim) in geo.primitives().values().iter().enumerate() {
        if !matches!(prim, GeoPrimitive::Polygon(_)) {
            continue;
        }
        let verts = prim.vertices();
        if verts.len() < 3 {
            continue;
        }

        let v0 = verts[0];
        for i in 1..verts.len() - 1 {
            let v1 = verts[i];
            let v2 = verts[i + 1];

            let (Some(p0), Some(p1), Some(p2)) = (
                vid_to_point_dense(geo, v0),
                vid_to_point_dense(geo, v1),
                vid_to_point_dense(geo, v2),
            ) else {
                continue;
            };
            if p0 == p1 || p1 == p2 || p2 == p0 {
                continue;
            }

            let (Some(vd0), Some(vd1), Some(vd2)) = (
                vid_to_vertex_dense(geo, v0),
                vid_to_vertex_dense(geo, v1),
                vid_to_vertex_dense(geo, v2),
            ) else {
                continue;
            };

            let (Some(a), Some(b), Some(c)) =
                (positions.get(p0), positions.get(p1), positions.get(p2))
            else {
                continue;
            };
            let n = (*b - *a).cross(*c - *a);
            let area2 = n.length();
            if !area2.is_finite() || area2 <= 1e-12 {
                continue;
            }
            let normal = n / area2;
            let area = 0.5 * area2;
            let centroid = (*a + *b + *c) * (1.0 / 3.0);
            tris.push(Tri {
                pts: [p0, p1, p2],
                verts: [vd0, vd1, vd2],
                normal,
                area,
                centroid,
                prim_dense,
            });
        }
    }

    if tris.is_empty() {
        return Err(AutoUvError::NoValidTriangles);
    }

    let cos_limit = opts.max_angle_deg.clamp(0.0, 180.0).to_radians().cos();

    let mut edge_map: FxHashMap<EdgeKey, Vec<usize>> = FxHashMap::default();
    for (ti, t) in tris.iter().enumerate() {
        let edges = [
            EdgeKey::new(t.pts[0], t.pts[1]),
            EdgeKey::new(t.pts[1], t.pts[2]),
            EdgeKey::new(t.pts[2], t.pts[0]),
        ];
        for e in edges {
            edge_map.entry(e).or_default().push(ti);
        }
    }

    let mut dsu = Dsu::new(tris.len());
    for tris_on_edge in edge_map.values() {
        if tris_on_edge.len() < 2 {
            continue;
        }
        for i in 0..tris_on_edge.len() {
            for j in (i + 1)..tris_on_edge.len() {
                let a = tris_on_edge[i];
                let b = tris_on_edge[j];
                if tris[a].normal.dot(tris[b].normal) >= cos_limit {
                    dsu.union(a, b);
                }
            }
        }
    }

    // Never split inside a single primitive: triangles generated from the same polygon share
    // vertex IDs, so splitting them into different charts would require duplicating vertices.
    let mut first_by_prim: FxHashMap<usize, usize> = FxHashMap::default();
    for (ti, t) in tris.iter().enumerate() {
        if let Some(&first) = first_by_prim.get(&t.prim_dense) {
            dsu.union(first, ti);
        } else {
            first_by_prim.insert(t.prim_dense, ti);
        }
    }

    // Never split inside a single primitive: triangles generated from the same polygon share
    // vertex IDs, so splitting them into different charts would require duplicating vertices.
    let mut first_by_prim: FxHashMap<usize, usize> = FxHashMap::default();
    for (ti, t) in tris.iter().enumerate() {
        if let Some(&first) = first_by_prim.get(&t.prim_dense) {
            dsu.union(first, ti);
        } else {
            first_by_prim.insert(t.prim_dense, ti);
        }
    }

    let mut root_to_chart: FxHashMap<usize, usize> = FxHashMap::default();
    let mut tri_chart = vec![0usize; tris.len()];
    for ti in 0..tris.len() {
        let root = dsu.find(ti);
        let idx = match root_to_chart.get(&root).copied() {
            Some(idx) => idx,
            None => {
                let idx = root_to_chart.len();
                root_to_chart.insert(root, idx);
                idx
            }
        };
        tri_chart[ti] = idx;
    }
    let chart_count = root_to_chart.len().max(1);
    let mut chart_tris: Vec<Vec<usize>> = vec![Vec::new(); chart_count];
    let mut chart_verts: Vec<Vec<usize>> = vec![Vec::new(); chart_count];
    for (ti, t) in tris.iter().enumerate() {
        let ci = tri_chart[ti];
        chart_tris[ci].push(ti);
        chart_verts[ci].extend_from_slice(&t.verts);
    }
    for vs in chart_verts.iter_mut() {
        vs.sort_unstable();
        vs.dedup();
    }

    let mut chart_depth = vec![0u32; chart_tris.len()];
    let mut uvs = vec![Vec2::ZERO; geo.vertices().len()];
    let mut uv_set = vec![false; geo.vertices().len()];
    let mut chart_bounds: Vec<(Vec2, Vec2)> = Vec::with_capacity(chart_count);

    for ci in 0..chart_count {
        let mut n_sum = Vec3::ZERO;
        let mut c_sum = Vec3::ZERO;
        let mut area_sum = 0.0f32;
        for &ti in &chart_tris[ci] {
            let t = tris[ti];
            n_sum += t.normal * t.area;
            let (a, b, c) = (
                positions[t.pts[0]],
                positions[t.pts[1]],
                positions[t.pts[2]],
            );
            c_sum += (a + b + c) * (t.area / 3.0);
            area_sum += t.area;
        }

        let normal = if n_sum.length_squared() > 1e-20 && n_sum.is_finite() {
            n_sum.normalize()
        } else {
            Vec3::Z
        };
        let origin = if area_sum.is_finite() && area_sum > 0.0 {
            c_sum / area_sum
        } else {
            Vec3::ZERO
        };

        let up = if normal.z.abs() < 0.999 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let mut tangent = normal.cross(up);
        if tangent.length_squared() <= 1e-20 {
            tangent = normal.cross(Vec3::X);
        }
        let tangent = tangent.normalize();
        let bitangent = normal.cross(tangent).normalize();

        let mut min = Vec2::new(f32::INFINITY, f32::INFINITY);
        let mut max = Vec2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);

        for &vd in &chart_verts[ci] {
            let Some(v) = geo.vertices().values().get(vd) else {
                continue;
            };
            let Some(pd) = geo.points().get_dense_index(v.point_id.into()) else {
                continue;
            };
            let Some(p) = positions.get(pd) else {
                continue;
            };
            let d = *p - origin;
            let uv = Vec2::new(d.dot(tangent), d.dot(bitangent));
            if uv.is_finite() {
                uvs[vd] = uv;
                uv_set[vd] = true;
                min.x = min.x.min(uv.x);
                min.y = min.y.min(uv.y);
                max.x = max.x.max(uv.x);
                max.y = max.y.max(uv.y);
            }
        }

        if !min.is_finite() || !max.is_finite() {
            min = Vec2::ZERO;
            max = Vec2::ZERO;
        }
        chart_bounds.push((min, max));
    }

    let charts = chart_bounds.len();
    let mut pack: Vec<PackChart> = chart_bounds
        .iter()
        .enumerate()
        .map(|(id, (min, max))| {
            let mut size = *max - *min;
            if !size.is_finite() {
                size = Vec2::splat(1e-3);
            }
            size.x = size.x.abs().max(1e-3);
            size.y = size.y.abs().max(1e-3);
            PackChart {
                id,
                min: *min,
                size,
                place: Vec2::ZERO,
                rotated: false,
            }
        })
        .collect();

    let (w0, h0) = pack_skyline(&mut pack, 0.0);
    let dim0 = w0.max(h0);
    let pad_world = opts.padding.clamp(0.0, 0.5) * dim0;
    let (w1, h1) = pack_skyline(&mut pack, pad_world);
    let dim1 = w1.max(h1).max(1e-6);
    let scale = (1.0 / dim1).max(0.0);

    let mut placements = vec![Vec2::ZERO; charts];
    let mut rotations = vec![false; charts];
    let mut sizes = vec![Vec2::ZERO; charts];
    for c in pack.iter() {
        placements[c.id] = c.place;
        rotations[c.id] = c.rotated;
        sizes[c.id] = c.size;
    }

    for ci in 0..charts {
        let content_origin = placements[ci] + Vec2::splat(pad_world * 0.5);
        let (min, _) = chart_bounds[ci];
        let rot = rotations[ci];
        let size = sizes[ci];
        for &vd in &chart_verts[ci] {
            if !uv_set.get(vd).copied().unwrap_or(false) {
                continue;
            }
            let mut rel = uvs[vd] - min;
            if rot {
                rel = Vec2::new(rel.y, size.x - rel.x);
            }
            uvs[vd] = (rel + content_origin) * scale;
        }
    }

    Ok(AutoUvSmartProjectResult {
        uvs,
        points: positions.len(),
        vertices: geo.vertices().len(),
        triangles: tris.len(),
        charts,
    })
}

#[inline]
fn cg_dot(a: &[f32], b: &[f32]) -> f32 {
    let mut s = 0.0f32;
    for i in 0..a.len() {
        s += a[i] * b[i];
    }
    s
}

#[inline]
fn cg_mat_mul(diag: &[f32], neighbors: &[Vec<(usize, f32)>], x: &[f32], out: &mut [f32]) {
    for i in 0..diag.len() {
        let mut v = diag[i] * x[i];
        for &(j, w) in &neighbors[i] {
            v -= w * x[j];
        }
        out[i] = v;
    }
}

fn cg_solve(
    diag: &[f32],
    neighbors: &[Vec<(usize, f32)>],
    b: &[f32],
    tol: f32,
    max_iter: u32,
) -> Vec<f32> {
    let n = b.len();
    let mut x = vec![0.0f32; n];
    if n == 0 {
        return x;
    }

    let b_norm2 = cg_dot(b, b);
    if !b_norm2.is_finite() || b_norm2 <= 0.0 {
        return x;
    }

    let mut r = b.to_vec();
    let mut p = r.clone();
    let mut ap = vec![0.0f32; n];

    let mut rs_old = cg_dot(&r, &r);
    let tol2 = (tol.max(0.0) * tol.max(0.0)) * b_norm2.max(1.0);
    if !rs_old.is_finite() {
        return x;
    }
    if rs_old <= tol2 {
        return x;
    }

    let iters = max_iter.max(1).min(100_000);
    for _ in 0..iters {
        cg_mat_mul(diag, neighbors, &p, &mut ap);
        let denom = cg_dot(&p, &ap);
        if !denom.is_finite() || denom.abs() <= 1e-30 {
            break;
        }
        let alpha = rs_old / denom;
        if !alpha.is_finite() {
            break;
        }
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        let rs_new = cg_dot(&r, &r);
        if !rs_new.is_finite() {
            break;
        }
        if rs_new <= tol2 {
            break;
        }
        let beta = rs_new / rs_old.max(1e-30);
        if !beta.is_finite() {
            break;
        }
        for i in 0..n {
            p[i] = r[i] + beta * p[i];
        }
        rs_old = rs_new;
    }

    x
}

fn pcg_solve(
    diag: &[f32],
    neighbors: &[Vec<(usize, f32)>],
    b: &[f32],
    x0: Option<&[f32]>,
    tol: f32,
    max_iter: u32,
) -> Vec<f32> {
    let n = b.len();
    let mut x = if let Some(x0) = x0 {
        if x0.len() == n {
            x0.to_vec()
        } else {
            vec![0.0f32; n]
        }
    } else {
        vec![0.0f32; n]
    };

    if n == 0 {
        return x;
    }

    let b_norm2 = cg_dot(b, b);
    if !b_norm2.is_finite() || b_norm2 <= 0.0 {
        return x;
    }

    // r = b - A x
    let mut ax = vec![0.0f32; n];
    cg_mat_mul(diag, neighbors, &x, &mut ax);
    let mut r = vec![0.0f32; n];
    for i in 0..n {
        r[i] = b[i] - ax[i];
    }

    let mut z = vec![0.0f32; n];
    for i in 0..n {
        let d = diag[i];
        z[i] = if d.is_finite() && d.abs() > 1e-20 {
            r[i] / d
        } else {
            r[i]
        };
    }
    let mut p = z.clone();

    let mut rz_old = cg_dot(&r, &z);
    let tol2 = (tol.max(0.0) * tol.max(0.0)) * b_norm2.max(1.0);
    if !rz_old.is_finite() {
        return x;
    }
    if cg_dot(&r, &r) <= tol2 {
        return x;
    }

    let mut ap = vec![0.0f32; n];
    let iters = max_iter.max(1).min(200_000);
    for _ in 0..iters {
        cg_mat_mul(diag, neighbors, &p, &mut ap);
        let denom = cg_dot(&p, &ap);
        if !denom.is_finite() || denom.abs() <= 1e-30 {
            break;
        }
        let alpha = rz_old / denom;
        if !alpha.is_finite() {
            break;
        }
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }

        let rr = cg_dot(&r, &r);
        if !rr.is_finite() {
            break;
        }
        if rr <= tol2 {
            break;
        }

        for i in 0..n {
            let d = diag[i];
            z[i] = if d.is_finite() && d.abs() > 1e-20 {
                r[i] / d
            } else {
                r[i]
            };
        }
        let rz_new = cg_dot(&r, &z);
        if !rz_new.is_finite() {
            break;
        }
        let beta = rz_new / rz_old.max(1e-30);
        if !beta.is_finite() {
            break;
        }
        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
        rz_old = rz_new;
    }

    x
}

#[inline]
fn spmv_sym(diag: &[f32], neighbors: &[Vec<(usize, f32)>], x: &[f32], out: &mut [f32]) {
    for i in 0..diag.len() {
        let mut v = diag[i] * x[i];
        for &(j, aij) in &neighbors[i] {
            v += aij * x[j];
        }
        out[i] = v;
    }
}

fn pcg_solve_sym(
    diag: &[f32],
    neighbors: &[Vec<(usize, f32)>],
    b: &[f32],
    x0: Option<&[f32]>,
    tol: f32,
    max_iter: u32,
) -> Vec<f32> {
    let n = b.len();
    let mut x = if let Some(x0) = x0 {
        if x0.len() == n {
            x0.to_vec()
        } else {
            vec![0.0f32; n]
        }
    } else {
        vec![0.0f32; n]
    };

    if n == 0 {
        return x;
    }

    let b_norm2 = cg_dot(b, b);
    if !b_norm2.is_finite() || b_norm2 <= 0.0 {
        return x;
    }

    // r = b - A x
    let mut ax = vec![0.0f32; n];
    spmv_sym(diag, neighbors, &x, &mut ax);
    let mut r = vec![0.0f32; n];
    for i in 0..n {
        r[i] = b[i] - ax[i];
    }

    let mut z = vec![0.0f32; n];
    for i in 0..n {
        let d = diag[i];
        z[i] = if d.is_finite() && d.abs() > 1e-20 {
            r[i] / d
        } else {
            r[i]
        };
    }
    let mut p = z.clone();

    let tol2 = (tol.max(0.0) * tol.max(0.0)) * b_norm2.max(1.0);
    let mut rz_old = cg_dot(&r, &z);
    if !rz_old.is_finite() {
        return x;
    }
    if cg_dot(&r, &r) <= tol2 {
        return x;
    }

    let mut ap = vec![0.0f32; n];
    let iters = max_iter.max(1).min(200_000);
    for _ in 0..iters {
        spmv_sym(diag, neighbors, &p, &mut ap);
        let denom = cg_dot(&p, &ap);
        if !denom.is_finite() || denom.abs() <= 1e-30 {
            break;
        }
        let alpha = rz_old / denom;
        if !alpha.is_finite() {
            break;
        }
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }

        let rr = cg_dot(&r, &r);
        if !rr.is_finite() {
            break;
        }
        if rr <= tol2 {
            break;
        }

        for i in 0..n {
            let d = diag[i];
            z[i] = if d.is_finite() && d.abs() > 1e-20 {
                r[i] / d
            } else {
                r[i]
            };
        }
        let rz_new = cg_dot(&r, &z);
        if !rz_new.is_finite() {
            break;
        }
        let beta = rz_new / rz_old.max(1e-30);
        if !beta.is_finite() {
            break;
        }
        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
        rz_old = rz_new;
    }

    x
}

fn canonicalize_loop(loop_vertices: &mut Vec<usize>, points_global: &[usize]) {
    if loop_vertices.len() < 2 {
        return;
    }

    let mut best_i = 0usize;
    let mut best = points_global[loop_vertices[0]];
    for (i, &v) in loop_vertices.iter().enumerate().skip(1) {
        let key = points_global[v];
        if key < best {
            best = key;
            best_i = i;
        }
    }
    loop_vertices.rotate_left(best_i);

    if loop_vertices.len() >= 3 {
        let a = points_global[loop_vertices[1]];
        let b = points_global[*loop_vertices.last().unwrap()];
        if a > b {
            let first = loop_vertices[0];
            let mut rest = loop_vertices[1..].to_vec();
            rest.reverse();
            loop_vertices.clear();
            loop_vertices.push(first);
            loop_vertices.extend(rest);
        }
    }
}

fn loop_perimeter(loop_vertices: &[usize], points_global: &[usize], positions: &[Vec3]) -> f32 {
    if loop_vertices.len() < 2 {
        return 0.0;
    }
    let mut len = 0.0f32;
    for i in 0..loop_vertices.len() {
        let a = positions[points_global[loop_vertices[i]]];
        let b = positions[points_global[loop_vertices[(i + 1) % loop_vertices.len()]]];
        len += (b - a).length();
    }
    len
}

fn loop_params(loop_vertices: &[usize], points_global: &[usize], positions: &[Vec3]) -> Vec<f32> {
    let n = loop_vertices.len();
    if n == 0 {
        return Vec::new();
    }

    let mut cum = Vec::with_capacity(n);
    let mut total = 0.0f32;
    for i in 0..n {
        cum.push(total);
        let a = positions[points_global[loop_vertices[i]]];
        let b = positions[points_global[loop_vertices[(i + 1) % n]]];
        total += (b - a).length();
    }

    if !total.is_finite() || total <= 1e-20 {
        return vec![0.0; n];
    }

    let inv = 1.0 / total;
    for v in cum.iter_mut() {
        *v *= inv;
    }
    cum
}

fn extract_boundary_loops(tris_local: &[[usize; 3]], point_count: usize) -> Vec<Vec<usize>> {
    let mut edge_counts: FxHashMap<EdgeKey, u8> = FxHashMap::default();
    for t in tris_local {
        let edges = [
            EdgeKey::new(t[0], t[1]),
            EdgeKey::new(t[1], t[2]),
            EdgeKey::new(t[2], t[0]),
        ];
        for e in edges {
            let c = edge_counts.entry(e).or_insert(0);
            *c = c.saturating_add(1);
        }
    }

    let mut b_adj: Vec<Vec<usize>> = vec![Vec::new(); point_count];
    let mut boundary_edges = 0usize;
    for (e, c) in edge_counts.into_iter() {
        if c == 1 {
            let a = e.0 as usize;
            let b = e.1 as usize;
            if a < point_count && b < point_count {
                b_adj[a].push(b);
                b_adj[b].push(a);
                boundary_edges += 1;
            }
        }
    }

    if boundary_edges == 0 {
        return Vec::new();
    }

    let mut visited: FxHashSet<EdgeKey> = FxHashSet::default();
    let mut loops: Vec<Vec<usize>> = Vec::new();

    for start in 0..point_count {
        if b_adj[start].is_empty() {
            continue;
        }
        let has_unvisited = b_adj[start]
            .iter()
            .any(|&nb| !visited.contains(&EdgeKey::new(start, nb)));
        if !has_unvisited {
            continue;
        }

        let mut loop_vertices: Vec<usize> = Vec::new();
        let mut prev = usize::MAX;
        let mut cur = start;
        let mut closed = false;
        let mut safety_budget = boundary_edges.saturating_mul(4).max(64);

        loop_vertices.push(cur);

        loop {
            if safety_budget == 0 {
                break;
            }
            safety_budget -= 1;

            let mut next: Option<usize> = None;
            for &nb in &b_adj[cur] {
                if nb == prev {
                    continue;
                }
                let e = EdgeKey::new(cur, nb);
                if visited.contains(&e) {
                    continue;
                }
                next = Some(nb);
                break;
            }
            if next.is_none() {
                for &nb in &b_adj[cur] {
                    let e = EdgeKey::new(cur, nb);
                    if visited.contains(&e) {
                        continue;
                    }
                    next = Some(nb);
                    break;
                }
            }
            let Some(next) = next else {
                break;
            };

            visited.insert(EdgeKey::new(cur, next));
            prev = cur;
            cur = next;

            if cur == start {
                closed = true;
                break;
            }
            loop_vertices.push(cur);
        }

        if closed && loop_vertices.len() >= 3 {
            loops.push(loop_vertices);
        }
    }

    loops
}

fn build_laplacian_weights(
    points_global: &[usize],
    tris_local: &[[usize; 3]],
    positions: &[Vec3],
    weighting: AutoUvWeighting,
) -> (Vec<f32>, Vec<Vec<(usize, f32)>>) {
    let n = points_global.len();
    let mut wmap: FxHashMap<EdgeKey, f32> = FxHashMap::default();

    match weighting {
        AutoUvWeighting::Uniform => {
            for t in tris_local {
                let edges = [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])];
                for (a, b) in edges {
                    let e = EdgeKey::new(a, b);
                    *wmap.entry(e).or_insert(0.0) += 1.0;
                }
            }
        }
        AutoUvWeighting::Cotangent => {
            for t in tris_local {
                let (i, j, k) = (t[0], t[1], t[2]);
                let (pi, pj, pk) = (
                    positions[points_global[i]],
                    positions[points_global[j]],
                    positions[points_global[k]],
                );

                let cot_i = {
                    let a = pj - pi;
                    let b = pk - pi;
                    let denom = a.cross(b).length();
                    if denom.is_finite() && denom > 1e-20 {
                        (a.dot(b) / denom).max(0.0)
                    } else {
                        0.0
                    }
                };
                let cot_j = {
                    let a = pk - pj;
                    let b = pi - pj;
                    let denom = a.cross(b).length();
                    if denom.is_finite() && denom > 1e-20 {
                        (a.dot(b) / denom).max(0.0)
                    } else {
                        0.0
                    }
                };
                let cot_k = {
                    let a = pi - pk;
                    let b = pj - pk;
                    let denom = a.cross(b).length();
                    if denom.is_finite() && denom > 1e-20 {
                        (a.dot(b) / denom).max(0.0)
                    } else {
                        0.0
                    }
                };

                let add = |wmap: &mut FxHashMap<EdgeKey, f32>, a: usize, b: usize, w: f32| {
                    if w.is_finite() && w > 0.0 {
                        *wmap.entry(EdgeKey::new(a, b)).or_insert(0.0) += 0.5 * w;
                    }
                };

                add(&mut wmap, j, k, cot_i);
                add(&mut wmap, k, i, cot_j);
                add(&mut wmap, i, j, cot_k);
            }
        }
    }

    let mut diag = vec![0.0f32; n];
    let mut neighbors: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n];

    for (e, w) in wmap.into_iter() {
        if !w.is_finite() || w <= 0.0 {
            continue;
        }
        let a = e.0 as usize;
        let b = e.1 as usize;
        if a >= n || b >= n {
            continue;
        }
        neighbors[a].push((b, w));
        neighbors[b].push((a, w));
        diag[a] += w;
        diag[b] += w;
    }

    (diag, neighbors)
}

#[inline]
fn project_triangle(p0: Vec3, p1: Vec3, p2: Vec3) -> Option<(Vec2, Vec2, Vec2)> {
    let x = (p1 - p0).normalize_or_zero();
    if x.length_squared() <= 1e-20 {
        return None;
    }
    let z = x.cross(p2 - p0).normalize_or_zero();
    if z.length_squared() <= 1e-20 {
        return None;
    }
    let y = z.cross(x);
    let o = p0;
    let z0 = Vec2::ZERO;
    let z1 = Vec2::new((p1 - o).length(), 0.0);
    let d2 = p2 - o;
    let z2 = Vec2::new(d2.dot(x), d2.dot(y));
    if !z1.is_finite() || !z2.is_finite() {
        return None;
    }
    Some((z0, z1, z2))
}

fn approx_diameter_vertices(points_global: &[usize], positions: &[Vec3]) -> Option<(usize, usize)> {
    let n = points_global.len();
    if n < 2 {
        return None;
    }

    let mut min_v = [0usize; 3];
    let mut max_v = [0usize; 3];
    let mut min_p = Vec3::splat(f32::INFINITY);
    let mut max_p = Vec3::splat(f32::NEG_INFINITY);

    for (li, &pg) in points_global.iter().enumerate() {
        let p = positions.get(pg).copied().unwrap_or(Vec3::ZERO);
        if p.x < min_p.x {
            min_p.x = p.x;
            min_v[0] = li;
        }
        if p.x > max_p.x {
            max_p.x = p.x;
            max_v[0] = li;
        }
        if p.y < min_p.y {
            min_p.y = p.y;
            min_v[1] = li;
        }
        if p.y > max_p.y {
            max_p.y = p.y;
            max_v[1] = li;
        }
        if p.z < min_p.z {
            min_p.z = p.z;
            min_v[2] = li;
        }
        if p.z > max_p.z {
            max_p.z = p.z;
            max_v[2] = li;
        }
    }

    let mut best = (min_v[0], max_v[0]);
    let mut best_len = 0.0f32;
    for axis in 0..3 {
        let a = min_v[axis];
        let b = max_v[axis];
        let pa = positions[points_global[a]];
        let pb = positions[points_global[b]];
        let l = (pb - pa).length();
        if l > best_len {
            best_len = l;
            best = (a, b);
        }
    }

    if best.0 == best.1 || !best_len.is_finite() || best_len <= 1e-12 {
        None
    } else {
        Some(best)
    }
}

fn lscm_flatten(
    points_global: &[usize],
    tris_local: &[[usize; 3]],
    positions: &[Vec3],
    uv_init: Option<&[Vec2]>,
    opts: AutoUvSmartFlattenOptions,
) -> Option<Vec<Vec2>> {
    let n = points_global.len();
    if n < 3 || tris_local.is_empty() {
        return None;
    }

    // Needs at least one boundary edge to be well-posed in practice.
    let loops = extract_boundary_loops(tris_local, n);
    if loops.is_empty() {
        return None;
    }

    let (p0, p1) = approx_diameter_vertices(points_global, positions)?;

    let mut pin0 = Vec2::ZERO;
    let mut pin1 = Vec2::new(1.0, 0.0);
    if let Some(init) = uv_init {
        if init.len() == n {
            pin0 = init[p0];
            pin1 = init[p1];
            if (pin1 - pin0).length_squared() <= 1e-12 {
                pin1 = pin0 + Vec2::X;
            }
        }
    }

    let var_count = 2 * n;
    let mut pinned = vec![false; var_count];
    let mut pinned_value = vec![0.0f32; var_count];
    let u0 = 2 * p0;
    let v0 = 2 * p0 + 1;
    let u1 = 2 * p1;
    let v1 = 2 * p1 + 1;
    pinned[u0] = true;
    pinned[v0] = true;
    pinned[u1] = true;
    pinned[v1] = true;
    pinned_value[u0] = pin0.x;
    pinned_value[v0] = pin0.y;
    pinned_value[u1] = pin1.x;
    pinned_value[v1] = pin1.y;

    let mut free_of_var = vec![usize::MAX; var_count];
    let mut free_count = 0usize;
    for vi in 0..var_count {
        if pinned[vi] {
            continue;
        }
        free_of_var[vi] = free_count;
        free_count += 1;
    }
    if free_count == 0 {
        return None;
    }

    let mut diag = vec![0.0f32; free_count];
    let mut rhs = vec![0.0f32; free_count];
    let mut off: FxHashMap<u64, f32> = FxHashMap::default();

    #[inline]
    fn key2(i: usize, j: usize) -> u64 {
        let (a, b) = if i <= j { (i, j) } else { (j, i) };
        ((a as u64) << 32) | (b as u64)
    }

    let mut add_q = |a_var: usize, b_var: usize, val: f32| {
        if !val.is_finite() || val == 0.0 {
            return;
        }
        let af = free_of_var[a_var];
        let bf = free_of_var[b_var];
        let a_free = af != usize::MAX;
        let b_free = bf != usize::MAX;

        match (a_free, b_free) {
            (true, true) => {
                let (i, j) = if af <= bf { (af, bf) } else { (bf, af) };
                if i == j {
                    diag[i] += val;
                } else {
                    *off.entry(key2(i, j)).or_insert(0.0) += val;
                }
            }
            (true, false) => {
                rhs[af] -= val * pinned_value[b_var];
            }
            (false, true) => {
                rhs[bf] -= val * pinned_value[a_var];
            }
            (false, false) => {}
        }
    };

    for t in tris_local.iter().copied() {
        let (i, j, k) = (t[0], t[1], t[2]);
        let (p0, p1, p2) = (
            positions[points_global[i]],
            positions[points_global[j]],
            positions[points_global[k]],
        );
        let Some((z0, z1, z2)) = project_triangle(p0, p1, p2) else {
            continue;
        };
        let a = z1.x - z0.x;
        let b = z1.y - z0.y;
        let c = z2.x - z0.x;
        let d = z2.y - z0.y;
        if !a.is_finite() || !b.is_finite() || !c.is_finite() || !d.is_finite() {
            continue;
        }

        let u_i = 2 * i;
        let v_i = 2 * i + 1;
        let u_j = 2 * j;
        let v_j = 2 * j + 1;
        let u_k = 2 * k;
        let v_k = 2 * k + 1;

        // From OpenNL LSCM example (xatlas fallback).
        let row1: [(usize, f32); 5] = [(u_i, -a + c), (v_i, b - d), (u_j, -c), (v_j, d), (u_k, a)];
        let row2: [(usize, f32); 5] =
            [(u_i, -b + d), (v_i, -a + c), (u_j, -d), (v_j, -c), (v_k, a)];

        for row in [row1, row2] {
            for p in 0..row.len() {
                let (vp, cp) = row[p];
                if !cp.is_finite() || cp == 0.0 {
                    continue;
                }
                for q in p..row.len() {
                    let (vq, cq) = row[q];
                    if !cq.is_finite() || cq == 0.0 {
                        continue;
                    }
                    add_q(vp, vq, cp * cq);
                }
            }
        }
    }

    for d in diag.iter_mut() {
        if !d.is_finite() || d.abs() <= 1e-20 {
            *d = 1.0;
        } else if *d < 1e-8 {
            *d += 1e-8;
        }
    }

    let mut neighbors: Vec<Vec<(usize, f32)>> = vec![Vec::new(); free_count];
    for (k, v) in off.into_iter() {
        if !v.is_finite() || v == 0.0 {
            continue;
        }
        let i = (k >> 32) as usize;
        let j = (k & 0xFFFF_FFFF) as usize;
        if i >= free_count || j >= free_count || i == j {
            continue;
        }
        neighbors[i].push((j, v));
        neighbors[j].push((i, v));
    }

    let x0 = uv_init.and_then(|init| {
        if init.len() != n {
            return None;
        }
        let mut guess = vec![0.0f32; free_count];
        for vi in 0..var_count {
            let fi = free_of_var[vi];
            if fi == usize::MAX {
                continue;
            }
            let vtx = vi / 2;
            let is_u = (vi & 1) == 0;
            let uv = init[vtx];
            guess[fi] = if is_u { uv.x } else { uv.y };
        }
        Some(guess)
    });

    let sol = pcg_solve_sym(
        &diag,
        &neighbors,
        &rhs,
        x0.as_deref(),
        opts.solver_tol,
        opts.max_solver_iters,
    );
    if sol.len() != free_count {
        return None;
    }

    let mut uv = vec![Vec2::ZERO; n];
    uv[p0] = pin0;
    uv[p1] = pin1;
    for vi in 0..var_count {
        let fi = free_of_var[vi];
        let vtx = vi / 2;
        let is_u = (vi & 1) == 0;
        let val = if fi == usize::MAX {
            pinned_value[vi]
        } else {
            sol[fi]
        };
        if is_u {
            uv[vtx].x = val;
        } else {
            uv[vtx].y = val;
        }
    }

    if uv.iter().any(|u| !u.is_finite()) {
        return None;
    }

    Some(uv)
}

fn harmonic_flatten(
    points_global: &[usize],
    tris_local: &[[usize; 3]],
    positions: &[Vec3],
    opts: AutoUvSmartFlattenOptions,
) -> Option<Vec<Vec2>> {
    let n = points_global.len();
    if n == 0 || tris_local.is_empty() {
        return None;
    }

    let mut loops = extract_boundary_loops(tris_local, n);
    if loops.is_empty() {
        return None;
    }

    for l in loops.iter_mut() {
        canonicalize_loop(l, points_global);
    }

    let mut uv = vec![Vec2::ZERO; n];
    let mut is_boundary = vec![false; n];

    let perims: Vec<f32> = loops
        .iter()
        .map(|l| loop_perimeter(l, points_global, positions))
        .collect();

    if loops.len() == 1 {
        let l = &loops[0];
        let t = loop_params(l, points_global, positions);
        for (&v, &u) in l.iter().zip(t.iter()) {
            let theta = std::f32::consts::TAU * u;
            uv[v] = Vec2::new(theta.cos(), theta.sin());
            is_boundary[v] = true;
        }
    } else if loops.len() == 2 {
        for (li, l) in loops.iter().enumerate() {
            let t = loop_params(l, points_global, positions);
            let v_val = if li == 0 { 0.0 } else { 1.0 };
            for (&v, &u) in l.iter().zip(t.iter()) {
                uv[v] = Vec2::new(u, v_val);
                is_boundary[v] = true;
            }
        }
    } else {
        let (outer_i, outer_perim) = perims
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, &p)| (i, p.max(1e-6)))
            .unwrap_or((0, 1.0));

        // Outer boundary on unit circle at origin.
        {
            let l = &loops[outer_i];
            let t = loop_params(l, points_global, positions);
            for (&v, &u) in l.iter().zip(t.iter()) {
                let theta = std::f32::consts::TAU * u;
                uv[v] = Vec2::new(theta.cos(), theta.sin());
                is_boundary[v] = true;
            }
        }

        // Inner boundaries as disjoint-ish circles inside the unit disk.
        let mut hole_idx = 0usize;
        let hole_count = loops.len() - 1;
        for (li, l) in loops.iter().enumerate() {
            if li == outer_i {
                continue;
            }

            let per = perims.get(li).copied().unwrap_or(0.0).max(1e-6);
            let mut r = (per / outer_perim) * 0.25;
            r = r.clamp(0.05, 0.35);

            let ang = std::f32::consts::TAU * (hole_idx as f32 / hole_count.max(1) as f32);
            let mut d = 0.55;
            if d + r > 0.9 {
                d = (0.9 - r).max(0.0);
            }
            let center = Vec2::new(ang.cos(), ang.sin()) * d;

            let t = loop_params(l, points_global, positions);
            for (&v, &u) in l.iter().zip(t.iter()) {
                let theta = std::f32::consts::TAU * u;
                uv[v] = center + Vec2::new(theta.cos(), theta.sin()) * r;
                is_boundary[v] = true;
            }

            hole_idx += 1;
        }
    }

    let (diag_all, neighbors_all) =
        build_laplacian_weights(points_global, tris_local, positions, opts.weighting);

    let mut interior_index = vec![usize::MAX; n];
    let mut interior_vertices: Vec<usize> = Vec::new();
    for i in 0..n {
        if is_boundary[i] {
            continue;
        }
        interior_index[i] = interior_vertices.len();
        interior_vertices.push(i);
    }

    let m = interior_vertices.len();
    if m == 0 {
        return Some(uv);
    }

    let mut diag = vec![0.0f32; m];
    let mut neighbors: Vec<Vec<(usize, f32)>> = vec![Vec::new(); m];
    let mut b_x = vec![0.0f32; m];
    let mut b_y = vec![0.0f32; m];

    for (ii, &v) in interior_vertices.iter().enumerate() {
        let mut rhs = Vec2::ZERO;
        for &(nb, w) in &neighbors_all[v] {
            if !w.is_finite() || w <= 0.0 {
                continue;
            }
            if is_boundary[nb] {
                rhs += uv[nb] * w;
            } else {
                let jj = interior_index[nb];
                if jj != usize::MAX {
                    neighbors[ii].push((jj, w));
                }
            }
        }

        let d = diag_all[v];
        diag[ii] = if d.is_finite() && d > 1e-12 { d } else { 1.0 };
        b_x[ii] = rhs.x;
        b_y[ii] = rhs.y;
    }

    let sol_x = cg_solve(
        &diag,
        &neighbors,
        &b_x,
        opts.solver_tol,
        opts.max_solver_iters,
    );
    let sol_y = cg_solve(
        &diag,
        &neighbors,
        &b_y,
        opts.solver_tol,
        opts.max_solver_iters,
    );

    for (ii, &v) in interior_vertices.iter().enumerate() {
        uv[v] = Vec2::new(sol_x[ii], sol_y[ii]);
    }

    Some(uv)
}

fn chart_has_boundary(tris: &[Tri], chart_tris: &[usize]) -> bool {
    let mut edge_counts: FxHashMap<EdgeKey, u8> = FxHashMap::default();
    for &ti in chart_tris {
        let t = tris[ti];
        let edges = [
            EdgeKey::new(t.pts[0], t.pts[1]),
            EdgeKey::new(t.pts[1], t.pts[2]),
            EdgeKey::new(t.pts[2], t.pts[0]),
        ];
        for e in edges {
            let c = edge_counts.entry(e).or_insert(0);
            *c = c.saturating_add(1);
        }
    }
    edge_counts.values().any(|&c| c == 1)
}

fn split_closed_chart_by_cube_normals(tris: &[Tri], chart_tris: &[usize]) -> Vec<Vec<usize>> {
    let mut bins: [Vec<usize>; 6] = std::array::from_fn(|_| Vec::new());
    for &ti in chart_tris {
        let n = tris[ti].normal;
        let ax = n.x.abs();
        let ay = n.y.abs();
        let az = n.z.abs();
        let bin = if ax >= ay && ax >= az {
            if n.x >= 0.0 {
                0
            } else {
                1
            }
        } else if ay >= az {
            if n.y >= 0.0 {
                2
            } else {
                3
            }
        } else {
            if n.z >= 0.0 {
                4
            } else {
                5
            }
        };
        bins[bin].push(ti);
    }

    let mut out: Vec<Vec<usize>> = Vec::new();

    for bin in bins.iter() {
        if bin.is_empty() {
            continue;
        }
        let mut edge_map: FxHashMap<EdgeKey, Vec<usize>> = FxHashMap::default();
        for &ti in bin.iter() {
            let t = tris[ti];
            let edges = [
                EdgeKey::new(t.pts[0], t.pts[1]),
                EdgeKey::new(t.pts[1], t.pts[2]),
                EdgeKey::new(t.pts[2], t.pts[0]),
            ];
            for e in edges {
                edge_map.entry(e).or_default().push(ti);
            }
        }

        let mut visited: FxHashSet<usize> = FxHashSet::default();
        for &seed in bin.iter() {
            if visited.contains(&seed) {
                continue;
            }
            let mut stack = vec![seed];
            visited.insert(seed);
            let mut comp: Vec<usize> = Vec::new();

            while let Some(cur) = stack.pop() {
                comp.push(cur);
                let t = tris[cur];
                let edges = [
                    EdgeKey::new(t.pts[0], t.pts[1]),
                    EdgeKey::new(t.pts[1], t.pts[2]),
                    EdgeKey::new(t.pts[2], t.pts[0]),
                ];
                for e in edges {
                    if let Some(ts) = edge_map.get(&e) {
                        for &adj in ts {
                            if visited.insert(adj) {
                                stack.push(adj);
                            }
                        }
                    }
                }
            }

            if !comp.is_empty() {
                out.push(comp);
            }
        }
    }

    out
}

#[inline]
fn uv_pca_rotate_in_place(uvs: &mut [Vec2]) {
    if uvs.len() < 3 {
        return;
    }

    let mut mean = Vec2::ZERO;
    for &u in uvs.iter() {
        mean += u;
    }
    mean /= uvs.len() as f32;

    let mut cxx = 0.0f32;
    let mut cxy = 0.0f32;
    let mut cyy = 0.0f32;
    for &u in uvs.iter() {
        let d = u - mean;
        cxx += d.x * d.x;
        cxy += d.x * d.y;
        cyy += d.y * d.y;
    }

    if !cxx.is_finite() || !cxy.is_finite() || !cyy.is_finite() {
        return;
    }

    let angle = 0.5 * (2.0 * cxy).atan2(cxx - cyy);
    if !angle.is_finite() {
        return;
    }

    let (s, c) = (-angle).sin_cos();
    for u in uvs.iter_mut() {
        let d = *u - mean;
        *u = Vec2::new(c * d.x - s * d.y, s * d.x + c * d.y) + mean;
    }
}

fn uv_total_area(tris_local: &[[usize; 3]], uvs_local: &[Vec2]) -> f32 {
    let mut a = 0.0f32;
    for t in tris_local {
        let (ua, ub, uc) = (uvs_local[t[0]], uvs_local[t[1]], uvs_local[t[2]]);
        let area2 = (ub - ua).perp_dot(uc - ua);
        a += 0.5 * area2.abs();
    }
    a
}

fn chart_connected_components(tris: &[Tri], chart_tris: &[usize]) -> Vec<Vec<usize>> {
    if chart_tris.is_empty() {
        return Vec::new();
    }

    let mut edge_map: FxHashMap<EdgeKey, Vec<usize>> = FxHashMap::default();
    for &ti in chart_tris {
        let t = tris[ti];
        let edges = [
            EdgeKey::new(t.pts[0], t.pts[1]),
            EdgeKey::new(t.pts[1], t.pts[2]),
            EdgeKey::new(t.pts[2], t.pts[0]),
        ];
        for e in edges {
            edge_map.entry(e).or_default().push(ti);
        }
    }

    let mut in_chart: FxHashSet<usize> = FxHashSet::default();
    for &ti in chart_tris {
        in_chart.insert(ti);
    }

    let mut visited: FxHashSet<usize> = FxHashSet::default();
    let mut out: Vec<Vec<usize>> = Vec::new();

    for &seed in chart_tris {
        if visited.contains(&seed) {
            continue;
        }
        let mut stack = vec![seed];
        visited.insert(seed);
        let mut comp: Vec<usize> = Vec::new();
        while let Some(cur) = stack.pop() {
            comp.push(cur);
            let t = tris[cur];
            let edges = [
                EdgeKey::new(t.pts[0], t.pts[1]),
                EdgeKey::new(t.pts[1], t.pts[2]),
                EdgeKey::new(t.pts[2], t.pts[0]),
            ];
            for e in edges {
                if let Some(ts) = edge_map.get(&e) {
                    for &adj in ts.iter() {
                        if !in_chart.contains(&adj) {
                            continue;
                        }
                        if visited.insert(adj) {
                            stack.push(adj);
                        }
                    }
                }
            }
        }
        if !comp.is_empty() {
            out.push(comp);
        }
    }

    out
}

fn split_chart_by_cube_normals_prims(tris: &[Tri], chart_tris: &[usize]) -> Vec<Vec<usize>> {
    let mut bins: [Vec<usize>; 6] = std::array::from_fn(|_| Vec::new());

    let mut prim_to_tris: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    let mut prim_nsum: FxHashMap<usize, Vec3> = FxHashMap::default();
    let mut prim_asum: FxHashMap<usize, f32> = FxHashMap::default();
    for &ti in chart_tris {
        let t = tris[ti];
        prim_to_tris.entry(t.prim_dense).or_default().push(ti);
        *prim_nsum.entry(t.prim_dense).or_insert(Vec3::ZERO) += t.normal * t.area;
        *prim_asum.entry(t.prim_dense).or_insert(0.0) += t.area;
    }

    for (prim, tis) in prim_to_tris.into_iter() {
        let nsum = prim_nsum.get(&prim).copied().unwrap_or(Vec3::ZERO);
        let asum = prim_asum.get(&prim).copied().unwrap_or(0.0);
        let n = if nsum.is_finite()
            && nsum.length_squared() > 1e-20
            && asum.is_finite()
            && asum > 0.0
        {
            nsum.normalize()
        } else {
            Vec3::Z
        };
        let ax = n.x.abs();
        let ay = n.y.abs();
        let az = n.z.abs();
        let bin = if ax >= ay && ax >= az {
            if n.x >= 0.0 {
                0
            } else {
                1
            }
        } else if ay >= az {
            if n.y >= 0.0 {
                2
            } else {
                3
            }
        } else {
            if n.z >= 0.0 {
                4
            } else {
                5
            }
        };
        bins[bin].extend(tis);
    }

    let mut out: Vec<Vec<usize>> = Vec::new();
    for b in bins.iter() {
        if b.is_empty() {
            continue;
        }
        let mut comps = chart_connected_components(tris, b);
        out.append(&mut comps);
    }
    out
}

#[inline]
fn orient2(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (b - a).perp_dot(c - a)
}

#[inline]
fn seg_intersect(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2, eps: f32) -> bool {
    // Robust-ish segment intersection in 2D using orientation tests.
    let o1 = orient2(a0, a1, b0);
    let o2 = orient2(a0, a1, b1);
    let o3 = orient2(b0, b1, a0);
    let o4 = orient2(b0, b1, a1);

    let s1 = if o1.abs() <= eps { 0.0 } else { o1.signum() };
    let s2 = if o2.abs() <= eps { 0.0 } else { o2.signum() };
    let s3 = if o3.abs() <= eps { 0.0 } else { o3.signum() };
    let s4 = if o4.abs() <= eps { 0.0 } else { o4.signum() };

    if s1 == 0.0 && s2 == 0.0 && s3 == 0.0 && s4 == 0.0 {
        // Colinear: overlap in 1D projection.
        let (min_ax, max_ax) = (a0.x.min(a1.x), a0.x.max(a1.x));
        let (min_ay, max_ay) = (a0.y.min(a1.y), a0.y.max(a1.y));
        let (min_bx, max_bx) = (b0.x.min(b1.x), b0.x.max(b1.x));
        let (min_by, max_by) = (b0.y.min(b1.y), b0.y.max(b1.y));
        let ox = max_ax + eps >= min_bx && max_bx + eps >= min_ax;
        let oy = max_ay + eps >= min_by && max_by + eps >= min_ay;
        return ox && oy;
    }

    (s1 * s2 <= 0.0) && (s3 * s4 <= 0.0)
}

#[inline]
fn point_in_tri(p: Vec2, a: Vec2, b: Vec2, c: Vec2, eps: f32) -> bool {
    let o1 = orient2(a, b, p);
    let o2 = orient2(b, c, p);
    let o3 = orient2(c, a, p);
    (o1 >= -eps && o2 >= -eps && o3 >= -eps) || (o1 <= eps && o2 <= eps && o3 <= eps)
}

fn tri_tri_intersect(a: [Vec2; 3], b: [Vec2; 3], eps: f32) -> bool {
    // Edge-edge intersections.
    let ae = [(a[0], a[1]), (a[1], a[2]), (a[2], a[0])];
    let be = [(b[0], b[1]), (b[1], b[2]), (b[2], b[0])];
    for (p0, p1) in ae {
        for (q0, q1) in be {
            if seg_intersect(p0, p1, q0, q1, eps) {
                return true;
            }
        }
    }
    // Containment.
    point_in_tri(a[0], b[0], b[1], b[2], eps) || point_in_tri(b[0], a[0], a[1], a[2], eps)
}

fn chart_uv_has_overlap(tris_local: &[[usize; 3]], uvs_local: &[Vec2]) -> bool {
    let ntri = tris_local.len();
    if ntri < 2 {
        return false;
    }

    let mut min = Vec2::new(f32::INFINITY, f32::INFINITY);
    let mut max = Vec2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
    for &u in uvs_local.iter() {
        if !u.is_finite() {
            return true;
        }
        min = min.min(u);
        max = max.max(u);
    }
    if !min.is_finite() || !max.is_finite() {
        return true;
    }

    let ext = (max - min).max(Vec2::splat(1e-6));
    let area = (ext.x * ext.y).abs().max(1e-12);
    let cell = (area / (ntri as f32)).sqrt().max(1e-4) * 1.5;
    let inv = 1.0 / cell;

    let mut grid: FxHashMap<(i32, i32), Vec<usize>> = FxHashMap::default();
    for (ti, t) in tris_local.iter().enumerate() {
        let (a, b, c) = (uvs_local[t[0]], uvs_local[t[1]], uvs_local[t[2]]);
        let tri_min = a.min(b).min(c);
        let tri_max = a.max(b).max(c);
        let x0 = ((tri_min.x - min.x) * inv).floor() as i32;
        let y0 = ((tri_min.y - min.y) * inv).floor() as i32;
        let x1 = ((tri_max.x - min.x) * inv).floor() as i32;
        let y1 = ((tri_max.y - min.y) * inv).floor() as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                grid.entry((x, y)).or_default().push(ti);
            }
        }
    }

    let eps = 1e-7f32;
    let mut tested: FxHashSet<u64> = FxHashSet::default();
    for list in grid.values() {
        if list.len() < 2 {
            continue;
        }
        for ii in 0..list.len() {
            for jj in (ii + 1)..list.len() {
                let a_i = list[ii];
                let b_i = list[jj];
                let (lo, hi) = if a_i <= b_i { (a_i, b_i) } else { (b_i, a_i) };
                let key = ((lo as u64) << 32) | (hi as u64);
                if !tested.insert(key) {
                    continue;
                }

                let ta = tris_local[a_i];
                let tb = tris_local[b_i];
                // Skip pairs that share any vertex (adjacent/corner-touching).
                if ta[0] == tb[0]
                    || ta[0] == tb[1]
                    || ta[0] == tb[2]
                    || ta[1] == tb[0]
                    || ta[1] == tb[1]
                    || ta[1] == tb[2]
                    || ta[2] == tb[0]
                    || ta[2] == tb[1]
                    || ta[2] == tb[2]
                {
                    continue;
                }

                let a = [uvs_local[ta[0]], uvs_local[ta[1]], uvs_local[ta[2]]];
                let b = [uvs_local[tb[0]], uvs_local[tb[1]], uvs_local[tb[2]]];
                if tri_tri_intersect(a, b, eps) {
                    return true;
                }
            }
        }
    }

    false
}

fn chart_uv_has_degenerate_tris(tris_local: &[[usize; 3]], uvs_local: &[Vec2]) -> bool {
    for t in tris_local {
        let (a, b, c) = (uvs_local[t[0]], uvs_local[t[1]], uvs_local[t[2]]);
        let area2 = orient2(a, b, c);
        if !area2.is_finite() || area2.abs() <= 1e-12 {
            return true;
        }
    }
    false
}

fn chart_max_stretch(
    tris_local: &[[usize; 3]],
    uvs_local: &[Vec2],
    points_global: &[usize],
    positions: &[Vec3],
) -> f32 {
    let mut max_s = 1.0f32;
    for t in tris_local {
        let (ia, ib, ic) = (t[0], t[1], t[2]);
        let (pa, pb, pc) = (
            positions[points_global[ia]],
            positions[points_global[ib]],
            positions[points_global[ic]],
        );
        let (ua, ub, uc) = (uvs_local[ia], uvs_local[ib], uvs_local[ic]);

        let l3 = [(pb - pa).length(), (pc - pb).length(), (pa - pc).length()];
        let l2 = [(ub - ua).length(), (uc - ub).length(), (ua - uc).length()];

        let mut min_r = f32::INFINITY;
        let mut max_r = 0.0f32;
        for e in 0..3 {
            let d3 = l3[e];
            let d2 = l2[e];
            if !d3.is_finite() || !d2.is_finite() || d3 <= 1e-9 {
                continue;
            }
            let r = d2 / d3;
            if !r.is_finite() || r <= 0.0 {
                continue;
            }
            min_r = min_r.min(r);
            max_r = max_r.max(r);
        }
        if min_r.is_finite() && max_r.is_finite() && min_r > 0.0 && max_r > 0.0 {
            max_s = max_s.max(max_r / min_r);
        }
    }
    max_s
}

fn split_chart_by_centroid_pca_prims(tris: &[Tri], chart_tris: &[usize]) -> Vec<Vec<usize>> {
    if chart_tris.len() < 8 {
        return vec![chart_tris.to_vec()];
    }

    let mut prim_to_tris: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    let mut prim_csum: FxHashMap<usize, Vec3> = FxHashMap::default();
    let mut prim_asum: FxHashMap<usize, f32> = FxHashMap::default();
    for &ti in chart_tris {
        let t = tris[ti];
        prim_to_tris.entry(t.prim_dense).or_default().push(ti);
        *prim_csum.entry(t.prim_dense).or_insert(Vec3::ZERO) += t.centroid * t.area;
        *prim_asum.entry(t.prim_dense).or_insert(0.0) += t.area;
    }

    if prim_to_tris.len() < 2 {
        return vec![chart_tris.to_vec()];
    }

    let mut mean = Vec3::ZERO;
    let mut wsum = 0.0f32;
    for (prim, _) in prim_to_tris.iter() {
        let a = prim_asum.get(prim).copied().unwrap_or(0.0);
        let c = prim_csum.get(prim).copied().unwrap_or(Vec3::ZERO);
        if a.is_finite() && a > 0.0 {
            mean += c;
            wsum += a;
        }
    }
    if !wsum.is_finite() || wsum <= 0.0 {
        return vec![chart_tris.to_vec()];
    }
    mean /= wsum;

    // Covariance (weighted) of primitive centroids.
    let mut cxx = 0.0f32;
    let mut cxy = 0.0f32;
    let mut cxz = 0.0f32;
    let mut cyy = 0.0f32;
    let mut cyz = 0.0f32;
    let mut czz = 0.0f32;
    for (prim, _) in prim_to_tris.iter() {
        let a = prim_asum.get(prim).copied().unwrap_or(0.0);
        let c = prim_csum.get(prim).copied().unwrap_or(Vec3::ZERO);
        if !a.is_finite() || a <= 0.0 {
            continue;
        }
        let p = c / a - mean;
        cxx += a * p.x * p.x;
        cxy += a * p.x * p.y;
        cxz += a * p.x * p.z;
        cyy += a * p.y * p.y;
        cyz += a * p.y * p.z;
        czz += a * p.z * p.z;
    }

    let mut axis = Vec3::X;
    for _ in 0..10 {
        let v = Vec3::new(
            cxx * axis.x + cxy * axis.y + cxz * axis.z,
            cxy * axis.x + cyy * axis.y + cyz * axis.z,
            cxz * axis.x + cyz * axis.y + czz * axis.z,
        );
        let l2 = v.length_squared();
        if !l2.is_finite() || l2 <= 1e-20 {
            break;
        }
        axis = v / l2.sqrt();
    }
    if !axis.is_finite() || axis.length_squared() <= 1e-12 {
        axis = Vec3::X;
    }

    let mut left: Vec<usize> = Vec::new();
    let mut right: Vec<usize> = Vec::new();

    for (prim, tis) in prim_to_tris.into_iter() {
        let a = prim_asum.get(&prim).copied().unwrap_or(0.0);
        let c = prim_csum.get(&prim).copied().unwrap_or(Vec3::ZERO);
        let center = if a.is_finite() && a > 0.0 {
            c / a
        } else {
            Vec3::ZERO
        };
        if (center - mean).dot(axis) >= 0.0 {
            left.extend(tis);
        } else {
            right.extend(tis);
        }
    }

    if left.is_empty() || right.is_empty() {
        return split_chart_by_cube_normals_prims(tris, chart_tris);
    }

    let mut out = Vec::new();
    out.append(&mut chart_connected_components(tris, &left));
    out.append(&mut chart_connected_components(tris, &right));
    out
}

/// Pure-Rust unwrap aimed at "Auto UV / UV Flatten" quality:
/// - triangulate polygons
/// - chart by normal angle threshold
/// - per-chart harmonic flatten (Dirichlet boundary + Laplacian solve)
/// - area-normalize charts, PCA-align, then shelf-pack into 0..1 atlas
pub fn auto_uv_smart_flatten(
    geo: &Geometry,
    opts: AutoUvSmartFlattenOptions,
) -> Result<AutoUvSmartProjectResult, AutoUvError> {
    let positions = geo
        .get_point_position_attribute()
        .ok_or(AutoUvError::MissingPositions)?;
    if positions.iter().any(|p| !p.is_finite()) {
        return Err(AutoUvError::NonFinitePositions);
    }

    let mut tris: Vec<Tri> = Vec::new();

    for (prim_dense, prim) in geo.primitives().values().iter().enumerate() {
        if !matches!(prim, GeoPrimitive::Polygon(_)) {
            continue;
        }
        let verts = prim.vertices();
        if verts.len() < 3 {
            continue;
        }

        let v0 = verts[0];
        for i in 1..verts.len() - 1 {
            let v1 = verts[i];
            let v2 = verts[i + 1];

            let (Some(p0), Some(p1), Some(p2)) = (
                vid_to_point_dense(geo, v0),
                vid_to_point_dense(geo, v1),
                vid_to_point_dense(geo, v2),
            ) else {
                continue;
            };
            if p0 == p1 || p1 == p2 || p2 == p0 {
                continue;
            }

            let (Some(vd0), Some(vd1), Some(vd2)) = (
                vid_to_vertex_dense(geo, v0),
                vid_to_vertex_dense(geo, v1),
                vid_to_vertex_dense(geo, v2),
            ) else {
                continue;
            };

            let (Some(a), Some(b), Some(c)) =
                (positions.get(p0), positions.get(p1), positions.get(p2))
            else {
                continue;
            };
            let n = (*b - *a).cross(*c - *a);
            let area2 = n.length();
            if !area2.is_finite() || area2 <= 1e-12 {
                continue;
            }
            let normal = n / area2;
            let area = 0.5 * area2;
            let centroid = (*a + *b + *c) * (1.0 / 3.0);
            tris.push(Tri {
                pts: [p0, p1, p2],
                verts: [vd0, vd1, vd2],
                normal,
                area,
                centroid,
                prim_dense,
            });
        }
    }

    if tris.is_empty() {
        return Err(AutoUvError::NoValidTriangles);
    }

    let cos_limit = opts.max_angle_deg.clamp(0.0, 180.0).to_radians().cos();

    let mut edge_map: FxHashMap<EdgeKey, Vec<usize>> = FxHashMap::default();
    for (ti, t) in tris.iter().enumerate() {
        let edges = [
            EdgeKey::new(t.pts[0], t.pts[1]),
            EdgeKey::new(t.pts[1], t.pts[2]),
            EdgeKey::new(t.pts[2], t.pts[0]),
        ];
        for e in edges {
            edge_map.entry(e).or_default().push(ti);
        }
    }

    let mut dsu = Dsu::new(tris.len());
    for tris_on_edge in edge_map.values() {
        if tris_on_edge.len() < 2 {
            continue;
        }
        for i in 0..tris_on_edge.len() {
            for j in (i + 1)..tris_on_edge.len() {
                let a = tris_on_edge[i];
                let b = tris_on_edge[j];
                if tris[a].normal.dot(tris[b].normal) >= cos_limit {
                    dsu.union(a, b);
                }
            }
        }
    }

    let mut root_to_chart: FxHashMap<usize, usize> = FxHashMap::default();
    let mut tri_chart = vec![0usize; tris.len()];
    for ti in 0..tris.len() {
        let root = dsu.find(ti);
        let idx = match root_to_chart.get(&root).copied() {
            Some(idx) => idx,
            None => {
                let idx = root_to_chart.len();
                root_to_chart.insert(root, idx);
                idx
            }
        };
        tri_chart[ti] = idx;
    }

    let init_chart_count = root_to_chart.len().max(1);
    let mut init_chart_tris: Vec<Vec<usize>> = vec![Vec::new(); init_chart_count];
    for (ti, _) in tris.iter().enumerate() {
        init_chart_tris[tri_chart[ti]].push(ti);
    }

    // Ensure charts have boundary: split closed charts by cube-normal bins.
    let mut chart_tris: Vec<Vec<usize>> = Vec::new();
    for ct in init_chart_tris.into_iter() {
        if ct.is_empty() {
            continue;
        }
        if chart_has_boundary(&tris, &ct) {
            chart_tris.push(ct);
        } else {
            let mut sub = split_chart_by_cube_normals_prims(&tris, &ct);
            if sub.is_empty() {
                chart_tris.push(ct);
            } else {
                chart_tris.append(&mut sub);
            }
        }
    }

    if chart_tris.is_empty() {
        return Err(AutoUvError::NoValidTriangles);
    }
    let chart_count = chart_tris.len();
    let mut chart_verts: Vec<Vec<usize>> = vec![Vec::new(); chart_count];
    for (ci, ct) in chart_tris.iter().enumerate() {
        for &ti in ct.iter() {
            let t = tris[ti];
            chart_verts[ci].extend_from_slice(&t.verts);
        }
        chart_verts[ci].sort_unstable();
        chart_verts[ci].dedup();
    }

    let mut uvs = vec![Vec2::ZERO; geo.vertices().len()];
    let mut uv_set = vec![false; geo.vertices().len()];
    let mut chart_bounds: Vec<(Vec2, Vec2)> = Vec::with_capacity(chart_count);

    for ci in 0..chart_count {
        let ct = &chart_tris[ci];
        if ct.is_empty() {
            chart_bounds.push((Vec2::ZERO, Vec2::ZERO));
            continue;
        }

        // Build chart-local point index map + per-point vertex list (for vertex-attribute assignment).
        let mut point_to_local: FxHashMap<usize, usize> = FxHashMap::default();
        let mut points_global: Vec<usize> = Vec::new();
        let mut local_to_vertices: Vec<Vec<usize>> = Vec::new();
        let mut tris_local: Vec<[usize; 3]> = Vec::new();

        for &ti in ct.iter() {
            let t = tris[ti];
            let mut tri = [0usize; 3];
            for k in 0..3 {
                let p = t.pts[k];
                let li = match point_to_local.get(&p).copied() {
                    Some(li) => li,
                    None => {
                        let li = points_global.len();
                        points_global.push(p);
                        point_to_local.insert(p, li);
                        local_to_vertices.push(Vec::new());
                        li
                    }
                };
                tri[k] = li;
                local_to_vertices[li].push(t.verts[k]);
            }
            tris_local.push(tri);
        }

        for vs in local_to_vertices.iter_mut() {
            vs.sort_unstable();
            vs.dedup();
        }

        let mut uv_local = match harmonic_flatten(&points_global, &tris_local, positions, opts) {
            Some(uv) => uv,
            None => {
                // Fallback: area-weighted best-fit planar projection for this chart.
                let mut n_sum = Vec3::ZERO;
                let mut c_sum = Vec3::ZERO;
                let mut area_sum = 0.0f32;
                for &ti in ct.iter() {
                    let t = tris[ti];
                    n_sum += t.normal * t.area;
                    c_sum += t.centroid * t.area;
                    area_sum += t.area;
                }

                let normal = if n_sum.length_squared() > 1e-20 && n_sum.is_finite() {
                    n_sum.normalize()
                } else {
                    Vec3::Z
                };
                let origin = if area_sum.is_finite() && area_sum > 0.0 {
                    c_sum / area_sum
                } else {
                    Vec3::ZERO
                };

                let up = if normal.z.abs() < 0.999 {
                    Vec3::Z
                } else {
                    Vec3::Y
                };
                let mut tangent = normal.cross(up);
                if tangent.length_squared() <= 1e-20 {
                    tangent = normal.cross(Vec3::X);
                }
                let tangent = tangent.normalize();
                let bitangent = normal.cross(tangent).normalize();

                let mut uv = vec![Vec2::ZERO; points_global.len()];
                for (li, &pg) in points_global.iter().enumerate() {
                    let p = positions[pg];
                    let d = p - origin;
                    uv[li] = Vec2::new(d.dot(tangent), d.dot(bitangent));
                }
                uv
            }
        };

        if let Some(uv_lscm) = lscm_flatten(
            &points_global,
            &tris_local,
            positions,
            Some(&uv_local),
            opts,
        ) {
            uv_local = uv_lscm;
        }

        // Area-normalize chart UVs to roughly match 3D surface area.
        let area3d: f32 = ct.iter().map(|&ti| tris[ti].area).sum();
        let area_uv = uv_total_area(&tris_local, &uv_local);
        if area3d.is_finite() && area3d > 0.0 && area_uv.is_finite() && area_uv > 1e-12 {
            let s = (area3d / area_uv).sqrt();
            if s.is_finite() && s > 0.0 {
                for u in uv_local.iter_mut() {
                    *u *= s;
                }
            }
        }

        // PCA-align to reduce AABB area (helps packing).
        uv_pca_rotate_in_place(&mut uv_local);

        // Assign to vertex-attribute buffer (per chart point -> all its vertices).
        for (li, &uv) in uv_local.iter().enumerate() {
            for &vd in local_to_vertices[li].iter() {
                if vd < uvs.len() {
                    uvs[vd] = uv;
                    uv_set[vd] = true;
                }
            }
        }

        // Bounds (in the same UV space we just wrote).
        let mut min = Vec2::new(f32::INFINITY, f32::INFINITY);
        let mut max = Vec2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
        for &u in uv_local.iter() {
            if !u.is_finite() {
                continue;
            }
            min.x = min.x.min(u.x);
            min.y = min.y.min(u.y);
            max.x = max.x.max(u.x);
            max.y = max.y.max(u.y);
        }
        if !min.is_finite() || !max.is_finite() {
            min = Vec2::ZERO;
            max = Vec2::ZERO;
        }
        chart_bounds.push((min, max));
    }

    let charts = chart_bounds.len();
    let mut pack: Vec<PackChart> = chart_bounds
        .iter()
        .enumerate()
        .map(|(id, (min, max))| {
            let mut size = *max - *min;
            if !size.is_finite() {
                size = Vec2::splat(1e-3);
            }
            size.x = size.x.abs().max(1e-3);
            size.y = size.y.abs().max(1e-3);
            PackChart {
                id,
                min: *min,
                size,
                place: Vec2::ZERO,
                rotated: false,
            }
        })
        .collect();

    let (w0, h0) = pack_skyline(&mut pack, 0.0);
    let dim0 = w0.max(h0);
    let pad_world = opts.padding.clamp(0.0, 0.5) * dim0;
    let (w1, h1) = pack_skyline(&mut pack, pad_world);
    let dim1 = w1.max(h1).max(1e-6);
    let scale = (1.0 / dim1).max(0.0);

    let mut placements = vec![Vec2::ZERO; charts];
    let mut rotations = vec![false; charts];
    let mut sizes = vec![Vec2::ZERO; charts];
    for c in pack.iter() {
        placements[c.id] = c.place;
        rotations[c.id] = c.rotated;
        sizes[c.id] = c.size;
    }

    for ci in 0..charts {
        let content_origin = placements[ci] + Vec2::splat(pad_world * 0.5);
        let (min, _) = chart_bounds[ci];
        let rot = rotations[ci];
        let size = sizes[ci];
        for &vd in &chart_verts[ci] {
            if !uv_set.get(vd).copied().unwrap_or(false) {
                continue;
            }
            let mut rel = uvs[vd] - min;
            if rot {
                rel = Vec2::new(rel.y, size.x - rel.x);
            }
            uvs[vd] = (rel + content_origin) * scale;
        }
    }

    Ok(AutoUvSmartProjectResult {
        uvs,
        points: positions.len(),
        vertices: geo.vertices().len(),
        triangles: tris.len(),
        charts,
    })
}
