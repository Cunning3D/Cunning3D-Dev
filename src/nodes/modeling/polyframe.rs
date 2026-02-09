use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::group::ElementGroupMask;
use crate::libs::geometry::ids::{AttributeId, PointId, VertexId};
use crate::mesh::{Attribute, GeoPrimitive, Geometry};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
enum Entity {
    Primitives = 0,
    Points = 1,
    Vertices = 2,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
enum Style {
    FirstEdge = 0,
    TwoEdges = 1,
    PrimitiveCentroid = 2,
    TextureUV = 3,
    TextureUVGradient = 4,
    AttributeGradient = 5,
    MikkT = 6,
}

#[derive(Clone, Copy, Debug)]
struct Frame {
    n: Vec3,
    tu: Vec3,
    tv: Vec3,
}

#[derive(Clone, Copy)]
struct Tri {
    v0: VertexId,
    v1: VertexId,
    v2: VertexId,
    p0: Vec3,
    p1: Vec3,
    p2: Vec3,
}

#[derive(Default)]
pub struct PolyFrameNode;

impl NodeParameters for PolyFrameNode {
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
                "entity",
                "Entity",
                "General",
                ParameterValue::Int(Entity::Primitives as i32),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Primitives".into(), 0),
                        ("Points".into(), 1),
                        ("Vertices".into(), 2),
                    ],
                },
            ),
            Parameter::new(
                "style",
                "Style",
                "General",
                ParameterValue::Int(Style::FirstEdge as i32),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("First Edge".into(), 0),
                        ("Two Edges".into(), 1),
                        ("Primitive Centroid".into(), 2),
                        ("Texture UV".into(), 3),
                        ("Texture UV Gradient".into(), 4),
                        ("Attribute Gradient".into(), 5),
                        ("MikkT".into(), 6),
                    ],
                },
            ),
            Parameter::new(
                "attrib_name",
                "Attribute Name",
                "General",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "out_n",
                "Normal Name",
                "Output",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "n_name",
                "",
                "Output",
                ParameterValue::String("N".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "out_tu",
                "Tangent Name",
                "Output",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "tu_name",
                "",
                "Output",
                ParameterValue::String("tangentu".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "out_tv",
                "Bitangent Name",
                "Output",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "tv_name",
                "",
                "Output",
                ParameterValue::String("tangentv".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "orthogonal",
                "Make Frame Orthogonal",
                "Options",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "left_handed",
                "Left-Handed Frame",
                "Options",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for PolyFrameNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let default_geo = Arc::new(Geometry::new());
        let input_geo = mats.first().unwrap_or(&default_geo);
        let mut geo = input_geo.fork();

        let group_str = get_param_string(params, "group", "");
        let entity = get_param_int(params, "entity", Entity::Primitives as i32);
        let style = get_param_int(params, "style", Style::FirstEdge as i32);
        let attrib_name = get_param_string(params, "attrib_name", "");

        let out_n = get_param_bool(params, "out_n", true);
        let out_tu = get_param_bool(params, "out_tu", true);
        let out_tv = get_param_bool(params, "out_tv", false);
        let orth = get_param_bool(params, "orthogonal", true);
        let left = get_param_bool(params, "left_handed", false);

        let n_name = norm_attr_name(&get_param_string(params, "n_name", "N"));
        let tu_name = norm_attr_name(&get_param_string(params, "tu_name", "tangentu"));
        let tv_name = norm_attr_name(&get_param_string(params, "tv_name", "tangentv"));

        let prim_count = geo.primitives().len();
        let pt_count = geo.points().len();
        let vtx_count = geo.vertices().len();

        let Some(p_attr) = geo.get_point_attribute(attrs::P) else {
            return Arc::new(Geometry::new());
        };
        let p_s = p_attr.as_slice::<Vec3>();
        let p_p = p_attr.as_paged::<Vec3>();
        if p_s.is_none() && p_p.is_none() {
            return Arc::new(Geometry::new());
        }
        let get_p = |geo: &Geometry, pid: PointId| -> Option<Vec3> {
            let di = geo.points().get_dense_index(pid.into())?;
            p_s.and_then(|s| s.get(di).copied())
                .or_else(|| p_p.and_then(|pb| pb.get(di)))
        };

        let sel = match entity {
            x if x == Entity::Points as i32 => {
                if group_str.is_empty() {
                    let mut m = ElementGroupMask::new(pt_count);
                    m.invert();
                    m
                } else {
                    geo.point_groups
                        .get(&AttributeId::from(group_str.as_str()))
                        .cloned()
                        .unwrap_or_else(|| {
                            crate::nodes::group::utils::parse_pattern(&group_str, pt_count)
                        })
                }
            }
            x if x == Entity::Vertices as i32 => {
                if group_str.is_empty() {
                    let mut m = ElementGroupMask::new(vtx_count);
                    m.invert();
                    m
                } else {
                    geo.vertex_groups
                        .get(&AttributeId::from(group_str.as_str()))
                        .cloned()
                        .unwrap_or_else(|| {
                            crate::nodes::group::utils::parse_pattern(&group_str, vtx_count)
                        })
                }
            }
            _ => {
                if group_str.is_empty() {
                    let mut m = ElementGroupMask::new(prim_count);
                    m.invert();
                    m
                } else {
                    geo.primitive_groups
                        .get(&AttributeId::from(group_str.as_str()))
                        .cloned()
                        .unwrap_or_else(|| {
                            crate::nodes::group::utils::parse_pattern(&group_str, prim_count)
                        })
                }
            }
        };
        if sel.is_empty() {
            return Arc::new(geo);
        }

        // Compute outputs without mutating `geo` (avoids borrow conflicts with cached attribute refs).
        let mut prim_n: Option<Vec<Vec3>> = None;
        let mut prim_tu: Option<Vec<Vec3>> = None;
        let mut prim_tv: Option<Vec<Vec3>> = None;
        let mut pt_n: Option<Vec<Vec3>> = None;
        let mut pt_tu: Option<Vec<Vec3>> = None;
        let mut pt_tv: Option<Vec<Vec3>> = None;
        let mut vtx_n: Option<Vec<Vec3>> = None;
        let mut vtx_tu: Option<Vec<Vec3>> = None;
        let mut vtx_tv: Option<Vec<Vec3>> = None;

        {
            // Precompute MikkT tangents if requested.
            let mikkt = style == Style::MikkT as i32;
            let mikkt_data = if mikkt {
                match compute_mikkt(&geo, &get_p, left, orth) {
                    Some(d) => Some(d),
                    None => return Arc::new(Geometry::new()),
                }
            } else {
                None
            };

            match entity {
                x if x == Entity::Primitives as i32 => {
                    if out_n {
                        prim_n = Some(vec![Vec3::ZERO; prim_count]);
                    }
                    if out_tu {
                        prim_tu = Some(vec![Vec3::ZERO; prim_count]);
                    }
                    if out_tv {
                        prim_tv = Some(vec![Vec3::ZERO; prim_count]);
                    }
                    for prim_di in sel.iter_ones() {
                        let Some(pid) = geo.primitives().get_id_from_dense(prim_di) else {
                            continue;
                        };
                        let Some(src) = geo.primitives().get(pid) else {
                            continue;
                        };
                        let fr = if let Some(d) = &mikkt_data {
                            match frame_from_mikkt_prim(&geo, src, d, left, orth) {
                                Some(f) => f,
                                None => return Arc::new(Geometry::new()),
                            }
                        } else {
                            match compute_frame_for_primitive(
                                &geo,
                                src,
                                style,
                                &attrib_name,
                                &get_p,
                                left,
                                orth,
                            ) {
                                Some(f) => f,
                                None => return Arc::new(Geometry::new()),
                            }
                        };
                        if let Some(v) = prim_n.as_mut() {
                            v[prim_di] = fr.n;
                        }
                        if let Some(v) = prim_tu.as_mut() {
                            v[prim_di] = fr.tu;
                        }
                        if let Some(v) = prim_tv.as_mut() {
                            v[prim_di] = fr.tv;
                        }
                    }
                }
                x if x == Entity::Points as i32 => {
                    if out_n {
                        pt_n = Some(vec![Vec3::ZERO; pt_count]);
                    }
                    if out_tu {
                        pt_tu = Some(vec![Vec3::ZERO; pt_count]);
                    }
                    if out_tv {
                        pt_tv = Some(vec![Vec3::ZERO; pt_count]);
                    }

                    let mut n_acc = vec![Vec3::ZERO; pt_count];
                    let mut tu_acc = vec![Vec3::ZERO; pt_count];
                    let mut tv_acc = vec![Vec3::ZERO; pt_count];
                    let mut cnt = vec![0u32; pt_count];

                    if let Some(d) = &mikkt_data {
                        for (vdi, v) in geo.vertices().values().iter().enumerate() {
                            let Some(pdi) = geo.points().get_dense_index(v.point_id.into()) else {
                                continue;
                            };
                            if !sel.get(pdi) {
                                continue;
                            }
                            let fr = d.frames.get(vdi).copied().unwrap_or(Frame {
                                n: Vec3::Y,
                                tu: Vec3::X,
                                tv: Vec3::Z,
                            });
                            n_acc[pdi] += fr.n;
                            tu_acc[pdi] += fr.tu;
                            tv_acc[pdi] += fr.tv;
                            cnt[pdi] += 1;
                        }
                    } else {
                        for prim in geo.primitives().iter() {
                            let fr = match compute_frame_for_primitive(
                                &geo,
                                prim,
                                style,
                                &attrib_name,
                                &get_p,
                                left,
                                orth,
                            ) {
                                Some(f) => f,
                                None => continue,
                            };
                            for &vid in prim.vertices() {
                                let Some(v) = geo.vertices().get(vid.into()) else {
                                    continue;
                                };
                                let Some(pdi) = geo.points().get_dense_index(v.point_id.into())
                                else {
                                    continue;
                                };
                                if !sel.get(pdi) {
                                    continue;
                                }
                                n_acc[pdi] += fr.n;
                                tu_acc[pdi] += fr.tu;
                                tv_acc[pdi] += fr.tv;
                                cnt[pdi] += 1;
                            }
                        }
                    }

                    for pdi in sel.iter_ones() {
                        if cnt[pdi] == 0 {
                            continue;
                        }
                        let inv = 1.0 / cnt[pdi] as f32;
                        let mut n = (n_acc[pdi] * inv).normalize_or_zero();
                        let mut tu = (tu_acc[pdi] * inv).normalize_or_zero();
                        let mut tv = (tv_acc[pdi] * inv).normalize_or_zero();
                        if orth {
                            tu = (tu - n * tu.dot(n)).normalize_or_zero();
                            tv = if left { tu.cross(n) } else { n.cross(tu) };
                            n = if left { tv.cross(tu) } else { tu.cross(tv) }.normalize_or_zero();
                        }
                        if let Some(v) = pt_n.as_mut() {
                            v[pdi] = n;
                        }
                        if let Some(v) = pt_tu.as_mut() {
                            v[pdi] = tu;
                        }
                        if let Some(v) = pt_tv.as_mut() {
                            v[pdi] = tv;
                        }
                    }
                }
                _ => {
                    if out_n {
                        vtx_n = Some(vec![Vec3::ZERO; vtx_count]);
                    }
                    if out_tu {
                        vtx_tu = Some(vec![Vec3::ZERO; vtx_count]);
                    }
                    if out_tv {
                        vtx_tv = Some(vec![Vec3::ZERO; vtx_count]);
                    }

                    if let Some(d) = &mikkt_data {
                        for vdi in sel.iter_ones() {
                            let fr = d.frames.get(vdi).copied().unwrap_or(Frame {
                                n: Vec3::Y,
                                tu: Vec3::X,
                                tv: Vec3::Z,
                            });
                            if let Some(v) = vtx_n.as_mut() {
                                v[vdi] = fr.n;
                            }
                            if let Some(v) = vtx_tu.as_mut() {
                                v[vdi] = fr.tu;
                            }
                            if let Some(v) = vtx_tv.as_mut() {
                                v[vdi] = fr.tv;
                            }
                        }
                    } else {
                        for prim in geo.primitives().iter() {
                            let fr = match compute_frame_for_primitive(
                                &geo,
                                prim,
                                style,
                                &attrib_name,
                                &get_p,
                                left,
                                orth,
                            ) {
                                Some(f) => f,
                                None => continue,
                            };
                            for &vid in prim.vertices() {
                                let Some(vdi) = geo.vertices().get_dense_index(vid.into()) else {
                                    continue;
                                };
                                if !sel.get(vdi) {
                                    continue;
                                }
                                if let Some(v) = vtx_n.as_mut() {
                                    v[vdi] = fr.n;
                                }
                                if let Some(v) = vtx_tu.as_mut() {
                                    v[vdi] = fr.tu;
                                }
                                if let Some(v) = vtx_tv.as_mut() {
                                    v[vdi] = fr.tv;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Commit attributes after immutable borrows are released.
        if let Some(v) = prim_n {
            geo.insert_primitive_attribute(n_name.clone(), Attribute::new_auto(v));
        }
        if let Some(v) = prim_tu {
            geo.insert_primitive_attribute(tu_name.clone(), Attribute::new_auto(v));
        }
        if let Some(v) = prim_tv {
            geo.insert_primitive_attribute(tv_name.clone(), Attribute::new_auto(v));
        }
        if let Some(v) = pt_n {
            geo.insert_point_attribute(n_name.clone(), Attribute::new_auto(v));
        }
        if let Some(v) = pt_tu {
            geo.insert_point_attribute(tu_name.clone(), Attribute::new_auto(v));
        }
        if let Some(v) = pt_tv {
            geo.insert_point_attribute(tv_name.clone(), Attribute::new_auto(v));
        }
        if let Some(v) = vtx_n {
            geo.insert_vertex_attribute(n_name.clone(), Attribute::new_auto(v));
        }
        if let Some(v) = vtx_tu {
            geo.insert_vertex_attribute(tu_name.clone(), Attribute::new_auto(v));
        }
        if let Some(v) = vtx_tv {
            geo.insert_vertex_attribute(tv_name.clone(), Attribute::new_auto(v));
        }

        Arc::new(geo)
    }
}

fn compute_frame_for_primitive(
    geo: &Geometry,
    prim: &GeoPrimitive,
    style: i32,
    attrib_name: &str,
    get_p: &dyn Fn(&Geometry, PointId) -> Option<Vec3>,
    left: bool,
    orth: bool,
) -> Option<Frame> {
    let vids = prim.vertices();
    if vids.len() < 2 {
        return None;
    }
    let p_of = |vid: VertexId| -> Option<Vec3> {
        let v = geo.vertices().get(vid.into())?;
        get_p(geo, v.point_id)
    };
    let p0 = p_of(vids[0])?;
    let p1 = p_of(vids[1])?;
    let mut tu = (p1 - p0);
    let mut n = Vec3::ZERO;
    let mut tv = Vec3::ZERO;

    let tri = first_valid_tri(vids, &p_of)?;
    let (a, b, c) = (tri.p0, tri.p1, tri.p2);
    let n_raw = (b - a).cross(c - a);
    if n_raw.length_squared() <= 1e-20 {
        return None;
    }
    n = n_raw.normalize_or_zero();

    match style {
        x if x == Style::FirstEdge as i32 => {
            tu = tu.normalize_or_zero();
            tv = if left { tu.cross(n) } else { n.cross(tu) };
        }
        x if x == Style::TwoEdges as i32 => {
            if vids.len() >= 3 {
                let plast = p_of(vids[vids.len() - 1])?;
                let e0 = (p1 - p0).normalize_or_zero();
                let e1 = (plast - p0).normalize_or_zero();
                let nn = if left { e1.cross(e0) } else { e0.cross(e1) }.normalize_or_zero();
                if nn.length_squared() <= 1e-20 {
                    return None;
                }
                n = nn;
                tu = e0;
                tv = if left { tu.cross(n) } else { n.cross(tu) };
            } else {
                return None;
            }
        }
        x if x == Style::PrimitiveCentroid as i32 => {
            let mut c = Vec3::ZERO;
            let mut k = 0.0;
            for &vid in vids {
                if let Some(p) = p_of(vid) {
                    c += p;
                    k += 1.0;
                }
            }
            if k <= 0.0 {
                return None;
            }
            c /= k;
            tu = (p0 - c).normalize_or_zero();
            if tu.length_squared() <= 1e-20 {
                return None;
            }
            tv = if left { tu.cross(n) } else { n.cross(tu) };
        }
        x if x == Style::TextureUV as i32 || x == Style::TextureUVGradient as i32 => {
            let (uv0, uv1, uv2) = tri_uv(geo, tri.v0, tri.v1, tri.v2, attrs::UV)?;
            let dp1 = b - a;
            let dp2 = c - a;
            let duv1 = uv1 - uv0;
            let duv2 = uv2 - uv0;
            let denom = duv1.x * duv2.y - duv1.y * duv2.x;
            if denom.abs() <= 1e-20 {
                return None;
            }
            let r = 1.0 / denom;
            tu = (dp1 * duv2.y - dp2 * duv1.y) * r;
            tv = (dp2 * duv1.x - dp1 * duv2.x) * r;
        }
        x if x == Style::AttributeGradient as i32 => {
            if attrib_name.trim().is_empty() {
                return None;
            }
            let g = attr_grad_tri(geo, (tri.v0, tri.v1, tri.v2), attrib_name, (a, b, c), n_raw)?;
            tu = g;
            tv = if left { tu.cross(n) } else { n.cross(tu) };
        }
        x if x == Style::MikkT as i32 => return None, // handled outside
        // Not implemented yet (strict)
        _ => return None,
    }

    if n.length_squared() <= 1e-20 {
        return None;
    }
    if tu.length_squared() <= 1e-20 {
        return None;
    }

    if orth {
        tu = tu.normalize_or_zero();
        tv = if tv.length_squared() <= 1e-20 {
            if left {
                tu.cross(n)
            } else {
                n.cross(tu)
            }
        } else {
            tv.normalize_or_zero()
        };
        tu = (tu - n * tu.dot(n)).normalize_or_zero();
        if tu.length_squared() <= 1e-20 {
            return None;
        }
        tv = if left { tu.cross(n) } else { n.cross(tu) };
        n = if left { tv.cross(tu) } else { tu.cross(tv) }.normalize_or_zero();
        if n.length_squared() <= 1e-20 {
            return None;
        }
    } else {
        tu = tu.normalize_or_zero();
        tv = if tv.length_squared() <= 1e-20 {
            if left {
                tu.cross(n)
            } else {
                n.cross(tu)
            }
        } else {
            tv.normalize_or_zero()
        };
    }

    Some(Frame { n, tu, tv })
}

fn set_prim_v3(geo: &mut Geometry, name: &str, prim_di: usize, v: Vec3) -> bool {
    let Some(attr) = geo.get_primitive_attribute_mut(name) else {
        return false;
    };
    if let Some(s) = attr.as_mut_slice::<Vec3>() {
        if let Some(x) = s.get_mut(prim_di) {
            *x = v;
            return true;
        }
    }
    if let Some(pb) = attr.as_paged_mut::<Vec3>() {
        if let Some(x) = pb.get_mut(prim_di) {
            *x = v;
            return true;
        }
    }
    false
}

fn set_point_v3(geo: &mut Geometry, name: &str, pt_di: usize, v: Vec3) -> bool {
    let Some(attr) = geo.get_point_attribute_mut(name) else {
        return false;
    };
    if let Some(s) = attr.as_mut_slice::<Vec3>() {
        if let Some(x) = s.get_mut(pt_di) {
            *x = v;
            return true;
        }
    }
    if let Some(pb) = attr.as_paged_mut::<Vec3>() {
        if let Some(x) = pb.get_mut(pt_di) {
            *x = v;
            return true;
        }
    }
    false
}

fn set_vert_v3(geo: &mut Geometry, name: &str, v_di: usize, v: Vec3) -> bool {
    let Some(attr) = geo.get_vertex_attribute_mut(name) else {
        return false;
    };
    if let Some(s) = attr.as_mut_slice::<Vec3>() {
        if let Some(x) = s.get_mut(v_di) {
            *x = v;
            return true;
        }
    }
    if let Some(pb) = attr.as_paged_mut::<Vec3>() {
        if let Some(x) = pb.get_mut(v_di) {
            *x = v;
            return true;
        }
    }
    false
}

#[inline]
fn norm_attr_name(name: &str) -> String {
    let n = name.trim();
    if n.is_empty() {
        return attrs::N.to_string();
    }
    if n.starts_with('@') {
        n.to_string()
    } else {
        format!("@{}", n)
    }
}

fn first_valid_tri(vids: &[VertexId], p_of: &dyn Fn(VertexId) -> Option<Vec3>) -> Option<Tri> {
    if vids.len() < 3 {
        return None;
    }
    let v0 = vids[0];
    let p0 = p_of(v0)?;
    for i in 1..vids.len() - 1 {
        let v1 = vids[i];
        let v2 = vids[i + 1];
        let p1 = p_of(v1)?;
        let p2 = p_of(v2)?;
        if (p1 - p0).cross(p2 - p0).length_squared() > 1e-20 {
            return Some(Tri {
                v0,
                v1,
                v2,
                p0,
                p1,
                p2,
            });
        }
    }
    None
}

fn tri_uv(
    geo: &Geometry,
    v0: VertexId,
    v1: VertexId,
    v2: VertexId,
    uv_name: &str,
) -> Option<(Vec2, Vec2, Vec2)> {
    // Prefer vertex uv, then point uv. Strict: must exist.
    if let Some(a) = geo.get_vertex_attribute(uv_name) {
        let uv = a.as_slice::<Vec2>()?;
        let i0 = geo.vertices().get_dense_index(v0.into())?;
        let i1 = geo.vertices().get_dense_index(v1.into())?;
        let i2 = geo.vertices().get_dense_index(v2.into())?;
        return Some((*uv.get(i0)?, *uv.get(i1)?, *uv.get(i2)?));
    }
    let a = geo.get_point_attribute(uv_name)?;
    let uv = a.as_slice::<Vec2>()?;
    let vv0 = geo.vertices().get(v0.into())?;
    let vv1 = geo.vertices().get(v1.into())?;
    let vv2 = geo.vertices().get(v2.into())?;
    let p0 = geo.points().get_dense_index(vv0.point_id.into())?;
    let p1 = geo.points().get_dense_index(vv1.point_id.into())?;
    let p2 = geo.points().get_dense_index(vv2.point_id.into())?;
    Some((*uv.get(p0)?, *uv.get(p1)?, *uv.get(p2)?))
}

fn attr_grad_tri(
    geo: &Geometry,
    tri_vids: (VertexId, VertexId, VertexId),
    name: &str,
    tri: (Vec3, Vec3, Vec3),
    n_raw: Vec3,
) -> Option<Vec3> {
    let (v0, v1, v2) = tri_vids;
    let mut get: Option<Box<dyn Fn(VertexId) -> Option<f32>>> = None;

    if let Some(a) = geo.get_vertex_attribute(name) {
        if let Some(s) = a.as_slice::<f32>() {
            get = Some(Box::new(move |vid| {
                Some(*s.get(geo.vertices().get_dense_index(vid.into())?)?)
            }));
        } else if let Some(s) = a.as_slice::<i32>() {
            get = Some(Box::new(move |vid| {
                Some(*s.get(geo.vertices().get_dense_index(vid.into())?)? as f32)
            }));
        } else if let Some(s) = a.as_slice::<u32>() {
            get = Some(Box::new(move |vid| {
                Some(*s.get(geo.vertices().get_dense_index(vid.into())?)? as f32)
            }));
        } else if let Some(s) = a.as_slice::<Vec2>() {
            get = Some(Box::new(move |vid| {
                Some(s.get(geo.vertices().get_dense_index(vid.into())?)?.length())
            }));
        } else if let Some(s) = a.as_slice::<Vec3>() {
            get = Some(Box::new(move |vid| {
                Some(s.get(geo.vertices().get_dense_index(vid.into())?)?.length())
            }));
        } else if let Some(s) = a.as_slice::<Vec4>() {
            get = Some(Box::new(move |vid| {
                Some(s.get(geo.vertices().get_dense_index(vid.into())?)?.length())
            }));
        } else {
            return None;
        }
    } else if let Some(a) = geo.get_point_attribute(name) {
        if let Some(s) = a.as_slice::<f32>() {
            get = Some(Box::new(move |vid| {
                let v = geo.vertices().get(vid.into())?;
                Some(*s.get(geo.points().get_dense_index(v.point_id.into())?)?)
            }));
        } else if let Some(s) = a.as_slice::<i32>() {
            get = Some(Box::new(move |vid| {
                let v = geo.vertices().get(vid.into())?;
                Some(*s.get(geo.points().get_dense_index(v.point_id.into())?)? as f32)
            }));
        } else if let Some(s) = a.as_slice::<u32>() {
            get = Some(Box::new(move |vid| {
                let v = geo.vertices().get(vid.into())?;
                Some(*s.get(geo.points().get_dense_index(v.point_id.into())?)? as f32)
            }));
        } else if let Some(s) = a.as_slice::<Vec2>() {
            get = Some(Box::new(move |vid| {
                let v = geo.vertices().get(vid.into())?;
                Some(
                    s.get(geo.points().get_dense_index(v.point_id.into())?)?
                        .length(),
                )
            }));
        } else if let Some(s) = a.as_slice::<Vec3>() {
            get = Some(Box::new(move |vid| {
                let v = geo.vertices().get(vid.into())?;
                Some(
                    s.get(geo.points().get_dense_index(v.point_id.into())?)?
                        .length(),
                )
            }));
        } else if let Some(s) = a.as_slice::<Vec4>() {
            get = Some(Box::new(move |vid| {
                let v = geo.vertices().get(vid.into())?;
                Some(
                    s.get(geo.points().get_dense_index(v.point_id.into())?)?
                        .length(),
                )
            }));
        } else {
            return None;
        }
    } else {
        return None;
    }

    let get = get?;
    let (f0, f1, f2) = (get(v0)?, get(v1)?, get(v2)?);
    let (p0, p1, p2) = tri;
    let e1 = p1 - p0;
    let e2 = p2 - p0;
    let denom = n_raw.length_squared();
    if denom <= 1e-20 {
        return None;
    }
    let g = (e2.cross(n_raw) * (f1 - f0) + n_raw.cross(e1) * (f2 - f0)) / denom;
    if g.length_squared() <= 1e-20 {
        None
    } else {
        Some(g.normalize_or_zero())
    }
}

#[derive(Clone)]
struct MikktData {
    frames: Vec<Frame>,
}

fn compute_mikkt(
    geo: &Geometry,
    get_p: &dyn Fn(&Geometry, PointId) -> Option<Vec3>,
    left: bool,
    orth: bool,
) -> Option<MikktData> {
    use bevy::mesh::Indices;
    use bevy::prelude::Mesh;
    use bevy::render::render_resource::PrimitiveTopology;
    let vcnt = geo.vertices().len();
    if vcnt == 0 {
        return None;
    }
    let uv = geo
        .get_vertex_attribute(attrs::UV)
        .and_then(|a| a.as_slice::<Vec2>())
        .or_else(|| {
            geo.get_point_attribute(attrs::UV)
                .and_then(|a| a.as_slice::<Vec2>())
        })?;
    let mut pos: Vec<[f32; 3]> = Vec::with_capacity(vcnt);
    let mut uv0: Vec<[f32; 2]> = Vec::with_capacity(vcnt);
    for (vdi, v) in geo.vertices().values().iter().enumerate() {
        let p = get_p(geo, v.point_id)?;
        pos.push(p.to_array());
        // Vertex uv preferred; if point uv was used, mapping by point dense index.
        if geo.get_vertex_attribute(attrs::UV).is_some() {
            uv0.push(uv.get(vdi).copied()?.to_array());
        } else {
            let pdi = geo.points().get_dense_index(v.point_id.into())?;
            uv0.push(uv.get(pdi).copied()?.to_array());
        }
    }
    // Compute normals (area-weighted) if no vertex N.
    let mut nrm: Vec<[f32; 3]> = vec![[0.0; 3]; vcnt];
    for prim in geo.primitives().iter() {
        let vids = prim.vertices();
        if vids.len() < 3 {
            continue;
        }
        let v0 = geo.vertices().get_dense_index(vids[0].into())? as u32;
        for i in 1..vids.len() - 1 {
            let v1 = geo.vertices().get_dense_index(vids[i].into())? as u32;
            let v2 = geo.vertices().get_dense_index(vids[i + 1].into())? as u32;
            let p0 = Vec3::from_array(pos[v0 as usize]);
            let p1 = Vec3::from_array(pos[v1 as usize]);
            let p2 = Vec3::from_array(pos[v2 as usize]);
            let fnrm = (p1 - p0).cross(p2 - p0);
            for &vi in &[v0, v1, v2] {
                let a = Vec3::from_array(nrm[vi as usize]) + fnrm;
                nrm[vi as usize] = a.to_array();
            }
        }
    }
    for v in &mut nrm {
        let nn = Vec3::from_array(*v).normalize_or_zero();
        *v = nn.to_array();
    }
    // Build indices from polygon fans.
    let mut idx: Vec<u32> = Vec::new();
    for prim in geo.primitives().iter() {
        let vids = prim.vertices();
        if vids.len() < 3 {
            continue;
        }
        let v0 = geo.vertices().get_dense_index(vids[0].into())? as u32;
        for i in 1..vids.len() - 1 {
            let v1 = geo.vertices().get_dense_index(vids[i].into())? as u32;
            let v2 = geo.vertices().get_dense_index(vids[i + 1].into())? as u32;
            idx.extend_from_slice(&[v0, v1, v2]);
        }
    }
    if idx.is_empty() {
        return None;
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv0);
    mesh.insert_indices(Indices::U32(idx));
    #[cfg(feature = "bevy_mikktspace")]
    {
        if mesh.generate_tangents().is_err() {
            return None;
        }
        let tang = mesh.attribute(Mesh::ATTRIBUTE_TANGENT)?.as_float4()?;
        let nrm = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)?.as_float3()?;
        let mut frames: Vec<Frame> = Vec::with_capacity(vcnt);
        for i in 0..vcnt {
            let n = Vec3::from_array([nrm[i][0], nrm[i][1], nrm[i][2]]).normalize_or_zero();
            let t4 = tang[i];
            let tu = Vec3::new(t4[0], t4[1], t4[2]).normalize_or_zero();
            let sign = t4[3];
            let tv = if left {
                tu.cross(n) * sign
            } else {
                n.cross(tu) * sign
            };
            let mut fr = Frame { n, tu, tv };
            if orth {
                fr.tu = (fr.tu - fr.n * fr.tu.dot(fr.n)).normalize_or_zero();
                fr.tv = if left {
                    fr.tu.cross(fr.n)
                } else {
                    fr.n.cross(fr.tu)
                };
                fr.n = if left {
                    fr.tv.cross(fr.tu)
                } else {
                    fr.tu.cross(fr.tv)
                }
                .normalize_or_zero();
            }
            frames.push(fr);
        }
        return Some(MikktData { frames });
    }
    #[cfg(not(feature = "bevy_mikktspace"))]
    {
        let _ = left;
        let _ = orth;
        None
    }
}

fn frame_from_mikkt_prim(
    geo: &Geometry,
    prim: &GeoPrimitive,
    d: &MikktData,
    left: bool,
    orth: bool,
) -> Option<Frame> {
    let vids = prim.vertices();
    if vids.is_empty() {
        return None;
    }
    let mut n = Vec3::ZERO;
    let mut tu = Vec3::ZERO;
    let mut tv = Vec3::ZERO;
    let mut k = 0.0;
    for &vid in vids {
        let di = geo.vertices().get_dense_index(vid.into())?;
        let fr = *d.frames.get(di)?;
        n += fr.n;
        tu += fr.tu;
        tv += fr.tv;
        k += 1.0;
    }
    if k <= 0.0 {
        return None;
    }
    let mut fr = Frame {
        n: (n / k).normalize_or_zero(),
        tu: (tu / k).normalize_or_zero(),
        tv: (tv / k).normalize_or_zero(),
    };
    if orth {
        fr.tu = (fr.tu - fr.n * fr.tu.dot(fr.n)).normalize_or_zero();
        fr.tv = if left {
            fr.tu.cross(fr.n)
        } else {
            fr.n.cross(fr.tu)
        };
        fr.n = if left {
            fr.tv.cross(fr.tu)
        } else {
            fr.tu.cross(fr.tv)
        }
        .normalize_or_zero();
    }
    Some(fr)
}

#[inline]
fn get_param_string(params: &[Parameter], name: &str, default: &str) -> String {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| {
            if let ParameterValue::String(s) = &p.value {
                Some(s.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| default.to_string())
}
#[inline]
fn get_param_int(params: &[Parameter], name: &str, default: i32) -> i32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| {
            if let ParameterValue::Int(v) = p.value {
                Some(v)
            } else {
                None
            }
        })
        .unwrap_or(default)
}
#[inline]
fn get_param_bool(params: &[Parameter], name: &str, default: bool) -> bool {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| {
            if let ParameterValue::Bool(v) = p.value {
                Some(v)
            } else {
                None
            }
        })
        .unwrap_or(default)
}
#[inline]
fn get_param_float(params: &[Parameter], name: &str, default: f32) -> f32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| {
            if let ParameterValue::Float(v) = p.value {
                Some(v)
            } else {
                None
            }
        })
        .unwrap_or(default)
}

register_node!("PolyFrame", "Modeling", PolyFrameNode);
