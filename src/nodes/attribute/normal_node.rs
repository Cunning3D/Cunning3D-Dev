//! Normal recalculation node (Houdini-compatible). Runtime DLL level.
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::{
    cunning_core::traits::node_interface::{NodeOp, NodeParameters},
    libs::geometry::attrs,
    libs::geometry::ids::VertexId,
    mesh::{Attribute, GeoPrimitive, Geometry},
    nodes::parameter::{Parameter, ParameterUIType, ParameterValue},
};
use bevy::prelude::Vec3;
use std::{collections::HashMap, f32::consts::PI, sync::Arc};

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum NormalTarget {
    Points = 0,
    Vertices = 1,
    Primitives = 2,
    Detail = 3,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum WeightingMethod {
    EachVertexEqually = 0,
    ByVertexAngle = 1,
    ByFaceArea = 2,
}

#[derive(Default)]
pub struct NormalNode;

impl NodeParameters for NormalNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "compute_normals",
                "Compute Normals",
                "Construct",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "add_normals_to",
                "Add Normals to",
                "Construct",
                ParameterValue::Int(NormalTarget::Vertices as i32),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Points".into(), 0),
                        ("Vertices".into(), 1),
                        ("Primitives".into(), 2),
                        ("Detail".into(), 3),
                    ],
                },
            ),
            Parameter::new(
                "cusp_angle",
                "Cusp Angle",
                "Construct",
                ParameterValue::Float(60.0),
                ParameterUIType::FloatSlider {
                    min: 0.0,
                    max: 180.0,
                },
            ),
            Parameter::new(
                "weighting_method",
                "Weighting Method",
                "Construct",
                ParameterValue::Int(WeightingMethod::ByVertexAngle as i32),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Each Vertex Equally".into(), 0),
                        ("By Vertex Angle".into(), 1),
                        ("By Face Area".into(), 2),
                    ],
                },
            ),
            Parameter::new(
                "keep_original_zero",
                "Keep Original Normal Where Computed Normal Is Zero",
                "Construct",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "make_unit_length",
                "Make Normals Unit Length",
                "Modify",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "reverse_normals",
                "Reverse Normals",
                "Modify",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for NormalNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let input = inputs
            .first()
            .map(|g| Arc::new(g.materialize()))
            .unwrap_or_else(|| Arc::new(Geometry::new()));
        let mut geo = (*input).clone();
        let get_bool = |n: &str| {
            params
                .iter()
                .find(|p| p.name == n)
                .and_then(|p| {
                    if let ParameterValue::Bool(v) = &p.value {
                        Some(*v)
                    } else {
                        None
                    }
                })
                .unwrap_or(false)
        };
        let get_int = |n: &str| {
            params
                .iter()
                .find(|p| p.name == n)
                .and_then(|p| {
                    if let ParameterValue::Int(v) = &p.value {
                        Some(*v)
                    } else {
                        None
                    }
                })
                .unwrap_or(0)
        };
        let get_float = |n: &str| {
            params
                .iter()
                .find(|p| p.name == n)
                .and_then(|p| {
                    if let ParameterValue::Float(v) = &p.value {
                        Some(*v)
                    } else {
                        None
                    }
                })
                .unwrap_or(0.0)
        };

        let compute_normals = get_bool("compute_normals");
        let target = get_int("add_normals_to");
        let cusp_angle = get_float("cusp_angle");
        let weighting = get_int("weighting_method");
        let keep_original_zero = get_bool("keep_original_zero");
        let make_unit = get_bool("make_unit_length");
        let reverse = get_bool("reverse_normals");

        if compute_normals {
            compute_normals_impl(&mut geo, target, cusp_angle, weighting, keep_original_zero);
        }
        if make_unit {
            normalize_normals(&mut geo, target);
        }
        if reverse {
            reverse_normals_impl(&mut geo, target);
        }

        Arc::new(geo)
    }
}

fn compute_normals_impl(
    geo: &mut Geometry,
    target: i32,
    cusp_angle: f32,
    weighting: i32,
    keep_zero: bool,
) {
    let positions = match geo
        .get_point_attribute(attrs::P)
        .and_then(|a| a.as_slice::<Vec3>())
    {
        Some(p) => p.to_vec(),
        None => return,
    };
    let cusp_rad = cusp_angle * PI / 180.0;
    let cusp_cos = cusp_rad.cos();

    // Step 1: Compute face normals and areas
    let mut face_normals: Vec<Vec3> = Vec::with_capacity(geo.primitives().len());
    let mut face_areas: Vec<f32> = Vec::with_capacity(geo.primitives().len());

    for prim in geo.primitives().iter() {
        let (n, a) = compute_face_normal_and_area(geo, prim, &positions);
        face_normals.push(n);
        face_areas.push(a);
    }

    // Step 2: Build point -> vertex -> prim mapping
    let mut point_to_prims: HashMap<usize, Vec<(usize, usize, VertexId)>> = HashMap::new(); // point_idx -> [(prim_idx, vert_order_in_prim, vid)]
    for (prim_idx, prim) in geo.primitives().iter().enumerate() {
        for (order, &vid) in prim.vertices().iter().enumerate() {
            if let Some(v) = geo.vertices().get(vid.into()) {
                if let Some(pidx) = geo.points().get_dense_index(v.point_id.into()) {
                    point_to_prims
                        .entry(pidx)
                        .or_default()
                        .push((prim_idx, order, vid));
                }
            }
        }
    }

    // Step 3: Compute normals per vertex (cusp-aware smooth)
    let vert_count = geo.vertices().len();
    let mut vertex_normals = vec![Vec3::ZERO; vert_count];

    for (prim_idx, prim) in geo.primitives().iter().enumerate() {
        let verts = prim.vertices();
        let n_verts = verts.len();
        if n_verts < 3 {
            continue;
        }
        let fn_this = face_normals[prim_idx];

        for (order, &vid) in verts.iter().enumerate() {
            let Some(v_dense) = geo.vertices().get_dense_index(vid.into()) else {
                continue;
            };
            let Some(v) = geo.vertices().get(vid.into()) else {
                continue;
            };
            let Some(pidx) = geo.points().get_dense_index(v.point_id.into()) else {
                continue;
            };

            // Compute vertex angle (for By Vertex Angle weighting)
            let vert_angle = compute_vertex_angle(geo, prim, order, &positions);

            // Gather contributing faces (cusp test)
            let mut accum = Vec3::ZERO;
            if let Some(neighbors) = point_to_prims.get(&pidx) {
                for &(np_idx, _, _) in neighbors.iter() {
                    let fn_neigh = face_normals[np_idx];
                    if fn_this.dot(fn_neigh) >= cusp_cos {
                        // Within cusp angle
                        let w = match weighting {
                            0 => 1.0,                // Each Vertex Equally
                            1 => vert_angle,         // By Vertex Angle
                            2 => face_areas[np_idx], // By Face Area
                            _ => 1.0,
                        };
                        accum += fn_neigh * w;
                    }
                }
            }
            vertex_normals[v_dense] = accum;
        }
    }

    // Handle keep_zero: preserve original if computed is zero
    if keep_zero {
        if let Some(orig) = geo
            .get_vertex_attribute(attrs::N)
            .and_then(|a| a.as_slice::<Vec3>())
        {
            for (i, n) in vertex_normals.iter_mut().enumerate() {
                if n.length_squared() < 1e-12 {
                    if let Some(o) = orig.get(i) {
                        *n = *o;
                    }
                }
            }
        }
    }

    // Normalize
    for n in &mut vertex_normals {
        *n = n.normalize_or_zero();
    }

    // Step 4: Output based on target
    match target {
        0 => {
            // Points
            let mut point_normals = vec![Vec3::ZERO; geo.points().len()];
            let mut point_counts = vec![0u32; geo.points().len()];
            for (v_idx, (vid_raw, _)) in geo.vertices().iter_enumerated().enumerate() {
                let vid = VertexId::from(vid_raw);
                if let Some(v) = geo.vertices().get(vid.into()) {
                    if let Some(pidx) = geo.points().get_dense_index(v.point_id.into()) {
                        point_normals[pidx] += vertex_normals[v_idx];
                        point_counts[pidx] += 1;
                    }
                }
            }
            for (i, n) in point_normals.iter_mut().enumerate() {
                if point_counts[i] > 0 {
                    *n = (*n / point_counts[i] as f32).normalize_or_zero();
                }
            }
            geo.insert_point_attribute(attrs::N, Attribute::new(point_normals));
        }
        1 => {
            // Vertices
            geo.insert_vertex_attribute(attrs::N, Attribute::new(vertex_normals));
        }
        2 => {
            // Primitives
            geo.insert_primitive_attribute(attrs::N, Attribute::new(face_normals));
        }
        3 => {
            // Detail (average of all)
            let avg = if !vertex_normals.is_empty() {
                vertex_normals.iter().fold(Vec3::ZERO, |a, &n| a + n) / vertex_normals.len() as f32
            } else {
                Vec3::Y
            };
            geo.insert_detail_attribute(attrs::N, Attribute::new(vec![avg.normalize_or_zero()]));
        }
        _ => {}
    }
}

fn compute_face_normal_and_area(
    geo: &Geometry,
    prim: &GeoPrimitive,
    positions: &[Vec3],
) -> (Vec3, f32) {
    let verts = prim.vertices();
    if verts.len() < 3 {
        return (Vec3::Y, 0.0);
    }

    let get_pos = |vid: VertexId| -> Vec3 {
        geo.vertices()
            .get(vid.into())
            .and_then(|v| geo.points().get_dense_index(v.point_id.into()))
            .and_then(|pidx| positions.get(pidx).copied())
            .unwrap_or(Vec3::ZERO)
    };

    let p0 = get_pos(verts[0]);
    let mut normal = Vec3::ZERO;
    let mut area = 0.0;

    for i in 1..verts.len() - 1 {
        let p1 = get_pos(verts[i]);
        let p2 = get_pos(verts[i + 1]);
        let cross = (p1 - p0).cross(p2 - p0);
        normal += cross;
        area += cross.length() * 0.5;
    }

    (normal.normalize_or_zero(), area)
}

fn compute_vertex_angle(
    geo: &Geometry,
    prim: &GeoPrimitive,
    order: usize,
    positions: &[Vec3],
) -> f32 {
    let verts = prim.vertices();
    let n = verts.len();
    if n < 3 {
        return 1.0;
    }

    let get_pos = |vid: VertexId| -> Vec3 {
        geo.vertices()
            .get(vid.into())
            .and_then(|v| geo.points().get_dense_index(v.point_id.into()))
            .and_then(|pidx| positions.get(pidx).copied())
            .unwrap_or(Vec3::ZERO)
    };

    let p_curr = get_pos(verts[order]);
    let p_prev = get_pos(verts[(order + n - 1) % n]);
    let p_next = get_pos(verts[(order + 1) % n]);

    let v1 = (p_prev - p_curr).normalize_or_zero();
    let v2 = (p_next - p_curr).normalize_or_zero();
    v1.dot(v2).clamp(-1.0, 1.0).acos()
}

fn normalize_normals(geo: &mut Geometry, target: i32) {
    let normalize_vec = |attr: Option<&mut Attribute>| {
        if let Some(a) = attr {
            if let Some(slice) = a.as_mut_slice::<Vec3>() {
                for n in slice.iter_mut() {
                    *n = n.normalize_or_zero();
                }
            }
        }
    };
    match target {
        0 => normalize_vec(geo.get_point_attribute_mut(attrs::N)),
        1 => normalize_vec(geo.get_vertex_attribute_mut(attrs::N)),
        2 => normalize_vec(geo.get_primitive_attribute_mut(attrs::N)),
        3 => normalize_vec(geo.get_detail_attribute_mut(attrs::N)),
        _ => {}
    }
}

fn reverse_normals_impl(geo: &mut Geometry, target: i32) {
    let reverse_vec = |attr: Option<&mut Attribute>| {
        if let Some(a) = attr {
            if let Some(slice) = a.as_mut_slice::<Vec3>() {
                for n in slice.iter_mut() {
                    *n = -*n;
                }
            }
        }
    };
    match target {
        0 => reverse_vec(geo.get_point_attribute_mut(attrs::N)),
        1 => reverse_vec(geo.get_vertex_attribute_mut(attrs::N)),
        2 => reverse_vec(geo.get_primitive_attribute_mut(attrs::N)),
        3 => reverse_vec(geo.get_detail_attribute_mut(attrs::N)),
        _ => {}
    }
}

crate::register_node!(
    "Normal",
    "Attribute",
    crate::nodes::attribute::normal_node::NormalNode
);
