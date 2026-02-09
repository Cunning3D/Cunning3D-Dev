use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::Geometry;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use std::sync::Arc;

#[derive(Default)]
pub struct ForEachBeginNode;

impl NodeParameters for ForEachBeginNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "block_id",
                "Block ID",
                "Block",
                ParameterValue::String("foreach1".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "block_uid",
                "Block UID",
                "Block",
                ParameterValue::String(String::new()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "method",
                "Method",
                "Block",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Extract Piece or Point".into(), 0),
                        ("Fetch Feedback".into(), 1),
                        ("Fetch Metadata".into(), 2),
                        ("Fetch Input".into(), 3),
                    ],
                },
            ),
        ]
    }
}

impl NodeOp for ForEachBeginNode {
    fn compute(&self, _params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        Arc::new(
            inputs
                .first()
                .map(|g| g.materialize())
                .unwrap_or_else(Geometry::new),
        )
    }
}

crate::register_node!("ForEach Begin", "Flow", crate::nodes::flow::foreach_begin::ForEachBeginNode;
    inputs: &["Input"], outputs: &["Piece"],
    style: crate::cunning_core::registries::node_registry::InputStyle::NamedPorts);
