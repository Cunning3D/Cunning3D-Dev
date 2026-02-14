use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::{Attribute, Geometry};
use crate::nodes::parameter::{CurveData, Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use bevy::prelude::Vec3;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct AddNode;

impl NodeParameters for AddNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![Parameter::new(
            "points",
            "Points",
            "Add",
            ParameterValue::Curve(CurveData::default()),
            ParameterUIType::CurvePoints,
        )]
    }
}

impl NodeOp for AddNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mut positions = inputs
            .first()
            .map(|g| g.materialize())
            .map(|g| read_input_positions(&g))
            .unwrap_or_default();

        if let Some(curve) = params
            .iter()
            .find(|p| p.name == "points")
            .and_then(|p| match &p.value {
                ParameterValue::Curve(data) => Some(data),
                _ => None,
            })
        {
            positions.extend(curve.points.iter().map(|pt| pt.position));
        }

        let mut out = Geometry::new();
        for _ in 0..positions.len() {
            out.add_point();
        }
        out.insert_point_attribute(attrs::P, Attribute::new_auto(positions));
        Arc::new(out)
    }
}

#[inline]
fn read_input_positions(input: &Geometry) -> Vec<Vec3> {
    if let Some(a) = input.get_point_attribute(attrs::P) {
        if let Some(v) = a.as_slice::<Vec3>() {
            return v.to_vec();
        }
        if let Some(pb) = a.as_paged::<Vec3>() {
            return pb.flatten();
        }
    }
    vec![Vec3::ZERO; input.points().len()]
}

register_node!("Add", "Modeling", AddNode);
