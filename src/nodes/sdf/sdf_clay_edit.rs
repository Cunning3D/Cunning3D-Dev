use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::Geometry;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use std::sync::Arc;

#[derive(Default)]
pub struct SdfClayEditNode;

impl NodeParameters for SdfClayEditNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "enabled",
                "Enabled",
                "Tool",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "radius",
                "Radius",
                "Tool",
                ParameterValue::Float(0.5),
                ParameterUIType::FloatSlider { min: 0.01, max: 1000.0 },
            ),
            Parameter::new(
                "smooth_k",
                "Smooth k",
                "Tool",
                ParameterValue::Float(0.05),
                ParameterUIType::FloatSlider { min: 0.0, max: 5.0 },
            ),
            Parameter::new(
                "mode",
                "Mode",
                "Tool",
                ParameterValue::Int(0),
                ParameterUIType::IntSlider { min: 0, max: 2 },
            ),
        ]
    }
}

impl NodeOp for SdfClayEditNode {
    fn compute(&self, _params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let input = inputs
            .first()
            .map(|g| g.materialize())
            .unwrap_or_else(Geometry::new);
        // Pass-through: edits happen on the GPU preview path (RenderApp) for now.
        // Keeping the same `SdfHandle` Arc identity ensures edits persist across recooks.
        Arc::new(input.fork())
    }
}

register_node!(
    "SDF Clay Edit",
    "SDF",
    SdfClayEditNode;
    coverlay: &[cunning_viewport::coverlay_dock::CoverlayPanelKind::SdfTools]
);

