use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::ids::PointId;
use crate::libs::geometry::mesh::Geometry;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use bevy::prelude::*;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

pub mod builder;
pub mod pipeline_v2;
pub mod spokes;
pub mod structures;

use builder::BevelBuilder;
use pipeline_v2::BevelPipeline;

#[derive(Default)]
pub struct PolyBevelNode;

impl NodeParameters for PolyBevelNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            // Selection
            Parameter::new(
                "group",
                "Group",
                "Selection",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            // Main
            Parameter::new(
                "affect",
                "Affect",
                "Main",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![("Edges".into(), 0), ("Vertices".into(), 1)],
                },
            ),
            Parameter::new(
                "offset_type",
                "Offset Type",
                "Main",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Offset".into(), 0),
                        ("Width".into(), 1),
                        ("Depth".into(), 2),
                        ("Percent".into(), 3),
                        ("Absolute".into(), 4),
                    ],
                },
            ),
            Parameter::new(
                "distance",
                "Amount",
                "Main",
                ParameterValue::Float(0.1),
                ParameterUIType::FloatSlider { min: 0.0, max: 2.0 },
            ),
            Parameter::new(
                "divisions",
                "Segments",
                "Main",
                ParameterValue::Int(1),
                ParameterUIType::IntSlider { min: 1, max: 16 },
            ),
            // Profile (Blender style: 0.0=SquareIn, 0.5=Circle, 1.0=SquareOut)
            Parameter::new(
                "profile_type",
                "Type",
                "Profile",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![("Superellipse".into(), 0), ("Custom".into(), 1)],
                },
            ),
            Parameter::new(
                "profile",
                "Shape",
                "Profile",
                ParameterValue::Float(0.5),
                ParameterUIType::FloatSlider { min: 0.0, max: 1.0 },
            ),
            Parameter::new(
                "custom_profile",
                "Custom Profile",
                "Profile",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            // Geometry
            Parameter::new(
                "clamp_overlap",
                "Clamp Overlap",
                "Geometry",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "loop_slide",
                "Loop Slide",
                "Geometry",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            // Miter
            Parameter::new(
                "miter_outer",
                "Outer",
                "Miter",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![("Sharp".into(), 0), ("Patch".into(), 1), ("Arc".into(), 2)],
                },
            ),
            Parameter::new(
                "miter_inner",
                "Inner",
                "Miter",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![("Sharp".into(), 0), ("Arc".into(), 1)],
                },
            ),
            Parameter::new(
                "spread",
                "Spread",
                "Miter",
                ParameterValue::Float(0.1),
                ParameterUIType::FloatSlider { min: 0.0, max: 1.0 },
            ),
            // Intersection
            Parameter::new(
                "vmesh_method",
                "Intersection",
                "Intersection",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![("Grid Fill".into(), 0), ("Cutoff".into(), 1)],
                },
            ),
            // Weights
            Parameter::new(
                "use_weights",
                "Use Weights",
                "Weights",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "vertex_group",
                "Vertex Group",
                "Weights",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "bweight_offset_vert",
                "Offset By Vertex Weight",
                "Weights",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "bweight_offset_edge",
                "Offset By Edge Weight",
                "Weights",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            // Shading
            Parameter::new(
                "harden_normals",
                "Harden Normals",
                "Shading",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "mark_seam",
                "Mark Seam",
                "Shading",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "mark_sharp",
                "Mark Sharp",
                "Shading",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "face_strength",
                "Face Strength",
                "Shading",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("None".into(), 0),
                        ("New".into(), 1),
                        ("Affected".into(), 2),
                        ("All".into(), 3),
                    ],
                },
            ),
            Parameter::new(
                "material",
                "Material",
                "Shading",
                ParameterValue::Int(-1),
                ParameterUIType::IntSlider { min: -1, max: 16 },
            ),
            // Debug
            Parameter::new(
                "debug_flip_input",
                "Flip Input",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "debug_invert_profile",
                "Invert Profile",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "debug_corner_scale",
                "Corner Scale",
                "Debug",
                ParameterValue::Float(1.0),
                ParameterUIType::FloatSlider { min: 0.0, max: 2.0 },
            ),
            Parameter::new(
                "debug_spoke_order",
                "Spoke Order",
                "Debug",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Default".into(), 0),
                        ("Reverse".into(), 1),
                        ("Rotate +1".into(), 2),
                        ("Rotate -1".into(), 3),
                    ],
                },
            ),
            Parameter::new(
                "debug_swap_left_right",
                "Swap Left/Right",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "debug_invert_face_normals",
                "Invert Face Normals",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "debug_flip_output_winding",
                "Flip Output Winding",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "debug_disable_strip_orient",
                "Disable Strip Orient Fix",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "debug_disable_face_rebuild_orient",
                "Disable Face Rebuild Orient Fix",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "debug_invert_edge_dirs",
                "Invert Edge Dirs",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "debug_invert_edge_ends",
                "Invert Edge Ends",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "debug_swap_offsets_lr",
                "Swap Offsets L/R",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "debug_swap_face_pair_normals",
                "Swap Face/Pair Normals",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "debug_invert_pair_face_normals",
                "Invert Pair Face Normals",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "debug_invert_arc_for",
                "Invert Arc Direction",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "debug_disable_adjust_offsets",
                "Disable Adjust Offsets",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "debug_disable_offset_limit",
                "Disable Offset Limit",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "debug_disable_square_in_vmesh",
                "Disable SquareIn VMesh",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "debug_disable_square_out_adj_vmesh",
                "Disable SquareOut Adj VMesh",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for PolyBevelNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let Some(input) = mats.first() else {
            return Arc::new(Geometry::new());
        };

        let group = params
            .iter()
            .find(|p| p.name == "group")
            .and_then(|p| match &p.value {
                ParameterValue::String(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("");
        let mut distance = get_param_float(params, "distance", 0.1);
        if distance.abs() < 1e-5 {
            return input.clone();
        }

        let divisions = get_param_int(params, "divisions", 1).max(1) as usize;
        let affect = get_param_int(params, "affect", 0);
        let offset_type = get_param_int(params, "offset_type", 0);
        let profile_type = get_param_int(params, "profile_type", 0).clamp(0, 1); // 0=Superellipse, 1=Custom
        let profile_amount = get_param_float(params, "profile", 0.5); // 0.0=SquareIn, 0.5=Circle, 1.0=SquareOut
        let custom_profile = params
            .iter()
            .find(|p| p.name == "custom_profile")
            .and_then(|p| match &p.value {
                ParameterValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let miter_outer = get_param_int(params, "miter_outer", 0);
        let miter_inner = get_param_int(params, "miter_inner", 0);
        let spread = get_param_float(params, "spread", 0.1);
        let clamp_overlap = get_param_bool(params, "clamp_overlap", true);
        let loop_slide = get_param_bool(params, "loop_slide", true);
        let use_weights = get_param_bool(params, "use_weights", false);
        let vertex_group = params
            .iter()
            .find(|p| p.name == "vertex_group")
            .and_then(|p| match &p.value {
                ParameterValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let bweight_offset_vert = get_param_bool(params, "bweight_offset_vert", false);
        let bweight_offset_edge = get_param_bool(params, "bweight_offset_edge", false);
        let harden_normals = get_param_bool(params, "harden_normals", false);
        let mark_seam = get_param_bool(params, "mark_seam", false);
        let mark_sharp = get_param_bool(params, "mark_sharp", false);
        let vmesh_method = get_param_int(params, "vmesh_method", 0);
        let face_strength = get_param_int(params, "face_strength", 0);
        let material = get_param_int(params, "material", -1);
        let debug_flip_input = get_param_bool(params, "debug_flip_input", false);
        let invert_profile = get_param_bool(params, "debug_invert_profile", false);
        let corner_scale = get_param_float(params, "debug_corner_scale", 1.0);
        let debug_spoke_order = get_param_int(params, "debug_spoke_order", 0);
        let debug_swap_left_right = get_param_bool(params, "debug_swap_left_right", false);
        let debug_invert_face_normals = get_param_bool(params, "debug_invert_face_normals", false);
        let debug_flip_output_winding = get_param_bool(params, "debug_flip_output_winding", false);
        let debug_disable_strip_orient = get_param_bool(params, "debug_disable_strip_orient", false);
        let debug_disable_face_rebuild_orient =
            get_param_bool(params, "debug_disable_face_rebuild_orient", false);
        let debug_invert_edge_dirs = get_param_bool(params, "debug_invert_edge_dirs", false);
        let debug_invert_edge_ends = get_param_bool(params, "debug_invert_edge_ends", false);
        let debug_swap_offsets_lr = get_param_bool(params, "debug_swap_offsets_lr", false);
        let debug_swap_face_pair_normals =
            get_param_bool(params, "debug_swap_face_pair_normals", false);
        let debug_invert_pair_face_normals =
            get_param_bool(params, "debug_invert_pair_face_normals", false);
        let debug_invert_arc_for = get_param_bool(params, "debug_invert_arc_for", false);
        let debug_disable_adjust_offsets =
            get_param_bool(params, "debug_disable_adjust_offsets", false);
        let debug_disable_offset_limit = get_param_bool(params, "debug_disable_offset_limit", false);
        let debug_disable_square_in_vmesh =
            get_param_bool(params, "debug_disable_square_in_vmesh", false);
        let debug_disable_square_out_adj_vmesh =
            get_param_bool(params, "debug_disable_square_out_adj_vmesh", false);

        let src_owned;
        let src: &Geometry = if debug_flip_input {
            src_owned = flip_input_winding(input.as_ref());
            &src_owned
        } else {
            input.as_ref()
        };

        let topo_arc = src.get_topology();
        let topo = topo_arc.as_ref();

        // Selection: by group (preferred) else default all manifold edges (pair exists).
        let mut edge_selection = vec![false; topo.half_edges.len()];
        if !group.trim().is_empty() {
            let gname = group.trim();
            let prim_group = src.get_primitive_group(gname);
            if let Some(pg) = prim_group {
                for (he_idx, he) in topo.half_edges.iter_enumerated() {
                    let he_id = crate::libs::geometry::ids::HalfEdgeId::from(he_idx);
                    let pair = topo.pair(he_id);
                    if !pair.is_valid() {
                        continue;
                    }
                    let prim_di = src
                        .primitives()
                        .get_dense_index(he.primitive_index.into())
                        .unwrap_or(usize::MAX);
                    let sel_a = prim_di != usize::MAX && pg.get(prim_di);
                    let sel_b = if pair.is_valid() {
                        topo.half_edges
                            .get(pair.into())
                            .and_then(|p| {
                                src.primitives().get_dense_index(p.primitive_index.into())
                            })
                            .map(|pdi| pg.get(pdi))
                            .unwrap_or(false)
                    } else {
                        false
                    };
                    if !(sel_a || sel_b) {
                        continue;
                    }
                    if let Some(di) = topo.half_edges.get_dense_index(he_idx) {
                        if di < edge_selection.len() {
                            edge_selection[di] = true;
                        }
                    }
                    if let Some(pdi) = topo.half_edges.get_dense_index(pair.into()) {
                        if pdi < edge_selection.len() {
                            edge_selection[pdi] = true;
                        }
                    }
                }
            }
        }
        if edge_selection.iter().all(|v| !*v) {
            for (he_idx, _) in topo.half_edges.iter_enumerated() {
                let he_id = crate::libs::geometry::ids::HalfEdgeId::from(he_idx);
                if topo.pair(he_id).is_valid() {
                    if let Some(di) = topo.half_edges.get_dense_index(he_idx) {
                        if di < edge_selection.len() {
                            edge_selection[di] = true;
                        }
                    }
                }
            }
        }

        // OffsetType: Percent 用所选边平均长度转换为绝对距离（先保证 UI 变化可见）
        if offset_type == 3 {
            let positions = src.get_point_position_attribute().unwrap_or(&[]);
            let mut sum = 0.0f32;
            let mut cnt = 0usize;
            for (he_idx, he) in topo.half_edges.iter_enumerated() {
                if !topo
                    .pair(crate::libs::geometry::ids::HalfEdgeId::from(he_idx))
                    .is_valid()
                {
                    continue;
                }
                if topo
                    .half_edges
                    .get_dense_index(he_idx)
                    .and_then(|di| edge_selection.get(di).copied())
                    .unwrap_or(false)
                {
                    let u = src
                        .points()
                        .get_dense_index(he.origin_point.into())
                        .and_then(|di| positions.get(di))
                        .copied()
                        .unwrap_or(Vec3::ZERO);
                    let vpid =
                        topo.dest_point(crate::libs::geometry::ids::HalfEdgeId::from(he_idx));
                    let v = src
                        .points()
                        .get_dense_index(vpid.into())
                        .and_then(|di| positions.get(di))
                        .copied()
                        .unwrap_or(Vec3::ZERO);
                    let len = (v - u).length();
                    if len > 1e-8 {
                        sum += len;
                        cnt += 1;
                    }
                }
            }
            if cnt > 0 {
                let avg = sum / cnt as f32;
                distance = (distance.clamp(0.0, 1.0)) * avg;
            }
        }

        // OffsetType: Width/Depth -> convert to bevel offset using average dihedral angle of selected edges.
        if offset_type == 1 || offset_type == 2 {
            let positions = src.get_point_position_attribute().unwrap_or(&[]);
            let prim_normal = |prim_id: crate::libs::geometry::ids::PrimId| -> Vec3 {
                let Some(p) = src.primitives().get(prim_id.into()) else {
                    return Vec3::Y;
                };
                let crate::libs::geometry::mesh::GeoPrimitive::Polygon(poly) = p else {
                    return Vec3::Y;
                };
                if poly.vertices.len() < 3 {
                    return Vec3::Y;
                }
                let mut n = Vec3::ZERO;
                let mut pts: Vec<Vec3> = Vec::with_capacity(poly.vertices.len());
                for &vid in &poly.vertices {
                    let pid = src
                        .vertices()
                        .get(vid.into())
                        .map(|v| v.point_id)
                        .unwrap_or(crate::libs::geometry::ids::PointId::INVALID);
                    let p = src
                        .points()
                        .get_dense_index(pid.into())
                        .and_then(|pi| positions.get(pi).copied())
                        .unwrap_or(Vec3::ZERO);
                    pts.push(p);
                }
                for i in 0..pts.len() {
                    n += (pts[(i + 1) % pts.len()] - pts[i])
                        .cross(pts[(i + 2) % pts.len()] - pts[i]);
                }
                let nn = n.normalize_or_zero();
                if nn.length_squared() > 1e-12 {
                    nn
                } else {
                    Vec3::Y
                }
            };
            let mut seen: HashSet<(PointId, PointId)> = HashSet::new();
            let mut sum = 0.0f32;
            let mut cnt = 0usize;
            for (he_idx, he) in topo.half_edges.iter_enumerated() {
                let he_id = crate::libs::geometry::ids::HalfEdgeId::from(he_idx);
                let pair = topo.pair(he_id);
                if !pair.is_valid() {
                    continue;
                }
                if !topo
                    .half_edges
                    .get_dense_index(he_idx)
                    .and_then(|di| edge_selection.get(di).copied())
                    .unwrap_or(false)
                {
                    continue;
                }
                let u = he.origin_point;
                let v = topo.dest_point(he_id);
                if !u.is_valid() || !v.is_valid() {
                    continue;
                }
                let key = if u < v { (u, v) } else { (v, u) };
                if !seen.insert(key) {
                    continue;
                }
                let n1 = prim_normal(he.primitive_index);
                let n2 = topo
                    .half_edges
                    .get(pair.into())
                    .map(|p| prim_normal(p.primitive_index))
                    .unwrap_or(Vec3::Y);
                let dot = n1
                    .normalize_or_zero()
                    .dot(n2.normalize_or_zero())
                    .abs()
                    .clamp(-1.0, 1.0);
                let theta = dot.acos();
                if theta > 1e-5 {
                    sum += theta;
                    cnt += 1;
                }
            }
            if cnt > 0 {
                let th = (sum / cnt as f32) * 0.5;
                if offset_type == 2 {
                    // Depth: H = d / sin(th) => d = H * sin(th)
                    distance *= th.sin().max(1e-6);
                } else {
                    // Width: W = 2d / tan(th) => d = W * tan(th) / 2
                    distance *= th.tan() * 0.5;
                }
            }
        }

        let point_weights = if use_weights {
            if !vertex_group.trim().is_empty() {
                src.get_point_attribute(vertex_group.trim())
                    .and_then(|a| a.as_slice::<f32>())
            } else if bweight_offset_vert {
                [
                    "bevel_weight_vert",
                    "bevel_weight",
                    "bweight",
                    "__cunning.bevel_weight_vert",
                ]
                .into_iter()
                .find_map(|n| src.get_point_attribute(n).and_then(|a| a.as_slice::<f32>()))
            } else {
                None
            }
        } else {
            None
        };

        let edge_weights: Option<HashMap<(PointId, PointId), f32>> =
            if use_weights && bweight_offset_edge {
                let mut g = src.clone();
                let ec = crate::libs::geometry::edge_cache::EdgeCache::build(&g);
                for (p0, p1) in ec.edges.iter().copied() {
                    let _ = g.add_edge(p0, p1);
                }
                let ws = [
                    "bevel_weight_edge",
                    "bevel_weight",
                    "bweight",
                    "__cunning.bevel_weight_edge",
                ]
                .into_iter()
                .find_map(|n| g.get_edge_attribute(n).and_then(|a| a.as_slice::<f32>()));
                ws.map(|ws| {
                    let mut m = HashMap::with_capacity(g.edges().len());
                    for (dense_idx, (_i, e)) in g.edges().iter_enumerated().enumerate() {
                        let w = ws
                            .get(dense_idx)
                            .copied()
                            .unwrap_or(0.0f32)
                            .clamp(0.0f32, 1.0f32);
                        let key = if e.p0 < e.p1 {
                            (e.p0, e.p1)
                        } else {
                            (e.p1, e.p0)
                        };
                        m.insert(key, w);
                    }
                    m
                })
            } else {
                None
            };
        let builder = BevelBuilder::new(src, topo);
        let graph = builder.build(
            &edge_selection,
            divisions,
            distance,
            point_weights,
            edge_weights.as_ref(),
        );

        let bp = structures::BevelParams::from_node_params(
            affect,
            offset_type,
            distance,
            divisions,
            profile_type,
            profile_amount,
            custom_profile,
            miter_outer,
            miter_inner,
            spread,
            clamp_overlap,
            loop_slide,
            use_weights,
            vertex_group,
            bweight_offset_vert,
            bweight_offset_edge,
            harden_normals,
            mark_seam,
            mark_sharp,
            vmesh_method,
            face_strength,
            material,
            invert_profile,
            corner_scale,
            debug_spoke_order,
            debug_swap_left_right,
            debug_invert_face_normals,
            debug_flip_output_winding,
            debug_disable_strip_orient,
            debug_disable_face_rebuild_orient,
            debug_invert_edge_dirs,
            debug_invert_edge_ends,
            debug_swap_offsets_lr,
            debug_swap_face_pair_normals,
            debug_invert_pair_face_normals,
            debug_invert_arc_for,
            debug_disable_adjust_offsets,
            debug_disable_offset_limit,
            debug_disable_square_in_vmesh,
            debug_disable_square_out_adj_vmesh,
        );
        let pipeline = BevelPipeline::new_with_params(src, topo, graph, bp);
        Arc::new(pipeline.execute())
    }
}

register_node!("PolyBevel", "Modeling", PolyBevelNode);

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

fn flip_input_winding(src: &Geometry) -> Geometry {
    let mut g = src.clone();
    let ids: Vec<_> = g
        .primitives()
        .iter_enumerated()
        .map(|(i, _)| crate::libs::geometry::ids::PrimId::from(i))
        .collect();
    for pid in ids {
        if let Some(p) = g.primitives_mut().get_mut(pid.into()) {
            if let crate::libs::geometry::mesh::GeoPrimitive::Polygon(poly) = p {
                poly.vertices.reverse();
            }
        }
    }
    g
}
