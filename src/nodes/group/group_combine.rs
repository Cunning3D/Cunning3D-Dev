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
pub struct GroupCombineNode;

impl NodeParameters for GroupCombineNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "domain",
                "Domain",
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
                "a",
                "A",
                "General",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "op",
                "Op",
                "General",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Union".into(), 0),
                        ("Intersect".into(), 1),
                        ("Subtract".into(), 2),
                    ],
                },
            ),
            Parameter::new(
                "b",
                "B",
                "General",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "out",
                "Out",
                "General",
                ParameterValue::String("group1".into()),
                ParameterUIType::String,
            ),
        ]
    }
}

impl NodeOp for GroupCombineNode {
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
        let domain = match get_i("domain", 2) {
            0 => gc::GroupDomain::Point,
            1 => gc::GroupDomain::Vertex,
            3 => gc::GroupDomain::Edge,
            _ => gc::GroupDomain::Primitive,
        };
        let op = match get_i("op", 0) {
            1 => gc::GroupOp::Intersect,
            2 => gc::GroupOp::Subtract,
            _ => gc::GroupOp::Union,
        };
        let a = get_s("a", "");
        let b = get_s("b", "");
        let o = get_s("out", "group1");
        let _ = gc::group_combine(&mut out, domain, op, a.as_str(), b.as_str(), o.as_str());
        Arc::new(out)
    }
}

impl GroupCombineNode {
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
        let domain = match get_i("domain", 2) {
            0 => gc::GroupDomain::Point,
            1 => gc::GroupDomain::Vertex,
            3 => gc::GroupDomain::Edge,
            _ => gc::GroupDomain::Primitive,
        };
        let op = match get_i("op", 0) {
            1 => gc::GroupOp::Intersect,
            2 => gc::GroupOp::Subtract,
            _ => gc::GroupOp::Union,
        };
        let a = get_s("a", "");
        let b = get_s("b", "");
        let o = get_s("out", "group1");
        let _ = gc::group_combine(&mut out, domain, op, a.as_str(), b.as_str(), o.as_str());
        Arc::new(out)
    }
}

register_node!("Group Combine", "Group", GroupCombineNode);
