use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::algorithms::algorithms_runtime::resample as rs;
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::group::ElementGroupMask;
use crate::libs::geometry::ids::{AttributeId, PointId, VertexId};
use crate::mesh::{Attribute, GeoPrimitive, Geometry, PolygonPrim, PolylinePrim};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct ResampleNode;

impl NodeParameters for ResampleNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "group",
                "Group",
                "General",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "method",
                "Method",
                "General",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Uniform Length".into(), 0),
                        ("Fixed Segment Count".into(), 1),
                        ("Adaptive (Curvature)".into(), 2),
                    ],
                },
            ),
            Parameter::new(
                "length",
                "Length / Tolerance",
                "General",
                ParameterValue::Float(0.1),
                ParameterUIType::FloatSlider {
                    min: 0.001,
                    max: 10.0,
                },
            ),
            Parameter::new(
                "segments",
                "Segments",
                "General",
                ParameterValue::Int(10),
                ParameterUIType::IntSlider { min: 1, max: 1000 },
            ),
            Parameter::new(
                "keep_knots",
                "Keep Knots",
                "General",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "poly_edges",
                "Resample Polygon Edges",
                "General",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "maintain_order",
                "Maintain Primitive Order",
                "General",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "out_tangent",
                "Output TangentU (@tangentu)",
                "Output",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "out_curveu",
                "Output CurveU (@curveu)",
                "Output",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for ResampleNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let default_geo = Arc::new(Geometry::new());
        let input_geo = mats.first().unwrap_or(&default_geo);

        let group_str = get_param_string(params, "group", "");
        let method = get_param_int(params, "method", 0);
        let length_or_tol = get_param_float(params, "length", 0.1).max(1e-6);
        let segments = get_param_int(params, "segments", 10).max(1) as usize;
        let keep_knots = get_param_bool(params, "keep_knots", true);
        let poly_edges = get_param_bool(params, "poly_edges", true);
        let out_tangent = get_param_bool(params, "out_tangent", true);
        let out_curveu = get_param_bool(params, "out_curveu", true);

        let prim_count = input_geo.primitives().len();
        let selection = if group_str.is_empty() {
            let mut mask = ElementGroupMask::new(prim_count);
            mask.invert();
            mask
        } else {
            input_geo
                .primitive_groups
                .get(&AttributeId::from(group_str.as_str()))
                .cloned()
                .unwrap_or_else(|| {
                    crate::nodes::group::utils::parse_pattern(&group_str, prim_count)
                })
        };
        if selection.is_empty() {
            return input_geo.clone();
        }

        let Some(p_attr) = input_geo.get_point_attribute(attrs::P) else {
            return Arc::new(Geometry::new());
        };
        let p_slice = p_attr.as_slice::<Vec3>();
        let p_paged = p_attr.as_paged::<Vec3>();
        if p_slice.is_none() && p_paged.is_none() {
            return Arc::new(Geometry::new());
        }
        let get_p = |geo: &Geometry, pid: PointId| -> Option<Vec3> {
            let di = geo.points().get_dense_index(pid.into())?;
            p_slice
                .and_then(|s| s.get(di).copied())
                .or_else(|| p_paged.and_then(|pb| pb.get(di)))
        };

        // Work on a fork so we preserve all existing attributes/groups; we only add new points/vertices and rewrite selected primitives.
        let mut out_geo = input_geo.fork();

        // Ensure vertex-domain outputs (avoid point-domain conflicts on shared mesh points).
        if out_tangent && out_geo.get_vertex_attribute(attrs::TANGENTU).is_none() {
            out_geo.insert_vertex_attribute(
                attrs::TANGENTU,
                Attribute::new_auto(vec![Vec3::ZERO; out_geo.vertices().len()]),
            );
        }
        if out_curveu && out_geo.get_vertex_attribute(attrs::CURVEU).is_none() {
            out_geo.insert_vertex_attribute(
                attrs::CURVEU,
                Attribute::new_auto(vec![0.0f32; out_geo.vertices().len()]),
            );
        }

        let set_v3 = |geo: &mut Geometry, name: &str, vid: VertexId, v: Vec3| -> bool {
            let Some(di) = geo.vertices().get_dense_index(vid.into()) else {
                return false;
            };
            let Some(attr) = geo.get_vertex_attribute_mut(name) else {
                return false;
            };
            if let Some(s) = attr.as_mut_slice::<Vec3>() {
                if let Some(x) = s.get_mut(di) {
                    *x = v;
                    return true;
                }
            }
            if let Some(pb) = attr.as_paged_mut::<Vec3>() {
                if let Some(x) = pb.get_mut(di) {
                    *x = v;
                    return true;
                }
            }
            false
        };
        let set_f32 = |geo: &mut Geometry, name: &str, vid: VertexId, v: f32| -> bool {
            let Some(di) = geo.vertices().get_dense_index(vid.into()) else {
                return false;
            };
            let Some(attr) = geo.get_vertex_attribute_mut(name) else {
                return false;
            };
            if let Some(s) = attr.as_mut_slice::<f32>() {
                if let Some(x) = s.get_mut(di) {
                    *x = v;
                    return true;
                }
            }
            if let Some(pb) = attr.as_paged_mut::<f32>() {
                if let Some(x) = pb.get_mut(di) {
                    *x = v;
                    return true;
                }
            }
            false
        };
        let set_p = |geo: &mut Geometry, pid: PointId, p: Vec3| -> bool {
            let Some(di) = geo.points().get_dense_index(pid.into()) else {
                return false;
            };
            let Some(attr) = geo.get_point_attribute_mut(attrs::P) else {
                return false;
            };
            if let Some(s) = attr.as_mut_slice::<Vec3>() {
                if let Some(x) = s.get_mut(di) {
                    *x = p;
                    return true;
                }
            }
            if let Some(pb) = attr.as_paged_mut::<Vec3>() {
                if let Some(x) = pb.get_mut(di) {
                    *x = p;
                    return true;
                }
            }
            false
        };

        // --- Polygon edge welding table: undirected edge -> interior points (shared) ---
        #[inline]
        fn edge_key(a: PointId, b: PointId) -> (PointId, PointId) {
            if (a.index, a.generation) <= (b.index, b.generation) {
                (a, b)
            } else {
                (b, a)
            }
        }
        let mut edge_to_pts: HashMap<(PointId, PointId), Vec<PointId>> = HashMap::new();

        // First pass: resample all unique edges for selected polygons (if enabled).
        if poly_edges {
            for (prim_i, prim) in input_geo.primitives().iter().enumerate() {
                if !selection.get(prim_i) {
                    continue;
                }
                let GeoPrimitive::Polygon(poly) = prim else {
                    continue;
                };
                let n = poly.vertices.len();
                if n < 3 {
                    continue;
                }
                let mut ring: Vec<PointId> = Vec::with_capacity(n);
                for &vid in &poly.vertices {
                    let Some(v) = input_geo.vertices().get(vid.into()) else {
                        ring.clear();
                        break;
                    };
                    ring.push(v.point_id);
                }
                if ring.len() != n {
                    continue;
                }
                for i in 0..n {
                    let a = ring[i];
                    let b = ring[(i + 1) % n];
                    let k = edge_key(a, b);
                    if edge_to_pts.contains_key(&k) {
                        continue;
                    }
                    let (Some(pa), Some(pb)) = (get_p(input_geo, a), get_p(input_geo, b)) else {
                        return Arc::new(Geometry::new());
                    };
                    let pts = match method {
                        1 => resample_segment_by_count(pa, pb, segments)
                            .into_iter()
                            .skip(1)
                            .take(segments.saturating_sub(1))
                            .collect::<Vec<_>>(),
                        _ => resample_segment_by_length(pa, pb, length_or_tol),
                    };
                    if pts.is_empty() {
                        edge_to_pts.insert(k, Vec::new());
                        continue;
                    }
                    let mut ids: Vec<PointId> = Vec::with_capacity(pts.len());
                    for p in pts {
                        let np = out_geo.add_point();
                        if !set_p(&mut out_geo, np, p) {
                            return Arc::new(Geometry::new());
                        }
                        ids.push(np);
                    }
                    // Important: store points in canonical key direction (k.0 -> k.1) so reconstruction can
                    // deterministically reverse based on edge direction without depending on "first seen" winding.
                    if k.0 != a {
                        ids.reverse();
                    }
                    edge_to_pts.insert(k, ids);
                }
            }
        }

        // Second pass: rewrite selected primitives.
        for (prim_i, prim) in input_geo.primitives().iter().enumerate() {
            if !selection.get(prim_i) {
                continue;
            }
            let Some(pid) = out_geo.primitives().get_id_from_dense(prim_i) else {
                continue;
            };

            let new_prim = match prim {
                GeoPrimitive::Polygon(poly) if poly_edges => {
                    let n = poly.vertices.len();
                    if n < 3 {
                        None
                    } else {
                        let mut ring: Vec<(VertexId, PointId)> = Vec::with_capacity(n);
                        for &vid in &poly.vertices {
                            let Some(v) = input_geo.vertices().get(vid.into()) else {
                                ring.clear();
                                break;
                            };
                            ring.push((vid, v.point_id));
                        }
                        if ring.len() != n {
                            None
                        } else {
                            let mut new_verts: Vec<VertexId> = Vec::new();
                            for i in 0..n {
                                let (v_a, p_a) = ring[i];
                                let (_v_b, p_b) = ring[(i + 1) % n];
                                new_verts.push(v_a); // reuse original corner vertex id

                                let k = edge_key(p_a, p_b);
                                if let Some(mid_pts) = edge_to_pts.get(&k) {
                                    if !mid_pts.is_empty() {
                                        let forward = k.0 == p_a && k.1 == p_b;
                                        let it: Box<dyn Iterator<Item = &PointId>> = if forward {
                                            Box::new(mid_pts.iter())
                                        } else {
                                            Box::new(mid_pts.iter().rev())
                                        };
                                        for &mp in it {
                                            let nv = out_geo.add_vertex(mp);
                                            new_verts.push(nv);
                                        }
                                    }
                                }
                            }
                            Some(GeoPrimitive::Polygon(PolygonPrim {
                                vertices: new_verts,
                            }))
                        }
                    }
                }
                GeoPrimitive::Polyline(pl) => {
                    if pl.vertices.len() < 2 {
                        continue;
                    }
                    let mut pts: Vec<Vec3> = Vec::with_capacity(pl.vertices.len());
                    let mut vtx: Vec<VertexId> = Vec::with_capacity(pl.vertices.len());
                    for &vid in &pl.vertices {
                        let Some(v) = input_geo.vertices().get(vid.into()) else {
                            pts.clear();
                            vtx.clear();
                            break;
                        };
                        let Some(p) = get_p(input_geo, v.point_id) else {
                            pts.clear();
                            vtx.clear();
                            break;
                        };
                        pts.push(p);
                        vtx.push(vid);
                    }
                    if pts.len() < 2 {
                        None
                    } else {
                        let (new_positions, new_u) = match method {
                            1 => (sample_polyline_by_count(&pts, pl.closed, segments), None),
                            _ => {
                                let (p, u, _) = rs::resample_polyline(
                                    &pts,
                                    pl.closed,
                                    length_or_tol,
                                    1_000_000,
                                );
                                (p, Some(u))
                            }
                        };
                        if new_positions.len() < 2 {
                            None
                        } else {
                            let mut out_vids: Vec<VertexId> =
                                Vec::with_capacity(new_positions.len());
                            // map endpoints to existing vertices if keep_knots=true (for polylines this means keep original verts)
                            // Always keep first/last if open, and keep original corners only when exact match.
                            for (i, p) in new_positions.iter().enumerate() {
                                let use_existing = keep_knots
                                    && ((i == 0) || (!pl.closed && i + 1 == new_positions.len()));
                                let vid = if use_existing {
                                    if i == 0 {
                                        vtx[0]
                                    } else {
                                        *vtx.last().unwrap()
                                    }
                                } else {
                                    let np = out_geo.add_point();
                                    if !set_p(&mut out_geo, np, *p) {
                                        return Arc::new(Geometry::new());
                                    }
                                    out_geo.add_vertex(np)
                                };
                                out_vids.push(vid);
                            }
                            // Vertex attributes
                            if out_curveu {
                                if let Some(u) = &new_u {
                                    for (i, &vid) in out_vids.iter().enumerate() {
                                        if !set_f32(
                                            &mut out_geo,
                                            attrs::CURVEU,
                                            vid,
                                            *u.get(i).unwrap_or(&0.0),
                                        ) {
                                            return Arc::new(Geometry::new());
                                        }
                                    }
                                }
                            }
                            if out_tangent {
                                let tans = tangents_from_positions(&new_positions, pl.closed);
                                for (i, &vid) in out_vids.iter().enumerate() {
                                    if !set_v3(&mut out_geo, attrs::TANGENTU, vid, tans[i]) {
                                        return Arc::new(Geometry::new());
                                    }
                                }
                            }
                            Some(GeoPrimitive::Polyline(PolylinePrim {
                                vertices: out_vids,
                                closed: pl.closed,
                            }))
                        }
                    }
                }
                GeoPrimitive::BezierCurve(c) => {
                    if c.vertices.len() < 2 {
                        continue;
                    }
                    // Strict: require curve knot attrs to exist and match point count.
                    let pos_attr = input_geo.get_point_attribute(attrs::P);
                    let tin_attr = input_geo.get_point_attribute(attrs::KNOT_TIN);
                    let tout_attr = input_geo.get_point_attribute(attrs::KNOT_TOUT);
                    let rot_attr = input_geo.get_point_attribute(attrs::KNOT_ROT);
                    let (Some(pos_a), Some(tin_a), Some(tout_a), Some(rot_a)) =
                        (pos_attr, tin_attr, tout_attr, rot_attr)
                    else {
                        return Arc::new(Geometry::new());
                    };
                    let (pos_s, pos_p) = (pos_a.as_slice::<Vec3>(), pos_a.as_paged::<Vec3>());
                    let (tin_s, tin_p) = (tin_a.as_slice::<Vec3>(), tin_a.as_paged::<Vec3>());
                    let (tout_s, tout_p) = (tout_a.as_slice::<Vec3>(), tout_a.as_paged::<Vec3>());
                    let (rot_s, rot_p) = (rot_a.as_slice::<Quat>(), rot_a.as_paged::<Quat>());
                    let pcount = input_geo.points().len();
                    let ok = (pos_s.map(|s| s.len()).or_else(|| pos_p.map(|pb| pb.len()))
                        == Some(pcount))
                        && (tin_s.map(|s| s.len()).or_else(|| tin_p.map(|pb| pb.len()))
                            == Some(pcount))
                        && (tout_s
                            .map(|s| s.len())
                            .or_else(|| tout_p.map(|pb| pb.len()))
                            == Some(pcount))
                        && (rot_s.map(|s| s.len()).or_else(|| rot_p.map(|pb| pb.len()))
                            == Some(pcount));
                    if !ok {
                        return Arc::new(Geometry::new());
                    }
                    let get_a3 =
                        |di: usize,
                         s: Option<&[Vec3]>,
                         p: Option<&crate::libs::algorithms::algorithms_dcc::PagedBuffer<Vec3>>|
                         -> Option<Vec3> {
                            s.and_then(|x| x.get(di).copied())
                                .or_else(|| p.and_then(|pb| pb.get(di)))
                        };
                    let get_aq =
                        |di: usize,
                         s: Option<&[Quat]>,
                         p: Option<&crate::libs::algorithms::algorithms_dcc::PagedBuffer<Quat>>|
                         -> Option<Quat> {
                            s.and_then(|x| x.get(di).copied())
                                .or_else(|| p.and_then(|pb| pb.get(di)))
                        };
                    let mut knots: Vec<crate::libs::algorithms::algorithms_runtime::unity_spline::unity_spline::BezierKnot> = Vec::with_capacity(c.vertices.len());
                    for &vid in &c.vertices {
                        let Some(v) = input_geo.vertices().get(vid.into()) else {
                            return Arc::new(Geometry::new());
                        };
                        let Some(di) = input_geo.points().get_dense_index(v.point_id.into()) else {
                            return Arc::new(Geometry::new());
                        };
                        let (Some(p0), Some(ti), Some(to), Some(r)) = (
                            get_a3(di, pos_s, pos_p),
                            get_a3(di, tin_s, tin_p),
                            get_a3(di, tout_s, tout_p),
                            get_aq(di, rot_s, rot_p),
                        ) else {
                            return Arc::new(Geometry::new());
                        };
                        knots.push(crate::libs::algorithms::algorithms_runtime::unity_spline::unity_spline::BezierKnot { position: p0, tangent_in: ti, tangent_out: to, rotation: r });
                    }

                    let mut policy = rs::ResamplePolicy::default();
                    match method {
                        2 => {
                            policy.max_error = length_or_tol;
                        } // adaptive tolerance
                        _ => {
                            policy.max_error = length_or_tol.max(1e-4);
                        } // base polyline for uniform/fixed (deterministic)
                    }
                    let (base_p, base_u, _base_sp) =
                        rs::resample_bezier_knots(&knots, c.closed, policy);
                    if base_p.len() < 2 {
                        continue;
                    }

                    // Uniform/fixed refine on polyline so it's not only curvature-based.
                    let (final_p, final_u) = match method {
                        1 => {
                            let p = sample_polyline_by_count(&base_p, c.closed, segments);
                            (p, None)
                        }
                        0 => {
                            let (p, u, _) =
                                rs::resample_polyline(&base_p, c.closed, length_or_tol, 1_000_000);
                            (p, Some(u))
                        }
                        _ => (base_p, Some(base_u)),
                    };
                    if final_p.len() < 2 {
                        continue;
                    }

                    // Build vertex list: always keep original knot vertices in order, and add new verts between them.
                    // For simplicity + determinism: if keep_knots, we ensure we reuse knot vertices at span boundaries; otherwise new points everywhere.
                    let tans = tangents_from_positions(&final_p, c.closed);
                    let mut out_vids: Vec<VertexId> = Vec::with_capacity(final_p.len());
                    for i in 0..final_p.len() {
                        let pid = if keep_knots
                            && i < c.vertices.len()
                            && (final_p[i] - knots[i].position).length_squared() < 1e-10
                        {
                            // reuse existing knot point
                            let Some(v) = input_geo.vertices().get(c.vertices[i].into()) else {
                                return Arc::new(Geometry::new());
                            };
                            v.point_id
                        } else {
                            let np = out_geo.add_point();
                            if !set_p(&mut out_geo, np, final_p[i]) {
                                return Arc::new(Geometry::new());
                            }
                            np
                        };
                        let vid = out_geo.add_vertex(pid);
                        out_vids.push(vid);
                        if out_tangent {
                            if !set_v3(&mut out_geo, attrs::TANGENTU, vid, tans[i]) {
                                return Arc::new(Geometry::new());
                            }
                        }
                        if out_curveu {
                            let u = final_u.as_ref().and_then(|u| u.get(i).copied()).unwrap_or(
                                (i as f32 / (final_p.len().saturating_sub(1).max(1)) as f32),
                            );
                            if !set_f32(&mut out_geo, attrs::CURVEU, vid, u) {
                                return Arc::new(Geometry::new());
                            }
                        }
                    }
                    // Bake curve to polyline
                    Some(GeoPrimitive::Polyline(PolylinePrim {
                        vertices: out_vids,
                        closed: c.closed,
                    }))
                }
                _ => None,
            };

            if let Some(new_prim) = new_prim {
                if let Some(dst) = out_geo.primitives_mut().get_mut(pid) {
                    *dst = new_prim;
                }
            }
        }

        Arc::new(out_geo)
    }
}

fn tangents_from_positions(p: &[Vec3], closed: bool) -> Vec<Vec3> {
    let n = p.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![Vec3::Y];
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = if closed {
            (p[(i + 1) % n] - p[(i + n - 1) % n]).normalize_or_zero()
        } else if i == 0 {
            (p[1] - p[0]).normalize_or_zero()
        } else if i + 1 == n {
            (p[n - 1] - p[n - 2]).normalize_or_zero()
        } else {
            (p[i + 1] - p[i - 1]).normalize_or_zero()
        };
        out.push(t);
    }
    out
}

fn resample_segment_by_length(a: Vec3, b: Vec3, seg_len: f32) -> Vec<Vec3> {
    let len = (b - a).length();
    if len <= seg_len * 1.0000001 {
        return Vec::new();
    }
    let steps = (len / seg_len).floor() as usize;
    if steps == 0 {
        return Vec::new();
    }
    (1..=steps)
        .map(|i| a.lerp(b, i as f32 / (steps as f32 + 1.0)))
        .collect()
}

fn resample_segment_by_count(a: Vec3, b: Vec3, segments: usize) -> Vec<Vec3> {
    let segs = segments.max(1);
    (0..=segs)
        .map(|i| a.lerp(b, i as f32 / segs as f32))
        .collect()
}

fn polyline_total_len(points: &[Vec3], closed: bool) -> f32 {
    if points.len() < 2 {
        return 0.0;
    }
    let segs = if closed {
        points.len()
    } else {
        points.len() - 1
    };
    let mut t = 0.0;
    for i in 0..segs {
        t += (points[(i + 1) % points.len()] - points[i]).length();
    }
    t
}

fn sample_polyline_by_count(points: &[Vec3], closed: bool, segments: usize) -> Vec<Vec3> {
    let segs = segments.max(1);
    let total = polyline_total_len(points, closed);
    if total <= 1e-20 {
        return Vec::new();
    }
    let want_pts = if closed { segs } else { segs + 1 };
    let mut out = Vec::with_capacity(want_pts);
    let mut target = 0.0f32;
    let step = total / segs as f32;
    let mut acc = 0.0f32;
    let edge_count = if closed {
        points.len()
    } else {
        points.len() - 1
    };
    let mut cur = points[0];
    out.push(cur);
    for e in 0..edge_count {
        let a = points[e];
        let b = points[(e + 1) % points.len()];
        let len = (b - a).length();
        while out.len() < want_pts && acc + len >= target + step - 1e-8 {
            let t = ((target + step - acc) / len.max(1e-20)).clamp(0.0, 1.0);
            cur = a.lerp(b, t);
            out.push(cur);
            target += step;
        }
        acc += len;
    }
    if closed && out.len() > 1 {
        if (out[0] - *out.last().unwrap()).length_squared() < 1e-10 {
            out.pop();
        }
    }
    out
}

fn get_param_string(params: &[Parameter], name: &str, default: &str) -> String {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| default.to_string())
}
fn get_param_int(params: &[Parameter], name: &str, default: i32) -> i32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::Int(i) => Some(*i),
            _ => None,
        })
        .unwrap_or(default)
}
fn get_param_bool(params: &[Parameter], name: &str, default: bool) -> bool {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::Bool(b) => Some(*b),
            _ => None,
        })
        .unwrap_or(default)
}
fn get_param_float(params: &[Parameter], name: &str, default: f32) -> f32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::Float(f) => Some(*f),
            _ => None,
        })
        .unwrap_or(default)
}

register_node!("Resample", "Modeling", ResampleNode);
