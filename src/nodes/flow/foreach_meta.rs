use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::mesh::Attribute;
use crate::mesh::Geometry;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use std::sync::Arc;

#[derive(Default)]
pub struct ForEachMetaNode;

impl NodeParameters for ForEachMetaNode {
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
        ]
    }
}

impl NodeOp for ForEachMetaNode {
    fn compute(
        &self,
        params: &[Parameter],
        _inputs: &[Arc<dyn crate::libs::geometry::geo_ref::GeometryRef>],
    ) -> Arc<Geometry> {
        let uid = params
            .iter()
            .find(|p| p.name == "block_uid")
            .and_then(|p| {
                if let ParameterValue::String(s) = &p.value {
                    Some(s.as_str())
                } else {
                    None
                }
            })
            .unwrap_or("");
        let bid = params
            .iter()
            .find(|p| p.name == "block_id")
            .and_then(|p| {
                if let ParameterValue::String(s) = &p.value {
                    Some(s.as_str())
                } else {
                    None
                }
            })
            .unwrap_or("");
        let key = if !uid.is_empty() { uid } else { bid };
        let m = if !key.is_empty() {
            crate::nodes::runtime::foreach_tls::find_rev(key)
        } else {
            crate::nodes::runtime::foreach_tls::last().map(|(_, m)| m)
        }
        .unwrap_or_default();
        let mut g = Geometry::new();
        g.insert_detail_attribute("iteration", Attribute::new(vec![m.iteration]));
        g.insert_detail_attribute("numiterations", Attribute::new(vec![m.numiterations]));
        g.insert_detail_attribute("value", Attribute::new(vec![m.value]));
        g.insert_detail_attribute("ivalue", Attribute::new(vec![m.ivalue]));
        Arc::new(g)
    }
}

crate::register_node!("ForEach Meta", "Flow", crate::nodes::flow::foreach_meta::ForEachMetaNode;
    inputs: &[], outputs: &["Meta"],
    style: crate::cunning_core::registries::node_registry::InputStyle::NamedPorts);
