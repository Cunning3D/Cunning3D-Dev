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
pub struct GroupManageNode;

impl NodeParameters for GroupManageNode {
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
                "action",
                "Action",
                "General",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Copy".into(), 0),
                        ("Rename".into(), 1),
                        ("Delete".into(), 2),
                    ],
                },
            ),
            Parameter::new(
                "from",
                "From",
                "General",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "to",
                "To",
                "General",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
        ]
    }
}

impl NodeOp for GroupManageNode {
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
        let action = match get_i("action", 0) {
            1 => "rename",
            2 => "delete",
            _ => "copy",
        };
        let f = get_s("from", "");
        let t = get_s("to", "");
        let _ = gc::group_manage(&mut out, domain, action, f.as_str(), t.as_str());
        Arc::new(out)
    }
}

impl GroupManageNode {
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
        let action = match get_i("action", 0) {
            1 => "rename",
            2 => "delete",
            _ => "copy",
        };
        let f = get_s("from", "");
        let t = get_s("to", "");
        let _ = gc::group_manage(&mut out, domain, action, f.as_str(), t.as_str());
        Arc::new(out)
    }
}

register_node!("Group Manage", "Group", GroupManageNode);
