//! AI texture nodes + runtime jobs.

use bevy::prelude::*;

mod nano_heightmap;
mod nano_hexplanar_baker;
mod nano_hexplanar_baker_v2;
mod nano_to_3d_common;
mod nano_to_mesh;
mod nano_to_voxel;
mod nano_voxel_point_scatter;
pub(crate) mod nano_voxel_painter;
mod voxel_from_heightmap;

pub use nano_heightmap::*;
pub use nano_hexplanar_baker::*;
pub use nano_hexplanar_baker_v2::{NanoHexPlanarBakerV2Node, NODE_NANO_HEXPLANAR_BAKER_V2};
pub use nano_to_mesh::{NanoToMeshNode, NODE_NANO_TO_MESH};
pub use nano_to_voxel::{NanoToVoxelNode, NODE_NANO_TO_VOXEL};
pub use nano_voxel_point_scatter::*;
pub use nano_voxel_painter::*;
pub use voxel_from_heightmap::*;

/// Conventional AI system prompt parameter name for all AI nodes.
pub const PARAM_AI_SYSTEM_PROMPT: &str = "__ai_system_prompt";

/// AI Texture plugin: background jobs + node registration side effects.
pub struct AiTexturePlugin;

impl Plugin for AiTexturePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<nano_heightmap::NanoHeightmapJobs>()
            .init_resource::<nano_hexplanar_baker::NanoHexPlanarBakerJobs>()
            .init_resource::<nano_hexplanar_baker_v2::NanoHexPlanarBakerV2Jobs>()
            .init_resource::<nano_to_mesh::NanoToMeshJobs>()
            .init_resource::<nano_to_voxel::NanoToVoxelJobs>()
            .init_resource::<nano_voxel_point_scatter::NanoVoxelPointScatterJobs>()
            .init_resource::<nano_voxel_painter::NanoVoxelPainterJobs>()
            .add_systems(
                Update,
                (
                    nano_heightmap::nano_heightmap_jobs_system,
                    nano_hexplanar_baker::nano_hexplanar_baker_jobs_system,
                    nano_hexplanar_baker_v2::nano_hexplanar_baker_v2_jobs_system,
                    nano_to_mesh::nano_to_mesh_jobs_system,
                    nano_to_voxel::nano_to_voxel_jobs_system,
                    nano_voxel_point_scatter::nano_voxel_point_scatter_jobs_system,
                    nano_voxel_painter::nano_voxel_painter_jobs_system,
                ),
            );
    }
}

