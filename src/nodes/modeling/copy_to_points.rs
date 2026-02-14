use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::algorithms::transform::transform_geometry_quat;
use crate::libs::algorithms::merge::merge_geometry_slice;
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::mesh::{Attribute, Geometry};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use bevy::math::{Quat, Vec3, Vec4};
use rayon::prelude::*;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct CopyToPointsNode;

impl NodeParameters for CopyToPointsNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "scale_mult",
                "Scale Multiplier",
                "General",
                ParameterValue::Float(1.0),
                ParameterUIType::FloatSlider {
                    min: 0.0,
                    max: 10.0,
                },
            ),
            Parameter::new(
                "forward_axis",
                "Forward Axis",
                "General",
                ParameterValue::Int(2), // +Z
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("+X".into(), 0),
                        ("+Y".into(), 1),
                        ("+Z".into(), 2),
                        ("-X".into(), 3),
                        ("-Y".into(), 4),
                        ("-Z".into(), 5),
                    ],
                },
            ),
        ]
    }
}

impl NodeOp for CopyToPointsNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let source_geo = match mats.get(0) {
            Some(g) => g,
            None => return Arc::new(Geometry::new()),
        };

        let target_points_geo = match mats.get(1) {
            Some(g) => g,
            None => return Arc::new(Geometry::new()),
        };

        if target_points_geo.get_point_count() == 0 {
            return Arc::new(Geometry::new());
        }

        let scale_mult = params
            .iter()
            .find(|p| p.name == "scale_mult")
            .and_then(|p| match p.value {
                ParameterValue::Float(v) => Some(v),
                _ => None,
            })
            .unwrap_or(1.0);
        let forward_axis = axis_from_choice(
            params
                .iter()
                .find(|p| p.name == "forward_axis")
                .and_then(|p| match p.value {
                    ParameterValue::Int(v) => Some(v),
                    _ => None,
                })
                .unwrap_or(2),
        );

        let positions = target_points_geo
            .get_point_position_attribute()
            .unwrap_or(&[]);
        let scales_attr = target_points_geo
            .get_point_attribute("pscale")
            .or_else(|| target_points_geo.get_point_attribute("@pscale"));
        let orient_attr = target_points_geo
            .get_point_attribute("orient")
            .or_else(|| target_points_geo.get_point_attribute("@orient"));
        let normals_attr = target_points_geo.get_point_attribute(attrs::N);
        let up_attr = target_points_geo
            .get_point_attribute("up")
            .or_else(|| target_points_geo.get_point_attribute("@up"));

        // Parallel instantiation
        // We iterate target points in parallel, create (fork) new geometries, transform them
        let result_geos: Vec<Geometry> = positions
            .par_iter()
            .enumerate()
            .map(|(i, &pos)| {
                let scale = sample_f32(scales_attr, i).unwrap_or(1.0) * scale_mult;

                // Houdini-like orientation priority:
                // 1) orient quaternion
                // 2) N (+ optional up)
                // 3) identity
                let rotation = if let Some(q) = sample_quat(orient_attr, i) {
                    if q.length_squared() > f32::EPSILON {
                        q.normalize()
                    } else {
                        Quat::IDENTITY
                    }
                } else if let Some(n) = sample_vec3(normals_attr, i) {
                    let up = sample_vec3(up_attr, i);
                    orientation_from_normal(n, up, forward_axis)
                } else {
                    Quat::IDENTITY
                };

                let mut instance = transform_geometry_quat(
                    source_geo.as_ref(),
                    pos,
                    rotation,
                    Vec3::splat(scale),
                );

                let mirrored = scale.is_sign_negative();

                // transform_geometry_quat handles vertex @N rotation;
                // rotate point/primitive @N here, then apply mirror flip for all domains.
                rotate_normals_on_domain(
                    instance
                        .get_point_attribute_mut(attrs::N)
                        .and_then(|a| a.as_mut_slice::<Vec3>()),
                    rotation,
                    mirrored,
                );
                rotate_normals_on_domain(
                    instance
                        .get_primitive_attribute_mut(attrs::N)
                        .and_then(|a| a.as_mut_slice::<Vec3>()),
                    rotation,
                    mirrored,
                );
                if mirrored {
                    flip_normals_on_domain(
                        instance
                            .get_vertex_attribute_mut(attrs::N)
                            .and_then(|a| a.as_mut_slice::<Vec3>()),
                    );
                }

                instance
            })
            .collect();

        Arc::new(merge_geometry_slice(&result_geos))
    }
}

#[inline]
fn rotate_normals_on_domain(normals: Option<&mut [Vec3]>, rotation: Quat, mirror_flip: bool) {
    let Some(normals) = normals else {
        return;
    };
    let sign = if mirror_flip { -1.0 } else { 1.0 };
    for n in normals.iter_mut() {
        *n = (rotation * *n) * sign;
        let len2 = n.length_squared();
        if len2 > f32::EPSILON {
            *n /= len2.sqrt();
        }
    }
}

#[inline]
fn flip_normals_on_domain(normals: Option<&mut [Vec3]>) {
    let Some(normals) = normals else {
        return;
    };
    for n in normals.iter_mut() {
        *n = -*n;
    }
}

#[inline]
fn orientation_from_normal(normal: Vec3, up: Option<Vec3>, local_forward: Vec3) -> Quat {
    let n = normal.normalize_or_zero();
    if n.length_squared() <= f32::EPSILON {
        return Quat::IDENTITY;
    }

    let fwd = local_forward.normalize_or_zero();
    if fwd.length_squared() <= f32::EPSILON {
        return Quat::IDENTITY;
    }

    // Base rotation: align chosen local forward axis to target N.
    let base = Quat::from_rotation_arc(fwd, n);

    // Optional twist from `up`: rotate around N so transformed local-up matches target up projection.
    if let Some(up) = up {
        let up_target = (up - n * up.dot(n)).normalize_or_zero();
        if up_target.length_squared() > f32::EPSILON {
            let local_up = pick_local_up(fwd);
            let cur_up = (base * local_up - n * (base * local_up).dot(n)).normalize_or_zero();
            if cur_up.length_squared() > f32::EPSILON {
                let sin = n.dot(cur_up.cross(up_target));
                let cos = cur_up.dot(up_target);
                let twist = Quat::from_axis_angle(n, sin.atan2(cos));
                return (twist * base).normalize();
            }
        }
    }

    base
}

#[inline]
fn pick_local_up(forward: Vec3) -> Vec3 {
    if forward.dot(Vec3::Y).abs() < 0.95 {
        Vec3::Y
    } else {
        Vec3::Z
    }
}

#[inline]
fn axis_from_choice(choice: i32) -> Vec3 {
    match choice {
        0 => Vec3::X,
        1 => Vec3::Y,
        2 => Vec3::Z,
        3 => -Vec3::X,
        4 => -Vec3::Y,
        5 => -Vec3::Z,
        _ => Vec3::Z,
    }
}

#[inline]
fn sample_f32(attr: Option<&Attribute>, index: usize) -> Option<f32> {
    let a = attr?;
    if let Some(s) = a.as_slice::<f32>() {
        return s.get(index).copied();
    }
    if let Some(pb) = a.as_paged::<f32>() {
        return pb.get(index);
    }
    None
}

#[inline]
fn sample_vec3(attr: Option<&Attribute>, index: usize) -> Option<Vec3> {
    let a = attr?;
    if let Some(s) = a.as_slice::<Vec3>() {
        return s.get(index).copied();
    }
    if let Some(pb) = a.as_paged::<Vec3>() {
        return pb.get(index);
    }
    None
}

#[inline]
fn sample_quat(attr: Option<&Attribute>, index: usize) -> Option<Quat> {
    let a = attr?;
    if let Some(s) = a.as_slice::<Quat>() {
        return s.get(index).copied();
    }
    if let Some(pb) = a.as_paged::<Quat>() {
        return pb.get(index);
    }
    if let Some(s) = a.as_slice::<Vec4>() {
        return s.get(index).map(|v| Quat::from_xyzw(v.x, v.y, v.z, v.w));
    }
    if let Some(pb) = a.as_paged::<Vec4>() {
        return pb
            .get(index)
            .map(|v| Quat::from_xyzw(v.x, v.y, v.z, v.w));
    }
    None
}

register_node!("CopyToPoints", "Modeling", CopyToPointsNode; inputs: &["Geometry", "Points"], outputs: &["Output"], style: crate::cunning_core::registries::node_registry::InputStyle::NamedPorts);
