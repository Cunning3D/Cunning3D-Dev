//! Built-in voxel cat test asset.

use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::{mesh::Geometry, nodes::Parameter, register_node};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Default)]
pub struct CreateCatVoxNode;

impl NodeParameters for CreateCatVoxNode {
    fn define_parameters() -> Vec<Parameter> { Vec::new() }
}

impl NodeOp for CreateCatVoxNode {
    fn compute(&self, _params: &[Parameter], _inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        puffin::profile_function!();
        const VOX: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/textures/chr_cat.vox"));
        const ID: Uuid = Uuid::from_u128(0x63a5f9151b3c4c1f9c8c90cb1c2f8c67);
        crate::nodes::io::importers::vox::import_vox_bytes(VOX, ID, 0.1).map(Arc::new).unwrap_or_else(|_| Arc::new(Geometry::new()))
    }
}

register_node!("Create Cat (VOX)", "Test Assets", CreateCatVoxNode);

