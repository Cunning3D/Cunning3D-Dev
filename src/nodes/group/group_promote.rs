use crate::libs::algorithms::algorithms_runtime::group_core as gc;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::{
    cunning_core::traits::node_interface::{NodeOp, NodeParameters},
    mesh::Geometry,
    nodes::parameter::{Parameter, ParameterUIType, ParameterValue},
    register_node,
};
use std::sync::Arc;

#[derive(Default)]
pub struct GroupPromoteNode;

impl NodeParameters for GroupPromoteNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "from_domain",
                "From",
                "General",
                ParameterValue::Int(2),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Point".into(), 0),
                        ("Vertex".into(), 1),
                        ("Primitive".into(), 2),
                        ("Edge".into(), 3),
                    ],
                },
            ),
            Parameter::new(
                "from",
                "Group",
                "General",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "to_domain",
                "To",
                "General",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Point".into(), 0),
                        ("Vertex".into(), 1),
                        ("Primitive".into(), 2),
                        ("Edge".into(), 3),
                    ],
                },
            ),
            Parameter::new(
                "to",
                "Out",
                "General",
                ParameterValue::String("group1".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "mode",
                "Mode",
                "General",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![("All".into(), 0), ("BoundaryOnly".into(), 1)],
                },
            ),
        ]
    }
}

impl NodeOp for GroupPromoteNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let input = inputs
            .first()
            .map(|g| Arc::new(g.materialize()))
            .unwrap_or_else(|| Arc::new(Geometry::new()));
        let mut out = input.fork();
        let get_i = |n: &str, d: i32| {
            params
                .iter()
                .find(|p| p.name == n)
                .and_then(|p| match p.value {
                    ParameterValue::Int(v) => Some(v),
                    _ => None,
                })
                .unwrap_or(d)
        };
        let get_s = |n: &str, d: &str| -> String {
            params
                .iter()
                .find(|p| p.name == n)
                .and_then(|p| match &p.value {
                    ParameterValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| d.to_string())
        };
        let dom = |v| match v {
            0 => gc::GroupDomain::Point,
            1 => gc::GroupDomain::Vertex,
            3 => gc::GroupDomain::Edge,
            _ => gc::GroupDomain::Primitive,
        };
        let mode = if get_i("mode", 0) == 1 {
            gc::PromoteMode::BoundaryOnly
        } else {
            gc::PromoteMode::All
        };
        let f = get_s("from", "");
        let t = get_s("to", "group1");
        let _ = gc::promote_named_group(
            &mut out,
            dom(get_i("from_domain", 2)),
            dom(get_i("to_domain", 0)),
            f.as_str(),
            t.as_str(),
            mode,
        );
        Arc::new(out)
    }
}

impl GroupPromoteNode {
    pub fn compute_params_map(
        &self,
        input: &Geometry,
        params: &std::collections::HashMap<String, ParameterValue>,
    ) -> Arc<Geometry> {
        let mut out = input.fork();
        let get_i = |n: &str, d: i32| {
            params
                .get(n)
                .and_then(|p| match p {
                    ParameterValue::Int(v) => Some(*v),
                    _ => None,
                })
                .unwrap_or(d)
        };
        let get_s = |n: &str, d: &str| -> String {
            params
                .get(n)
                .and_then(|p| match p {
                    ParameterValue::String(s) => Some(s.as_str()),
                    _ => None,
                })
                .unwrap_or(d)
                .to_string()
        };
        let dom = |v| match v {
            0 => gc::GroupDomain::Point,
            1 => gc::GroupDomain::Vertex,
            3 => gc::GroupDomain::Edge,
            _ => gc::GroupDomain::Primitive,
        };
        let mode = if get_i("mode", 0) == 1 {
            gc::PromoteMode::BoundaryOnly
        } else {
            gc::PromoteMode::All
        };
        let f = get_s("from", "");
        let t = get_s("to", "group1");
        let _ = gc::promote_named_group(
            &mut out,
            dom(get_i("from_domain", 2)),
            dom(get_i("to_domain", 0)),
            f.as_str(),
            t.as_str(),
            mode,
        );
        Arc::new(out)
    }
}

register_node!("Group Promote", "Group", GroupPromoteNode);
