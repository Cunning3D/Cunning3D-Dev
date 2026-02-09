//! Scene management module for Cunning3D.
//!
//! This module handles 3D scene rendering, including:
//! - Final mesh component management
//! - Scene updates from node graph changes
//! - Wireframe, point cloud, and normal visualization
//! - Volume (VDB) rendering
//!
//! ## Migration Strategy
//!
//! This module uses a gradual migration approach:
//! 1. Initially, items are re-exported from main.rs
//! 2. As components are moved here, the re-exports are replaced
//! 3. Eventually main.rs will only import from this module

pub mod components;
pub mod geo_material;
pub mod resources;
pub mod systems;

pub use components::{
    DisplayPointTag, DisplayedGeometryInfo, FinalMaterialKey, FinalMeshTag, FinalWireframeTag,
    HighlightPointTag, HighlightPrimitiveTag, PrimitiveNormalTag, TemplateMeshTag, VertexNormalTag,
    VolumeVizTag,
};
#[cfg(feature = "virtual_geometry_meshlet")]
pub use components::{MeshletConversionTask, MeshletOriginalMesh, OriginalMainCameraMsaa};

pub use crate::GraphChanged;
pub use resources::UpdateSceneFromGraphParam;
pub use systems::{
    update_3d_scene_from_node_graph, update_final_mesh_material_system,
    update_final_mesh_visibility_system,
};

// ============================================================================
// MIGRATED ITEMS (Moved from main.rs to this module)
// ============================================================================

// As items are migrated, they go here and are removed from the re-exports above
// Example after migration:
// pub use components::{FinalMeshTag, FinalMaterialKey, ...};

/// Plugin that registers all scene-related systems and types.
///
/// This plugin will eventually replace the individual system registrations in main.rs
pub struct ScenePlugin;

impl bevy::prelude::Plugin for ScenePlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        // Systems will be registered here as they are migrated
        // app.add_systems(Update, (
        //     systems::update_3d_scene_from_node_graph,
        //     systems::update_final_mesh_visibility_system,
        //     systems::update_final_mesh_material_system,
        // ));
    }
}
