//! Coverlay Panel node: carries bindings JSON for a dynamic coverlay UI.
use crate::{
    cunning_core::traits::node_interface::{NodeInteraction, NodeOp, NodeParameters, ServiceProvider},
    libs::geometry::geo_ref::GeometryRef,
    mesh::Geometry,
    nodes::parameter::{Parameter, ParameterUIType, ParameterValue},
    register_node,
};
use bevy_egui::egui;
use std::sync::Arc;
use uuid::Uuid;

const PARAM_BINDINGS_JSON: &str = "bindings_json";

#[derive(Default, Clone)]
pub struct CoverlayPanelNode;

impl NodeParameters for CoverlayPanelNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![Parameter::new(
            PARAM_BINDINGS_JSON,
            "Bindings (Internal)",
            "Internal",
            ParameterValue::String("{\"title\":\"Controls\",\"bindings\":[]}".to_string()),
            ParameterUIType::Code,
        )]
    }
}

impl NodeOp for CoverlayPanelNode {
    fn compute(&self, _params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        inputs.first().map(|g| Arc::new(g.materialize())).unwrap_or_else(|| Arc::new(Geometry::new()))
    }
}

#[derive(Default, Clone)]
pub struct CoverlayPanelInteraction;

impl NodeInteraction for CoverlayPanelInteraction {
    fn has_coverlay(&self) -> bool { true }
    fn draw_coverlay(&self, ui: &mut egui::Ui, _services: &dyn ServiceProvider, _node_id: Uuid) {
        ui.label("Coverlay Panel is rendered by coverlay runtime.");
    }
}

register_node!("Coverlay Panel", "Utility", CoverlayPanelNode, CoverlayPanelInteraction);

