use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::algorithms::algorithms_editor::fuse::{fuse_points, FuseOutputMode, FuseSettings};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::Geometry;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use std::sync::Arc;

#[derive(Default)]
pub struct FuseNode;

impl NodeParameters for FuseNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "distance",
                "Snap Distance",
                "Construct",
                ParameterValue::Float(0.001),
                ParameterUIType::FloatSlider {
                    min: 0.0,
                    max: 10.0,
                },
            ),
            Parameter::new(
                "remove_unused",
                "Remove Unused Points",
                "Cleanup",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "remove_degenerate",
                "Remove Degenerate Prims",
                "Cleanup",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for FuseNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let input = inputs
            .first()
            .map(|g| Arc::new(g.materialize()))
            .unwrap_or_else(|| Arc::new(Geometry::new()));
        let mut geo = (*input).clone();

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

        let distance = get_float("distance");
        let remove_unused = get_bool("remove_unused");
        let remove_degenerate = get_bool("remove_degenerate");

        let settings = FuseSettings {
            distance,
            output_mode: FuseOutputMode::Average,
            remove_unused_points: remove_unused,
            remove_degenerate_prims: remove_degenerate,
        };

        fuse_points(&mut geo, &settings);

        Arc::new(geo)
    }
}

crate::register_node!(
    "Fuse",
    "Modeling",
    crate::nodes::modeling::fuse_node::FuseNode
);
