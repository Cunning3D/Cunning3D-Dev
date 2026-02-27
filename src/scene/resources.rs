use bevy::prelude::*;
use bevy_ecs::system::SystemParam;

use crate::{
    nodes::{NodeGraph, NodeGraphResource},
    render::final_material::FinalMaterial,
    render::wireframe::WireframeTopology,
    ui::UiState,
    ui::NodeEditorState,
    viewport_options::DisplayOptions,
};

use super::components::{
    DisplayedGeometryInfo, FinalMeshTag, FinalWireframeTag, PrimitiveNormalTag, TemplateMeshTag,
    VolumeVizTag,
};

use super::GraphChanged;
use crate::GeometryChanged;

/// System parameter struct for `update_3d_scene_from_node_graph`.
/// This bundles all the dependencies needed by the main scene update system.
#[derive(SystemParam)]
pub struct UpdateSceneFromGraphParam<'w, 's> {
    pub commands: Commands<'w, 's>,
    /// Legacy GraphChanged reader - kept during migration, will be removed later.
    pub graph_changed_reader: MessageReader<'w, 's, GraphChanged>,
    /// New GeometryChanged reader - primary trigger for scene updates.
    pub geometry_changed_reader: MessageReader<'w, 's, GeometryChanged>,
    pub node_graph_res: Res<'w, NodeGraphResource>,
    pub node_editor_state: Res<'w, NodeEditorState>,
    pub ui_state: Res<'w, UiState>,
    pub meshes: ResMut<'w, Assets<Mesh>>,
    pub materials: ResMut<'w, Assets<StandardMaterial>>,
    pub final_materials: ResMut<'w, Assets<FinalMaterial>>,
    pub asset_server: Res<'w, AssetServer>,
    pub wireframe_topologies: ResMut<'w, Assets<WireframeTopology>>,
    pub query_final_mesh: Query<
        'w,
        's,
        (Entity, &'static Mesh3d, &'static mut DisplayedGeometryInfo),
        With<FinalMeshTag>,
    >,
    pub query_final_mat: Query<
        'w,
        's,
        (
            Option<&'static MeshMaterial3d<FinalMaterial>>,
            Option<&'static OriginalMaterialHandle>,
        ),
        With<FinalMeshTag>,
    >,
    pub query_has_viz: Query<
        'w,
        's,
        (),
        (
            With<FinalMeshTag>,
            With<crate::render::group_visualization::GroupVisualization>,
        ),
    >,
    pub query_final_wireframe_markers: Query<'w, 's, &'static crate::render::wireframe::WireframeMarker, With<FinalWireframeTag>>,
    pub query_final_wireframe_entities: Query<'w, 's, Entity, With<FinalWireframeTag>>,
    pub query_final_wireframe_meshes: Query<'w, 's, &'static Mesh3d, With<FinalWireframeTag>>,
    pub query_any_mesh3d: Query<'w, 's, &'static Mesh3d>,
    pub query_primitive_normals: Query<'w, 's, &'static Mesh3d, With<PrimitiveNormalTag>>,
    pub query_volume_viz: Query<'w, 's, Entity, With<VolumeVizTag>>,
    pub query_template_mesh: Query<'w, 's, Entity, With<TemplateMeshTag>>,
    pub sdf_dummy_mesh: Res<'w, crate::render::sdf_surface::SdfSurfaceDummyMesh>,
    pub display_options: ResMut<'w, DisplayOptions>,
    pub viewport_perf: ResMut<'w, crate::viewport_perf::ViewportPerfTrace>,
}

/// Component that stores the original material handle before it was replaced
/// with a final material.
#[derive(Component, Clone)]
pub struct OriginalMaterialHandle(pub Handle<crate::render::final_material::FinalMaterial>);
