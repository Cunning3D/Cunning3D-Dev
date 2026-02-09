//! Built-in voxel teapot test asset.

use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::{mesh::Geometry, nodes::Parameter, register_node};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Default)]
pub struct CreateTeapotVoxNode;

impl NodeParameters for CreateTeapotVoxNode {
    fn define_parameters() -> Vec<Parameter> { Vec::new() }
}

impl NodeOp for CreateTeapotVoxNode {
    fn compute(&self, _params: &[Parameter], _inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        puffin::profile_function!();
        const VOX: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/textures/teapot.vox"));
        const ID: Uuid = Uuid::from_u128(0x4c1f0f3c9e764e91a5f2e18d1f3b3f01);
        crate::nodes::io::importers::vox::import_vox_bytes(VOX, ID, 0.1).map(Arc::new).unwrap_or_else(|_| Arc::new(Geometry::new()))
    }
}

register_node!("Create Teapot (VOX)", "Test Assets", CreateTeapotVoxNode);

