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
pub struct GroupNormalizeNode;

impl NodeParameters for GroupNormalizeNode {
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
                "drop_empty",
                "Drop Empty",
                "General",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for GroupNormalizeNode {
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
        let get_b = |n: &str, d: bool| {
            params
                .iter()
                .find(|p| p.name == n)
                .and_then(|p| match p.value {
                    ParameterValue::Bool(v) => Some(v),
                    _ => None,
                })
                .unwrap_or(d)
        };
        let domain = match get_i("domain", 2) {
            0 => gc::GroupDomain::Point,
            1 => gc::GroupDomain::Vertex,
            3 => gc::GroupDomain::Edge,
            _ => gc::GroupDomain::Primitive,
        };
        gc::group_normalize(&mut out, domain, get_b("drop_empty", false));
        Arc::new(out)
    }
}

impl GroupNormalizeNode {
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
        let get_b = |n: &str, d: bool| {
            params
                .get(n)
                .and_then(|p| match p {
                    ParameterValue::Bool(v) => Some(*v),
                    _ => None,
                })
                .unwrap_or(d)
        };
        let domain = match get_i("domain", 2) {
            0 => gc::GroupDomain::Point,
            1 => gc::GroupDomain::Vertex,
            3 => gc::GroupDomain::Edge,
            _ => gc::GroupDomain::Primitive,
        };
        gc::group_normalize(&mut out, domain, get_b("drop_empty", false));
        Arc::new(out)
    }
}

register_node!("Group Normalize", "Group", GroupNormalizeNode);
