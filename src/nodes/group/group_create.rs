use bevy::prelude::*;
use std::sync::Arc;

use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::{
    cunning_core::traits::node_interface::{NodeOp, NodeParameters},
    libs::geometry::group::ElementGroupMask,
    mesh::Geometry,
    nodes::parameter::{Parameter, ParameterUIType, ParameterValue},
    register_node,
};

use crate::libs::algorithms::algorithms_runtime::group_core::{self as gc, select};

#[derive(Default)]
pub struct GroupCreateNode;

impl NodeParameters for GroupCreateNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "group_name",
                "Group Name",
                "General",
                ParameterValue::String("group1".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "group_type",
                "Group Type",
                "General",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Points".into(), 0),
                        ("Primitives".into(), 1),
                        ("Vertices".into(), 2),
                        ("Edges".into(), 3),
                    ],
                },
            ),
            Parameter::new(
                "merge_op",
                "Initial Merge",
                "General",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Replace".into(), 0),
                        ("Union".into(), 1),
                        ("Intersect".into(), 2),
                        ("Subtract".into(), 3),
                    ],
                },
            ),
            // Base Group
            Parameter::new(
                "base_enable",
                "Base Group",
                "Base",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "pattern",
                "Pattern",
                "Base",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            // Bounding
            Parameter::new(
                "bound_enable",
                "Keep in Bounding Regions",
                "Bounding",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "bound_type",
                "Bounding Type",
                "Bounding",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Box".into(), 0),
                        ("Sphere".into(), 1),
                        ("Input Box".into(), 2),
                        ("Input Volume".into(), 3),
                    ],
                },
            ),
            Parameter::new(
                "bound_center",
                "Center",
                "Bounding",
                ParameterValue::Vec3(Vec3::ZERO),
                ParameterUIType::Vec3Drag,
            ),
            Parameter::new(
                "bound_size",
                "Size",
                "Bounding",
                ParameterValue::Vec3(Vec3::ONE),
                ParameterUIType::Vec3Drag,
            ),
            Parameter::new(
                "bound_radius",
                "Radius",
                "Bounding",
                ParameterValue::Float(1.0),
                ParameterUIType::FloatSlider {
                    min: 0.0,
                    max: 10.0,
                },
            ),
            Parameter::new(
                "bound_iso",
                "Isovalue",
                "Bounding",
                ParameterValue::Float(0.0),
                ParameterUIType::FloatSlider {
                    min: -1.0,
                    max: 1.0,
                },
            ),
            // Normals
            Parameter::new(
                "normal_enable",
                "Keep by Normals",
                "Normals",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "normal_dir",
                "Direction",
                "Normals",
                ParameterValue::Vec3(Vec3::Y),
                ParameterUIType::Vec3Drag,
            ),
            Parameter::new(
                "normal_angle",
                "Spread Angle",
                "Normals",
                ParameterValue::Float(180.0),
                ParameterUIType::FloatSlider {
                    min: 0.0,
                    max: 180.0,
                },
            ),
            // Edges
            Parameter::new(
                "edge_enable",
                "Include by Edges (Boundary)",
                "Edges",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for GroupCreateNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let default_input = Arc::new(Geometry::new());
        let input_geo = mats.first().unwrap_or(&default_input);

        // 1. Fork Input (Copy-on-Write with new ID)
        let mut out_geo = input_geo.fork();
        let point_count = out_geo.get_point_count();
        let primitive_count = out_geo.primitives().len();
        let vertex_count = out_geo.vertices().len();

        // 2. Parse Parameters
        let group_name = get_param_string(params, "group_name", "group1");
        let group_type = get_param_int(params, "group_type", 0); // 0: Points, 1: Primitives, 2: Vertices, 3: Edges
        let merge_op = get_param_int(params, "merge_op", 0); // 0: Replace, 1: Union, 2: Intersect, 3: Subtract

        let base_enable = get_param_bool(params, "base_enable", false);
        let pattern = get_param_string(params, "pattern", "");

        let bound_enable = get_param_bool(params, "bound_enable", false);
        let bound_type = get_param_int(params, "bound_type", 0); // 0: Box, 1: Sphere, 2: Input Box, 3: Input Volume
        let bound_center = get_param_vec3(params, "bound_center", Vec3::ZERO);
        let bound_size = get_param_vec3(params, "bound_size", Vec3::ONE);
        let bound_radius = get_param_float(params, "bound_radius", 1.0);
        let bound_iso = get_param_float(params, "bound_iso", 0.0);

        let normal_enable = get_param_bool(params, "normal_enable", false);
        let normal_dir = get_param_vec3(params, "normal_dir", Vec3::Y);
        let normal_angle = get_param_float(params, "normal_angle", 180.0); // Default 180 means all directions

        let edge_enable = get_param_bool(params, "edge_enable", false);

        if group_name.is_empty() {
            return Arc::new(out_geo);
        }

        if group_type == 3 && out_geo.edges().is_empty() {
            let pcount = out_geo.points().len();
            let _ = gc::promote_mask(
                &mut out_geo,
                gc::GroupDomain::Point,
                gc::GroupDomain::Edge,
                &ElementGroupMask::new(pcount),
                gc::PromoteMode::All,
            );
        }
        let edge_count = out_geo.edges().len();
        let count = match group_type {
            0 => point_count,
            1 => primitive_count,
            2 => vertex_count,
            3 => edge_count,
            _ => point_count,
        };

        // 3. Initialize Candidate Selection
        let mut selection = if base_enable {
            if pattern.trim().is_empty() {
                // User expectation: Empty pattern with Base Enabled means "All" (or at least valid default)
                let mut all = ElementGroupMask::new(count);
                all.invert();
                all
            } else {
                gc::parse_pattern(&pattern, count)
            }
        } else {
            let mut all = ElementGroupMask::new(count);
            all.invert();
            all
        };

        // 4. Apply Filters (AND logic)

        // --- Bounding Filter ---
        if bound_enable {
            match bound_type {
                0 => {
                    // Box
                    let half_size = bound_size * 0.5;
                    let min = bound_center - half_size;
                    let max = bound_center + half_size;
                    if group_type == 0 {
                        select::keep_by_bounding_box(&out_geo, min, max, &mut selection);
                    } else if group_type == 1 {
                        select::keep_primitives_by_bounding_box(&out_geo, min, max, &mut selection);
                    } else if group_type == 2 {
                        keep_vertices_by_bounding_box(&out_geo, min, max, &mut selection);
                    } else if group_type == 3 {
                        keep_edges_by_bounding_box(&out_geo, min, max, &mut selection);
                    }
                }
                1 => {
                    // Sphere
                    if group_type == 0 {
                        select::keep_by_bounding_sphere(
                            &out_geo,
                            bound_center,
                            bound_radius,
                            &mut selection,
                        );
                    } else if group_type == 1 {
                        select::keep_primitives_by_bounding_sphere(
                            &out_geo,
                            bound_center,
                            bound_radius,
                            &mut selection,
                        );
                    } else if group_type == 2 {
                        keep_vertices_by_bounding_sphere(
                            &out_geo,
                            bound_center,
                            bound_radius,
                            &mut selection,
                        );
                    } else if group_type == 3 {
                        keep_edges_by_bounding_sphere(
                            &out_geo,
                            bound_center,
                            bound_radius,
                            &mut selection,
                        );
                    }
                }
                2 => {
                    // Input Box
                    if let Some(bound_input) = inputs.get(1) {
                        if let Some((min, max)) = bound_input.materialize().compute_bounds() {
                            if group_type == 0 {
                                select::keep_by_bounding_box(&out_geo, min, max, &mut selection);
                            } else if group_type == 1 {
                                select::keep_primitives_by_bounding_box(
                                    &out_geo,
                                    min,
                                    max,
                                    &mut selection,
                                );
                            } else if group_type == 2 {
                                keep_vertices_by_bounding_box(&out_geo, min, max, &mut selection);
                            } else if group_type == 3 {
                                keep_edges_by_bounding_box(&out_geo, min, max, &mut selection);
                            }
                        }
                    }
                }
                // 3: Input Volume omitted for brevity
                _ => {}
            }
        }

        // --- Normal Filter ---
        if normal_enable {
            if group_type == 0 {
                select::keep_by_normal(&out_geo, normal_dir, normal_angle, &mut selection);
            } else if group_type == 1 {
                select::keep_primitives_by_normal(
                    &out_geo,
                    normal_dir,
                    normal_angle,
                    &mut selection,
                );
            } else if group_type == 2 {
                keep_vertices_by_normal(&out_geo, normal_dir, normal_angle, &mut selection);
            } else if group_type == 3 {
                keep_edges_by_normal(&out_geo, normal_dir, normal_angle, &mut selection);
            }
        }

        // --- Edge/Topology Filter ---
        if edge_enable && group_type == 0 {
            let topo = out_geo.build_topology();
            select::keep_boundary_points(&out_geo, &topo, &mut selection);
        }
        if edge_enable && group_type == 3 {
            keep_boundary_edges(&mut out_geo, &mut selection);
        }

        // 5. Final Merge
        if group_type == 0 {
            out_geo.ensure_point_group(&group_name);
            if let Some(final_mask) = out_geo.get_point_group_mut(&group_name) {
                apply_merge(final_mask, &selection, merge_op);
            }
        } else if group_type == 1 {
            out_geo.ensure_primitive_group(&group_name);
            if let Some(final_mask) = out_geo.get_primitive_group_mut(&group_name) {
                apply_merge(final_mask, &selection, merge_op);
            }
        } else if group_type == 2 {
            out_geo.ensure_vertex_group(&group_name);
            if let Some(final_mask) = out_geo.get_vertex_group_mut(&group_name) {
                apply_merge(final_mask, &selection, merge_op);
            }
        } else if group_type == 3 {
            out_geo.ensure_edge_group(&group_name);
            if let Some(final_mask) = out_geo.get_edge_group_mut(&group_name) {
                apply_merge(final_mask, &selection, merge_op);
            }
        }

        Arc::new(out_geo)
    }
}

impl GroupCreateNode {
    pub fn compute_params_map(
        &self,
        input: &Geometry,
        params: &std::collections::HashMap<String, ParameterValue>,
        bound_input: Option<Arc<Geometry>>,
    ) -> Arc<Geometry> {
        let mut out_geo = input.fork();
        let point_count = out_geo.get_point_count();
        let primitive_count = out_geo.primitives().len();
        let vertex_count = out_geo.vertices().len();

        let get_i = |n: &str, d: i32| {
            params
                .get(n)
                .and_then(|p| match p {
                    ParameterValue::Int(v) => Some(*v),
                    _ => None,
                })
                .unwrap_or(d)
        };
        let get_b = |n: &str, d: bool| {
            params
                .get(n)
                .and_then(|p| match p {
                    ParameterValue::Bool(v) => Some(*v),
                    _ => None,
                })
                .unwrap_or(d)
        };
        let get_f = |n: &str, d: f32| {
            params
                .get(n)
                .and_then(|p| match p {
                    ParameterValue::Float(v) => Some(*v),
                    _ => None,
                })
                .unwrap_or(d)
        };
        let get_s = |n: &str, d: &str| {
            params
                .get(n)
                .and_then(|p| match p {
                    ParameterValue::String(v) => Some(v.as_str()),
                    _ => None,
                })
                .unwrap_or(d)
                .to_string()
        };
        let get_v = |n: &str, d: Vec3| {
            params
                .get(n)
                .and_then(|p| match p {
                    ParameterValue::Vec3(v) => Some(*v),
                    _ => None,
                })
                .unwrap_or(d)
        };

        let group_name = get_s("group_name", "group1");
        let group_type = get_i("group_type", 0);
        let merge_op = get_i("merge_op", 0);
        let base_enable = get_b("base_enable", false);
        let pattern = get_s("pattern", "");
        let bound_enable = get_b("bound_enable", false);
        let bound_type = get_i("bound_type", 0);
        let bound_center = get_v("bound_center", Vec3::ZERO);
        let bound_size = get_v("bound_size", Vec3::ONE);
        let bound_radius = get_f("bound_radius", 1.0);
        let bound_iso = get_f("bound_iso", 0.0);
        let normal_enable = get_b("normal_enable", false);
        let normal_dir = get_v("normal_dir", Vec3::Y);
        let normal_angle = get_f("normal_angle", 180.0);
        let edge_enable = get_b("edge_enable", false);

        if group_name.is_empty() {
            return Arc::new(out_geo);
        }
        if group_type == 3 && out_geo.edges().is_empty() {
            let pcount = out_geo.points().len();
            let _ = gc::promote_mask(
                &mut out_geo,
                gc::GroupDomain::Point,
                gc::GroupDomain::Edge,
                &ElementGroupMask::new(pcount),
                gc::PromoteMode::All,
            );
        }
        let edge_count = out_geo.edges().len();
        let count = match group_type {
            0 => point_count,
            1 => primitive_count,
            2 => vertex_count,
            3 => edge_count,
            _ => point_count,
        };

        let mut selection = if base_enable {
            if pattern.trim().is_empty() {
                let mut all = ElementGroupMask::new(count);
                all.invert();
                all
            } else {
                gc::parse_pattern(pattern.as_str(), count)
            }
        } else {
            let mut all = ElementGroupMask::new(count);
            all.invert();
            all
        };

        if bound_enable {
            match bound_type {
                0 => {
                    let half_size = bound_size * 0.5;
                    let min = bound_center - half_size;
                    let max = bound_center + half_size;
                    if group_type == 0 {
                        select::keep_by_bounding_box(&out_geo, min, max, &mut selection);
                    } else if group_type == 1 {
                        select::keep_primitives_by_bounding_box(&out_geo, min, max, &mut selection);
                    } else if group_type == 2 {
                        keep_vertices_by_bounding_box(&out_geo, min, max, &mut selection);
                    } else if group_type == 3 {
                        keep_edges_by_bounding_box(&out_geo, min, max, &mut selection);
                    }
                }
                1 => {
                    if group_type == 0 {
                        select::keep_by_bounding_sphere(
                            &out_geo,
                            bound_center,
                            bound_radius,
                            &mut selection,
                        );
                    } else if group_type == 1 {
                        select::keep_primitives_by_bounding_sphere(
                            &out_geo,
                            bound_center,
                            bound_radius,
                            &mut selection,
                        );
                    } else if group_type == 2 {
                        keep_vertices_by_bounding_sphere(
                            &out_geo,
                            bound_center,
                            bound_radius,
                            &mut selection,
                        );
                    } else if group_type == 3 {
                        keep_edges_by_bounding_sphere(
                            &out_geo,
                            bound_center,
                            bound_radius,
                            &mut selection,
                        );
                    }
                }
                2 => {
                    if let Some(bound_input) = bound_input {
                        if let Some((min, max)) = bound_input.compute_bounds() {
                            if group_type == 0 {
                                select::keep_by_bounding_box(&out_geo, min, max, &mut selection);
                            } else if group_type == 1 {
                                select::keep_primitives_by_bounding_box(
                                    &out_geo,
                                    min,
                                    max,
                                    &mut selection,
                                );
                            } else if group_type == 2 {
                                keep_vertices_by_bounding_box(&out_geo, min, max, &mut selection);
                            } else if group_type == 3 {
                                keep_edges_by_bounding_box(&out_geo, min, max, &mut selection);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if normal_enable {
            if group_type == 0 {
                select::keep_by_normal(&out_geo, normal_dir, normal_angle, &mut selection);
            } else if group_type == 1 {
                select::keep_primitives_by_normal(
                    &out_geo,
                    normal_dir,
                    normal_angle,
                    &mut selection,
                );
            } else if group_type == 2 {
                keep_vertices_by_normal(&out_geo, normal_dir, normal_angle, &mut selection);
            } else if group_type == 3 {
                keep_edges_by_normal(&out_geo, normal_dir, normal_angle, &mut selection);
            }
        }

        if edge_enable && group_type == 0 {
            let topo = out_geo.build_topology();
            select::keep_boundary_points(&out_geo, &topo, &mut selection);
        }
        if edge_enable && group_type == 3 {
            keep_boundary_edges(&mut out_geo, &mut selection);
        }

        if group_type == 0 {
            out_geo.ensure_point_group(group_name.as_str());
            if let Some(final_mask) = out_geo.get_point_group_mut(group_name.as_str()) {
                apply_merge(final_mask, &selection, merge_op);
            }
        } else if group_type == 1 {
            out_geo.ensure_primitive_group(group_name.as_str());
            if let Some(final_mask) = out_geo.get_primitive_group_mut(group_name.as_str()) {
                apply_merge(final_mask, &selection, merge_op);
            }
        } else if group_type == 2 {
            out_geo.ensure_vertex_group(group_name.as_str());
            if let Some(final_mask) = out_geo.get_vertex_group_mut(group_name.as_str()) {
                apply_merge(final_mask, &selection, merge_op);
            }
        } else if group_type == 3 {
            out_geo.ensure_edge_group(group_name.as_str());
            if let Some(final_mask) = out_geo.get_edge_group_mut(group_name.as_str()) {
                apply_merge(final_mask, &selection, merge_op);
            }
        }
        Arc::new(out_geo)
    }
}

#[inline]
fn in_box(p: Vec3, min: Vec3, max: Vec3) -> bool {
    p.x >= min.x && p.x <= max.x && p.y >= min.y && p.y <= max.y && p.z >= min.z && p.z <= max.z
}

fn keep_vertices_by_bounding_box(
    geo: &Geometry,
    min: Vec3,
    max: Vec3,
    mask: &mut ElementGroupMask,
) {
    let Some(pos) = geo.get_point_position_attribute() else {
        return;
    };
    for i in mask.ones_vec() {
        let keep = (|| {
            let vid = geo.vertices().get_id_from_dense(i)?;
            let v = geo.vertices().get(vid)?;
            let pi = geo.points().get_dense_index(v.point_id.into())?;
            Some(pos.get(pi).map(|p| in_box(*p, min, max)).unwrap_or(false))
        })()
        .unwrap_or(false);
        if !keep {
            mask.set(i, false);
        }
    }
}

fn keep_vertices_by_bounding_sphere(
    geo: &Geometry,
    center: Vec3,
    radius: f32,
    mask: &mut ElementGroupMask,
) {
    let Some(pos) = geo.get_point_position_attribute() else {
        return;
    };
    let r2 = radius * radius;
    for i in mask.ones_vec() {
        let keep = (|| {
            let vid = geo.vertices().get_id_from_dense(i)?;
            let v = geo.vertices().get(vid)?;
            let pi = geo.points().get_dense_index(v.point_id.into())?;
            Some(
                pos.get(pi)
                    .map(|p| p.distance_squared(center) <= r2)
                    .unwrap_or(false),
            )
        })()
        .unwrap_or(false);
        if !keep {
            mask.set(i, false);
        }
    }
}

fn keep_edges_by_bounding_box(geo: &Geometry, min: Vec3, max: Vec3, mask: &mut ElementGroupMask) {
    let Some(pos) = geo.get_point_position_attribute() else {
        return;
    };
    for i in mask.ones_vec() {
        let keep = (|| {
            let eid = geo.edges().get_id_from_dense(i)?;
            let e = geo.edges().get(eid)?;
            let p0i = geo.points().get_dense_index(e.p0.into())?;
            let p1i = geo.points().get_dense_index(e.p1.into())?;
            let (p0, p1) = (*pos.get(p0i)?, *pos.get(p1i)?);
            Some(in_box((p0 + p1) * 0.5, min, max))
        })()
        .unwrap_or(false);
        if !keep {
            mask.set(i, false);
        }
    }
}

fn keep_edges_by_bounding_sphere(
    geo: &Geometry,
    center: Vec3,
    radius: f32,
    mask: &mut ElementGroupMask,
) {
    let Some(pos) = geo.get_point_position_attribute() else {
        return;
    };
    let r2 = radius * radius;
    for i in mask.ones_vec() {
        let keep = (|| {
            let eid = geo.edges().get_id_from_dense(i)?;
            let e = geo.edges().get(eid)?;
            let p0i = geo.points().get_dense_index(e.p0.into())?;
            let p1i = geo.points().get_dense_index(e.p1.into())?;
            let (p0, p1) = (*pos.get(p0i)?, *pos.get(p1i)?);
            Some(((p0 + p1) * 0.5).distance_squared(center) <= r2)
        })()
        .unwrap_or(false);
        if !keep {
            mask.set(i, false);
        }
    }
}

fn keep_vertices_by_normal(
    geo: &Geometry,
    direction: Vec3,
    angle_degrees: f32,
    mask: &mut ElementGroupMask,
) {
    let threshold = angle_degrees.to_radians().cos();
    let dir = direction.normalize_or_zero();
    if let Some(normals) = geo
        .get_vertex_attribute(attrs::N)
        .and_then(|a| a.as_slice::<Vec3>())
    {
        for i in mask.ones_vec() {
            if normals
                .get(i)
                .map(|n| n.dot(dir) >= threshold)
                .unwrap_or(false)
                == false
            {
                mask.set(i, false);
            }
        }
        return;
    }
    let Some(pn) = geo
        .get_point_attribute(attrs::N)
        .and_then(|a| a.as_slice::<Vec3>())
    else {
        return;
    };
    for i in mask.ones_vec() {
        let keep = (|| {
            let vid = geo.vertices().get_id_from_dense(i)?;
            let v = geo.vertices().get(vid)?;
            let pi = geo.points().get_dense_index(v.point_id.into())?;
            Some(pn.get(pi).map(|n| n.dot(dir) >= threshold).unwrap_or(false))
        })()
        .unwrap_or(false);
        if !keep {
            mask.set(i, false);
        }
    }
}

fn keep_edges_by_normal(
    geo: &Geometry,
    direction: Vec3,
    angle_degrees: f32,
    mask: &mut ElementGroupMask,
) {
    let threshold = angle_degrees.to_radians().cos();
    let dir = direction.normalize_or_zero();
    let Some(pn) = geo
        .get_point_attribute(attrs::N)
        .and_then(|a| a.as_slice::<Vec3>())
    else {
        return;
    };
    for i in mask.ones_vec() {
        let keep = (|| {
            let eid = geo.edges().get_id_from_dense(i)?;
            let e = geo.edges().get(eid)?;
            let p0i = geo.points().get_dense_index(e.p0.into())?;
            let p1i = geo.points().get_dense_index(e.p1.into())?;
            let n = (*pn.get(p0i)? + *pn.get(p1i)?).normalize_or_zero();
            Some(n.dot(dir) >= threshold)
        })()
        .unwrap_or(false);
        if !keep {
            mask.set(i, false);
        }
    }
}

#[inline]
fn norm_edge(
    p0: crate::libs::geometry::ids::PointId,
    p1: crate::libs::geometry::ids::PointId,
) -> (
    crate::libs::geometry::ids::PointId,
    crate::libs::geometry::ids::PointId,
) {
    if (p0.index, p0.generation) <= (p1.index, p1.generation) {
        (p0, p1)
    } else {
        (p1, p0)
    }
}

fn keep_boundary_edges(geo: &mut Geometry, mask: &mut ElementGroupMask) {
    if geo.edges().is_empty() {
        return;
    }
    let topo = geo.build_topology();
    let mut map = std::collections::HashMap::with_capacity(geo.edges().len() * 2);
    for (ei, e) in geo.edges().iter().enumerate() {
        map.insert(norm_edge(e.p0, e.p1), ei);
    }
    let mut boundary = ElementGroupMask::new(geo.edges().len());
    for &he in topo.get_boundary_edges() {
        let Some(h) = topo.half_edges.get(he.into()) else {
            continue;
        };
        let k = norm_edge(h.origin_point, topo.dest_point(he));
        if let Some(&ei) = map.get(&k) {
            boundary.set(ei, true);
        }
    }
    mask.intersect_with(&boundary);
}

fn apply_merge(target: &mut ElementGroupMask, source: &ElementGroupMask, op: i32) {
    match op {
        0 => {
            // Replace
            *target = source.clone();
        }
        1 => {
            // Union
            target.union_with(source);
        }
        2 => {
            // Intersect
            target.intersect_with(source);
        }
        3 => {
            // Subtract
            target.difference_with(source);
        }
        _ => {}
    }
}

register_node!("Group Create", "Group", GroupCreateNode; inputs: &["in:0", "in:1"], outputs: &["out:0"], style: crate::cunning_core::registries::node_registry::InputStyle::NamedPorts);

// Helper functions for parameter retrieval
fn get_param_string(params: &[Parameter], name: &str, default: &str) -> String {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or(default.to_string())
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

fn get_param_vec3(params: &[Parameter], name: &str, default: Vec3) -> Vec3 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::Vec3(v) => Some(*v),
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

pub fn node_style() -> crate::nodes::NodeStyle {
    crate::nodes::NodeStyle::Normal
}

pub fn input_style() -> crate::nodes::InputStyle {
    crate::nodes::InputStyle::Individual
}
