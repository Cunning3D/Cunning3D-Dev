use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::Geometry;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use std::sync::Arc;

#[derive(Default)]
pub struct ForEachPointNode;

impl NodeParameters for ForEachPointNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![Parameter::new(
            "note",
            "Note",
            "General",
            ParameterValue::String("Spawner node (use context menu)".into()),
            ParameterUIType::String,
        )]
    }
}

impl NodeOp for ForEachPointNode {
    fn compute(&self, _params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        Arc::new(
            inputs
                .first()
                .map(|g| g.materialize())
                .unwrap_or_else(Geometry::new),
        )
    }
}

crate::register_node!("ForEach Point", "Flow", crate::nodes::flow::foreach_point::ForEachPointNode;
    inputs: &["Input"], outputs: &["Output"],
    style: crate::cunning_core::registries::node_registry::InputStyle::NamedPorts);
