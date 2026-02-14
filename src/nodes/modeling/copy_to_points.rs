use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::algorithms::algorithms_dcc::PagedBuffer;
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

        if positions.is_empty() {
            return Arc::new(Geometry::new());
        }

        let scale_sampler = F32Sampler::new(scales_attr);
        let orient_sampler = QuatSampler::new(orient_attr);
        let normal_sampler = Vec3Sampler::new(normals_attr);
        let up_sampler = Vec3Sampler::new(up_attr);

        // Heavy source + many target points can spike memory if we materialize one Geometry per point.
        // Build per-chunk in parallel, merge inside chunk first, then do one final merge.
        let chunk_size = recommend_chunk_size(source_geo.as_ref(), positions.len());

        let chunk_geos: Vec<Geometry> = positions
            .par_chunks(chunk_size)
            .enumerate()
            .map(|(chunk_idx, chunk_positions)| {
                let base = chunk_idx * chunk_size;
                let mut local_instances = Vec::with_capacity(chunk_positions.len());
                for (local_i, &pos) in chunk_positions.iter().enumerate() {
                    let i = base + local_i;
                    let scale = scale_sampler.get(i).unwrap_or(1.0) * scale_mult;
                    let rotation = compute_rotation(
                        i,
                        &orient_sampler,
                        &normal_sampler,
                        &up_sampler,
                        forward_axis,
                    );
                    local_instances.push(build_instance(
                        source_geo.as_ref(),
                        pos,
                        rotation,
                        scale,
                    ));
                }
                merge_geometry_slice(&local_instances)
            })
            .collect();

        if chunk_geos.len() == 1 {
            Arc::new(chunk_geos.into_iter().next().unwrap_or_else(Geometry::new))
        } else {
            Arc::new(merge_geometry_slice(&chunk_geos))
        }
    }
}

#[inline]
fn build_instance(source_geo: &Geometry, pos: Vec3, rotation: Quat, scale: f32) -> Geometry {
    let mut instance = transform_geometry_quat(
        source_geo,
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
}

#[inline]
fn compute_rotation(
    index: usize,
    orient_sampler: &QuatSampler<'_>,
    normal_sampler: &Vec3Sampler<'_>,
    up_sampler: &Vec3Sampler<'_>,
    forward_axis: Vec3,
) -> Quat {
    // Houdini-like orientation priority:
    // 1) orient quaternion
    // 2) N (+ optional up)
    // 3) identity
    if let Some(q) = orient_sampler.get(index) {
        if q.length_squared() > f32::EPSILON {
            return q.normalize();
        }
        return Quat::IDENTITY;
    }
    if let Some(n) = normal_sampler.get(index) {
        let up = up_sampler.get(index);
        return orientation_from_normal(n, up, forward_axis);
    }
    Quat::IDENTITY
}

#[inline]
fn recommend_chunk_size(source_geo: &Geometry, copies: usize) -> usize {
    let src_complexity = source_geo.get_point_count()
        + source_geo.vertices().len()
        + source_geo.primitives().len();
    let base = if src_complexity >= 120_000 {
        8
    } else if src_complexity >= 40_000 {
        12
    } else if src_complexity >= 12_000 {
        16
    } else if src_complexity >= 4_000 {
        24
    } else {
        48
    };
    base.min(copies.max(1))
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

enum F32Sampler<'a> {
    None,
    Slice(&'a [f32]),
    Paged(&'a PagedBuffer<f32>),
}

impl<'a> F32Sampler<'a> {
    #[inline]
    fn new(attr: Option<&'a Attribute>) -> Self {
        let Some(a) = attr else {
            return Self::None;
        };
        if let Some(s) = a.as_slice::<f32>() {
            return Self::Slice(s);
        }
        if let Some(pb) = a.as_paged::<f32>() {
            return Self::Paged(pb);
        }
        Self::None
    }

    #[inline]
    fn get(&self, index: usize) -> Option<f32> {
        match self {
            Self::None => None,
            Self::Slice(s) => s.get(index).copied(),
            Self::Paged(pb) => pb.get(index),
        }
    }
}

enum Vec3Sampler<'a> {
    None,
    Slice(&'a [Vec3]),
    Paged(&'a PagedBuffer<Vec3>),
}

impl<'a> Vec3Sampler<'a> {
    #[inline]
    fn new(attr: Option<&'a Attribute>) -> Self {
        let Some(a) = attr else {
            return Self::None;
        };
        if let Some(s) = a.as_slice::<Vec3>() {
            return Self::Slice(s);
        }
        if let Some(pb) = a.as_paged::<Vec3>() {
            return Self::Paged(pb);
        }
        Self::None
    }

    #[inline]
    fn get(&self, index: usize) -> Option<Vec3> {
        match self {
            Self::None => None,
            Self::Slice(s) => s.get(index).copied(),
            Self::Paged(pb) => pb.get(index),
        }
    }
}

enum QuatSampler<'a> {
    None,
    QuatSlice(&'a [Quat]),
    QuatPaged(&'a PagedBuffer<Quat>),
    Vec4Slice(&'a [Vec4]),
    Vec4Paged(&'a PagedBuffer<Vec4>),
}

impl<'a> QuatSampler<'a> {
    #[inline]
    fn new(attr: Option<&'a Attribute>) -> Self {
        let Some(a) = attr else {
            return Self::None;
        };
        if let Some(s) = a.as_slice::<Quat>() {
            return Self::QuatSlice(s);
        }
        if let Some(pb) = a.as_paged::<Quat>() {
            return Self::QuatPaged(pb);
        }
        if let Some(s) = a.as_slice::<Vec4>() {
            return Self::Vec4Slice(s);
        }
        if let Some(pb) = a.as_paged::<Vec4>() {
            return Self::Vec4Paged(pb);
        }
        Self::None
    }

    #[inline]
    fn get(&self, index: usize) -> Option<Quat> {
        match self {
            Self::None => None,
            Self::QuatSlice(s) => s.get(index).copied(),
            Self::QuatPaged(pb) => pb.get(index),
            Self::Vec4Slice(s) => s.get(index).map(|v| Quat::from_xyzw(v.x, v.y, v.z, v.w)),
            Self::Vec4Paged(pb) => pb.get(index).map(|v| Quat::from_xyzw(v.x, v.y, v.z, v.w)),
        }
    }
}

register_node!("CopyToPoints", "Modeling", CopyToPointsNode; inputs: &["Geometry", "Points"], outputs: &["Output"], style: crate::cunning_core::registries::node_registry::InputStyle::NamedPorts);
