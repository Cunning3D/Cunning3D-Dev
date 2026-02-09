use crate::libs::geometry::geo_ref::GeometryRef;
use crate::{
    cunning_core::traits::node_interface::{NodeOp, NodeParameters},
    libs::algorithms::merge::merge_geometry_slice,
    mesh::Geometry,
    nodes::parameter::Parameter,
    register_node,
};
use std::sync::Arc;

#[derive(Default)]
pub struct MergeNode;

impl NodeParameters for MergeNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![]
    }
}

impl NodeOp for MergeNode {
    fn compute(&self, _params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Geometry> = inputs.iter().map(|g| g.materialize()).collect();
        Arc::new(merge_geometry_slice(&mats))
    }
}

// Multi-input node for merging geometries
register_node!("Merge", "Utility", MergeNode;
    inputs: &["Input"], outputs: &["Output"],
    style: crate::cunning_core::registries::node_registry::InputStyle::Multi);
