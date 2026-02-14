use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::algorithms::algorithms_runtime::point_jitter::jitter_point_positions;
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::group::ElementGroupMask;
use crate::libs::geometry::mesh::Geometry;
use crate::nodes::attribute::normal_node::{NormalNode, NormalTarget};
use crate::nodes::group::utils::parse_pattern;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use bevy::prelude::Vec3;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct PointJitterNode;

impl NodeParameters for PointJitterNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "group",
                "Group",
                "Selection",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "scale",
                "Scale",
                "Jitter",
                ParameterValue::Float(0.1),
                ParameterUIType::FloatSlider {
                    min: 0.0,
                    max: 10.0,
                },
            ),
            Parameter::new(
                "axis_scales",
                "Axis Scales",
                "Jitter",
                ParameterValue::Vec3(Vec3::ONE),
                ParameterUIType::Vec3Drag,
            ),
            Parameter::new(
                "seed",
                "Seed",
                "Jitter",
                ParameterValue::Int(0),
                ParameterUIType::IntSlider {
                    min: 0,
                    max: 1_000_000,
                },
            ),
            Parameter::new(
                "update_normals",
                "Update Normals",
                "Normals",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for PointJitterNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let Some(input) = mats.first() else {
            return Arc::new(Geometry::new());
        };

        let group = get_string(params, "group", "");
        let scale = get_float(params, "scale", 0.1).max(0.0);
        let axis_scales = get_vec3(params, "axis_scales", Vec3::ONE);
        let seed = get_int(params, "seed", 0).max(0) as u64;
        let update_normals = get_bool(params, "update_normals", false);

        let mut out_geo = input.fork();

        // Build point selection mask:
        // - empty group => all points
        // - existing named point group => use it
        // - otherwise parse Houdini-style index pattern
        let point_count = out_geo.points().len();
        let selection: Option<ElementGroupMask> = if group.trim().is_empty() {
            None
        } else if let Some(mask) = out_geo.get_point_group(group.trim()) {
            Some(mask.clone())
        } else {
            Some(parse_pattern(group.trim(), point_count))
        };

        if let Some(p_attr) = out_geo.get_point_attribute_mut(attrs::P) {
            if let Some(positions) = p_attr.as_mut_slice::<Vec3>() {
                jitter_point_positions(
                    positions,
                    selection.as_ref(),
                    scale,
                    axis_scales,
                    seed,
                );
            }
        }

        if !update_normals {
            return Arc::new(out_geo);
        }

        // Recompute normals on jittered geometry.
        // We recompute vertex normals first (typical shading path) and then point normals.
        let out_geo = recompute_normals(out_geo, NormalTarget::Vertices);
        let out_geo = recompute_normals(out_geo, NormalTarget::Points);
        Arc::new(out_geo)
    }
}

fn recompute_normals(geo: Geometry, target: NormalTarget) -> Geometry {
    let mut normal_params = NormalNode::define_parameters();
    for p in &mut normal_params {
        if p.name == "compute_normals" {
            p.value = ParameterValue::Bool(true);
        } else if p.name == "add_normals_to" {
            p.value = ParameterValue::Int(target as i32);
        }
    }
    let op = NormalNode;
    let input: Arc<dyn GeometryRef> = Arc::new(geo);
    let out = op.compute(&normal_params, &[input]);
    (*out).clone()
}

#[inline]
fn get_float(params: &[Parameter], name: &str, default: f32) -> f32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::Float(v) => Some(*v),
            _ => None,
        })
        .unwrap_or(default)
}

#[inline]
fn get_int(params: &[Parameter], name: &str, default: i32) -> i32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::Int(v) => Some(*v),
            _ => None,
        })
        .unwrap_or(default)
}

#[inline]
fn get_bool(params: &[Parameter], name: &str, default: bool) -> bool {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::Bool(v) => Some(*v),
            _ => None,
        })
        .unwrap_or(default)
}

#[inline]
fn get_string(params: &[Parameter], name: &str, default: &str) -> String {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::String(v) => Some(v.clone()),
            _ => None,
        })
        .unwrap_or_else(|| default.to_string())
}

#[inline]
fn get_vec3(params: &[Parameter], name: &str, default: Vec3) -> Vec3 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::Vec3(v) => Some(*v),
            _ => None,
        })
        .unwrap_or(default)
}

register_node!("Point Jitter", "Modeling", PointJitterNode);
