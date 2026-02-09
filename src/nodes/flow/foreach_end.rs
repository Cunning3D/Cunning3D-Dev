use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::Geometry;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use std::sync::Arc;

#[derive(Default)]
pub struct ForEachEndNode;

impl NodeParameters for ForEachEndNode {
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
                "piece_domain",
                "Piece Elements",
                "Iteration",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![("Primitives".into(), 0), ("Points".into(), 1)],
                },
            ),
            Parameter::new(
                "iteration_method",
                "Iteration Method",
                "Iteration",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![("By Pieces".into(), 0), ("By Count".into(), 1)],
                },
            ),
            Parameter::new(
                "use_piece_attribute",
                "Use Piece Attribute",
                "Iteration",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "piece_attribute",
                "Piece Attribute",
                "Iteration",
                ParameterValue::String("class".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "count",
                "Count",
                "Iteration",
                ParameterValue::Int(1),
                ParameterUIType::IntSlider {
                    min: 1,
                    max: 1000000,
                },
            ),
            Parameter::new(
                "gather_method",
                "Gather Method",
                "Gather",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Merge Each Iteration".into(), 0),
                        ("Feedback Each Iteration".into(), 1),
                    ],
                },
            ),
            Parameter::new(
                "max_iterations",
                "Max Iterations",
                "Stop",
                ParameterValue::Int(100),
                ParameterUIType::IntSlider {
                    min: 1,
                    max: 1000000,
                },
            ),
            Parameter::new(
                "single_pass",
                "Single Pass",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "single_pass_mode",
                "Single Pass Select",
                "Debug",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("By Iteration Index".into(), 0),
                        ("By Attribute Value".into(), 1),
                    ],
                },
            ),
            Parameter::new(
                "single_pass_index",
                "Single Pass Index",
                "Debug",
                ParameterValue::Int(0),
                ParameterUIType::IntSlider {
                    min: 0,
                    max: 1000000,
                },
            ),
            Parameter::new(
                "single_pass_value",
                "Single Pass Value",
                "Debug",
                ParameterValue::String("0".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "stop_when_empty",
                "Stop When Empty",
                "Stop",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "stop_when_unchanged_hash",
                "Stop When Unchanged Hash",
                "Stop",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for ForEachEndNode {
    fn compute(&self, _params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        Arc::new(
            inputs
                .first()
                .map(|g| g.materialize())
                .unwrap_or_else(Geometry::new),
        )
    }
}

crate::register_node!("ForEach End", "Flow", crate::nodes::flow::foreach_end::ForEachEndNode;
    inputs: &["Input", "Feedback"], outputs: &["Output"],
    style: crate::cunning_core::registries::node_registry::InputStyle::NamedPorts);
