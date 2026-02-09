//! `create_cube` 节点的功能实现。

use crate::libs::geometry::geo_ref::GeometryRef;
use crate::nodes::parameter::{ParameterUIType, ParameterValue};
use crate::{
    cunning_core::traits::node_interface::{NodeOp, NodeParameters},
    libs::geometry::primitives,
    mesh::{Attribute, Geometry},
    nodes::Parameter,
    register_node,
};
use bevy::prelude::{UVec3, Vec3};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct CreateCubeNode;

impl NodeParameters for CreateCubeNode {
    /// 定义 "Create Cube" 节点的参数
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "size",
                "Size",
                "Cube",
                ParameterValue::Float(1.0),
                ParameterUIType::FloatSlider {
                    min: 0.0,
                    max: 100.0,
                },
            ),
            Parameter::new(
                "center",
                "Center",
                "Cube",
                ParameterValue::Vec3(Vec3::ZERO),
                ParameterUIType::Vec3Drag,
            ),
            Parameter::new(
                "divisions",
                "Divisions",
                "Cube",
                ParameterValue::Vec3(Vec3::new(10.0, 10.0, 10.0)),
                ParameterUIType::Vec3Drag,
            ),
        ]
    }
}

impl NodeOp for CreateCubeNode {
    fn compute(&self, params: &[Parameter], _inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let param_map: HashMap<String, ParameterValue> = params
            .iter()
            .map(|p| (p.name.clone(), p.value.clone()))
            .collect();
        Arc::new(create_cube_geometry(&param_map))
    }
}

register_node!("Create Cube", "Basic", CreateCubeNode);

pub fn node_style() -> crate::nodes::NodeStyle {
    crate::nodes::NodeStyle::Normal
}

pub fn input_style() -> crate::nodes::InputStyle {
    crate::nodes::InputStyle::Individual
}

/// Creates a cube geometry with subdivisions, ensuring points are shared.
pub fn create_cube_geometry(parameters: &HashMap<String, ParameterValue>) -> Geometry {
    puffin::profile_function!();

    let size = parameters
        .get("size")
        .and_then(|p| match p {
            ParameterValue::Float(f) => Some(*f),
            _ => None,
        })
        .unwrap_or(1.0);
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
            ParameterValue::Vec3(v) => Some(*v),
            _ => None,
        })
        .unwrap_or(Vec3::ONE);

    let divs = divisions.as_uvec3().max(UVec3::ONE);

    let mut geo = primitives::create_cube(size, divs);

    // Apply center translation
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
