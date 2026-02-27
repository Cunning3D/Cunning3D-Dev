//! Implementation of the `create_sphere` node.

use crate::libs::geometry::geo_ref::GeometryRef;
use crate::{
    cunning_core::traits::node_interface::{NodeOp, NodeParameters},
    libs::geometry::primitives,
    mesh::{Attribute, Geometry},
    nodes::{
        parameter::{Parameter, ParameterUIType, ParameterValue},
        InputStyle, NodeStyle,
    },
    register_node,
};
use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct CreateSphereNode;

impl NodeParameters for CreateSphereNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "radius",
                "Radius",
                "Sphere",
                ParameterValue::Float(0.5),
                ParameterUIType::FloatSlider { min: 0.1, max: 5.0 },
            ),
            Parameter::new(
                "center",
                "Center",
                "Sphere",
                ParameterValue::Vec3(Vec3::ZERO),
                ParameterUIType::Vec3Drag,
            ),
            Parameter::new(
                "divisions",
                "Divisions",
                "Sphere",
                ParameterValue::IVec2(IVec2::new(20, 10)),
                ParameterUIType::IVec2Drag,
            ),
        ]
    }
}

impl NodeOp for CreateSphereNode {
    fn compute(&self, params: &[Parameter], _inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        // Convert Vec<Parameter> to HashMap for legacy function
        let param_map: HashMap<String, ParameterValue> = params
            .iter()
            .map(|p| (p.name.clone(), p.value.clone()))
            .collect();
        Arc::new(create_sphere_geometry(&param_map))
    }
}

register_node!("Create Sphere", "Basic", CreateSphereNode);

pub fn create_sphere_geometry(parameters: &HashMap<String, ParameterValue>) -> Geometry {
    let radius = parameters
        .get("radius")
        .and_then(|p| match p {
            ParameterValue::Float(f) => Some(*f),
            _ => None,
        })
        .unwrap_or(0.5);
    let center = parameters
        .get("center")
        .and_then(|p| match p {
            ParameterValue::Vec3(v) => Some(*v),
            _ => None,
        })
        .unwrap_or(Vec3::ZERO);
    let divisions = parameters
        .get("divisions")
        .and_then(|p| match p {
            ParameterValue::IVec2(v) => Some(*v),
            _ => None,
        })
        .unwrap_or(IVec2::new(20, 10));

    // Use new primitives library
    let rows = divisions.y.max(2) as usize;
    let cols = divisions.x.max(3) as usize;

    let mut geo = primitives::create_sphere(radius, rows, cols);

    // Apply center offset
    if center != Vec3::ZERO {
        if let Some(pos) = geo
            .get_point_attribute_mut("@P")
            .and_then(|a| a.as_mut_slice::<Vec3>())
        {
            for p in pos {
                *p += center;
            }
        }
    }

    geo
}

pub fn node_style() -> NodeStyle {
    NodeStyle::Normal
}

pub fn input_style() -> InputStyle {
    InputStyle::Individual
}
