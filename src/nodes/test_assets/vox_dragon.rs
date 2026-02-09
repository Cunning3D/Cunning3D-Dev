//! Built-in voxel dragon test asset.

use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::{mesh::Geometry, nodes::Parameter, register_node};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Default)]
pub struct CreateDragonVoxNode;

impl NodeParameters for CreateDragonVoxNode {
    fn define_parameters() -> Vec<Parameter> { Vec::new() }
}

impl NodeOp for CreateDragonVoxNode {
    fn compute(&self, _params: &[Parameter], _inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        puffin::profile_function!();
        const VOX: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/textures/dragon.vox"));
        const ID: Uuid = Uuid::from_u128(0x3a2ad0b44d5343fa9e1d6f07b68f20c9);
        crate::nodes::io::importers::vox::import_vox_bytes(VOX, ID, 0.1).map(Arc::new).unwrap_or_else(|_| Arc::new(Geometry::new()))
    }
}

register_node!("Create Dragon (VOX)", "Test Assets", CreateDragonVoxNode);

