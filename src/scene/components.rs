use bevy::prelude::*;

/// Tag component for the final mesh entity representing the display node's geometry.
#[derive(Component)]
pub struct FinalMeshTag;

/// Key component for looking up the final material from the material library.
#[derive(Component, Clone)]
pub struct FinalMaterialKey(pub String);

/// Tag component for wireframe entities associated with the final mesh.
#[derive(Component)]
pub struct FinalWireframeTag;

/// Tag component for template meshes (legacy support).
#[derive(Component)]
pub struct TemplateMeshTag;

/// Tag component for volume visualization entities.
#[derive(Component)]
pub struct VolumeVizTag;

/// Component storing the original mesh handle before meshlet conversion.
#[cfg(feature = "virtual_geometry_meshlet")]
#[derive(Component)]
pub struct MeshletOriginalMesh(pub Handle<Mesh>);

/// Component holding the async task for meshlet conversion.
#[cfg(feature = "virtual_geometry_meshlet")]
#[derive(Component)]
pub struct MeshletConversionTask(
    pub bevy::tasks::Task<
        Result<
            bevy::pbr::experimental::meshlet::MeshletMesh,
            bevy::pbr::experimental::meshlet::MeshToMeshletMeshConversionError,
        >,
    >,
);

/// Component storing the original MSAA setting before meshlet conversion.
#[cfg(feature = "virtual_geometry_meshlet")]
#[derive(Component, Clone)]
pub struct OriginalMainCameraMsaa(pub Msaa);

/// Tag component for highlighting a specific point.
#[derive(Component, Clone, Copy)]
pub struct HighlightPointTag;

/// Tag component for displaying a specific point.
#[derive(Component, Clone, Copy)]
pub struct DisplayPointTag;

/// Tag component for highlighting a specific primitive.
#[derive(Component)]
pub struct HighlightPrimitiveTag;

/// Component storing geometry info for the displayed mesh.
/// Information about the currently displayed geometry.
#[derive(Component)]
pub struct DisplayedGeometryInfo {
    pub dirty_id: u64,
}

/// Tag component for vertex normal visualization entities.
#[derive(Component)]
pub struct VertexNormalTag;

/// Tag component for primitive normal visualization entities.
#[derive(Component)]
pub struct PrimitiveNormalTag;
